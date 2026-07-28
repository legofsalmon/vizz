//! Audio input: capture a device, analyse it, publish the result.
//!
//! Fail-soft like every other external dependency here. No sound card, no
//! permission, a device that vanishes mid-set — all end with the engine
//! reporting itself unavailable and the visuals continuing. An input
//! failure must never be able to stop a show.
//!
//! Threading follows the same rule as the rest of the app: the render
//! thread never blocks and never locks. cpal's callback pushes samples
//! into a lock-free ring; an analysis thread drains it, runs the FFT, and
//! publishes results into atomics; the render thread loads them. If
//! analysis falls behind, samples are dropped and counted — never awaited.

pub mod analysis;
pub mod beat;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub use analysis::{
    BAND_COUNT, Band, MAX_GAIN_DB, MIN_GAIN_DB, TARGET_PEAK, db_to_linear, default_bands,
    fit_gain_db, linear_to_db,
};
pub use beat::{MAX_BPM, MIN_BPM, TapTempo};

/// Published analysis results. Read by the render thread every frame,
/// written by the analysis thread ~94 times a second.
#[derive(Debug, Default)]
pub struct AudioState {
    bands: [AtomicU32; BAND_COUNT],
    raw: [AtomicU32; BAND_COUNT],
    /// Decaying peak of `raw`, which is what `fit` sets a gain against.
    ///
    /// Held here rather than sampled by the UI because the analysis thread
    /// sees every frame at ~94 Hz while the panel only draws at 60, and a
    /// peak is exactly the statistic that undersampling loses: the kick
    /// you missed is the one that mattered.
    raw_peak: [AtomicU32; BAND_COUNT],
    level: AtomicU32,
    bpm: AtomicU32,
    confidence: AtomicU32,
    /// Samples the analysis thread could not keep up with.
    pub dropped: AtomicUsize,
    /// Analysis frames completed — a liveness signal for the UI.
    pub frames: AtomicU64,
    connected: AtomicBool,
}

/// How long the peak hold takes to fall by 1/e. Long enough that a bar of
/// music is one measurement, short enough that moving a fader and pressing
/// `fit` again does what you meant.
const PEAK_TAU: f32 = 4.0;

/// Relaxed throughout: these are independent scalars sampled once a frame
/// for display and modulation. A torn read across two of them is invisible
/// at 60 Hz, and the ordering cost is not worth paying on the render path.
impl AudioState {
    fn store(slot: &AtomicU32, v: f32) {
        slot.store(v.to_bits(), Ordering::Relaxed);
    }
    fn load(slot: &AtomicU32) -> f32 {
        f32::from_bits(slot.load(Ordering::Relaxed))
    }

    pub fn band(&self, i: usize) -> f32 {
        self.bands.get(i).map(Self::load).unwrap_or(0.0)
    }
    pub fn raw(&self, i: usize) -> f32 {
        self.raw.get(i).map(Self::load).unwrap_or(0.0)
    }
    /// Loudest this band has been over roughly the last few seconds.
    pub fn raw_peak(&self, i: usize) -> f32 {
        self.raw_peak.get(i).map(Self::load).unwrap_or(0.0)
    }
    /// Fold a new reading into the decaying peak.
    ///
    /// Rises instantly and falls slowly, so a single kick is still visible
    /// to `fit` several seconds later. Decaying rather than absolute: a
    /// peak that never forgets would mean one accidental clip at the start
    /// of the night set the gain for the rest of it.
    /// Forget the held peak. Used when the input changes: a peak measured
    /// on one interface is not evidence about another, and `fit` reading a
    /// stale one would set every gain from a device that is gone.
    fn reset_peak(&self, i: usize) {
        Self::store(&self.raw_peak[i], 0.0);
    }

    /// The input has gone: say so, and drop the meters to nothing.
    ///
    /// Both halves matter. Leaving `connected` set means the panel shows a
    /// green dot and a device name for an interface that is no longer in
    /// the building, while every audio-reactive parameter sits frozen —
    /// which during a set reads as the application having broken rather
    /// than as a cable having come out, and sends the performer looking in
    /// the wrong place. Leaving the meters up means the last levels before
    /// it went stay painted, so the display insists audio is arriving.
    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::Relaxed);
        for i in 0..BAND_COUNT {
            Self::store(&self.bands[i], 0.0);
            Self::store(&self.raw[i], 0.0);
            self.reset_peak(i);
        }
        Self::store(&self.level, 0.0);
        Self::store(&self.confidence, 0.0);
    }

    /// Samples are arriving again.
    pub fn reconnect(&self) {
        self.connected.store(true, Ordering::Relaxed);
    }

    fn observe_peak(&self, i: usize, raw: f32, dt: f32) {
        let decay = (-dt / PEAK_TAU).exp();
        let held = Self::load(&self.raw_peak[i]) * decay;
        Self::store(&self.raw_peak[i], held.max(raw));
    }
    pub fn level(&self) -> f32 {
        Self::load(&self.level)
    }
    pub fn bpm(&self) -> f32 {
        Self::load(&self.bpm)
    }
    pub fn confidence(&self) -> f32 {
        Self::load(&self.confidence)
    }
    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

/// Single-producer single-consumer sample ring. The audio callback must be
/// allocation-free and wait-free; this is both.
struct Ring {
    buf: Box<[std::cell::UnsafeCell<f32>]>,
    write: AtomicUsize,
    read: AtomicUsize,
}

// Safety: exactly one producer (the cpal callback) and one consumer (the
// analysis thread). The atomic indices order the data writes against the
// reads, and the two threads never touch the same slot concurrently.
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

impl Ring {
    fn new(capacity: usize) -> Self {
        Self {
            buf: (0..capacity)
                .map(|_| std::cell::UnsafeCell::new(0.0))
                .collect(),
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
        }
    }

    /// Returns how many samples had to be dropped because the consumer is
    /// behind. Never blocks, never overwrites unread data.
    fn push(&self, samples: &[f32]) -> usize {
        let cap = self.buf.len();
        let w = self.write.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        let free = cap - (w.wrapping_sub(r)) - 1;
        let n = samples.len().min(free);
        for (i, &s) in samples[..n].iter().enumerate() {
            // Safety: slot is in the producer's exclusive region until the
            // release store below publishes it.
            unsafe { *self.buf[(w + i) % cap].get() = s };
        }
        self.write.store(w.wrapping_add(n), Ordering::Release);
        samples.len() - n
    }

    fn pop(&self, out: &mut [f32]) -> usize {
        let cap = self.buf.len();
        let r = self.read.load(Ordering::Relaxed);
        let w = self.write.load(Ordering::Acquire);
        let n = out.len().min(w.wrapping_sub(r));
        for (i, o) in out[..n].iter_mut().enumerate() {
            // Safety: published by the producer's release store.
            *o = unsafe { *self.buf[(r + i) % cap].get() };
        }
        self.read.store(r.wrapping_add(n), Ordering::Release);
        n
    }
}

/// Live-editable analysis settings. Written by the UI, read by the
/// analysis thread — not on the render path, so a mutex is fine here.
#[derive(Debug, Clone)]
pub struct AudioSettings {
    pub bands: [Band; BAND_COUNT],
    /// Let detected tempo drive the beat clock.
    pub auto_bpm: bool,
    /// Minimum confidence before a detected tempo is accepted.
    pub min_confidence: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            bands: default_bands(),
            auto_bpm: false,
            min_confidence: 0.25,
        }
    }
}

pub struct AudioEngine {
    pub state: Arc<AudioState>,
    pub settings: Arc<Mutex<AudioSettings>>,
    pub device_name: Option<String>,
    /// Held to keep the stream alive; dropping it stops capture.
    _stream: Option<cpal::Stream>,
    stop: Arc<AtomicBool>,
}

impl AudioEngine {
    /// Open the default input and start analysing. Never returns an error:
    /// a failure leaves the engine disconnected and the app running.
    pub fn start(device_name: Option<&str>) -> Self {
        let state = Arc::new(AudioState::default());
        let settings = Arc::new(Mutex::new(AudioSettings::default()));
        let stop = Arc::new(AtomicBool::new(false));

        match Self::open(device_name, &state, &settings, &stop) {
            Ok((stream, name)) => {
                log::info!("audio input: {name}");
                state.connected.store(true, Ordering::Relaxed);
                Self {
                    state,
                    settings,
                    device_name: Some(name),
                    _stream: Some(stream),
                    stop,
                }
            }
            Err(e) => {
                log::warn!("audio input unavailable: {e} — continuing without it");
                Self {
                    state,
                    settings,
                    device_name: None,
                    _stream: None,
                    stop,
                }
            }
        }
    }

    /// Switch to another input without disturbing anything downstream.
    ///
    /// Deliberately not "drop the engine and build a new one". `state` and
    /// `settings` are shared by `Arc` with the render thread and the
    /// analysis thread, and the settings hold the band gains — which are
    /// the one thing a user has hand-tuned to their interface. Rebuilding
    /// the engine would silently reset them at the exact moment the user
    /// plugged in the interface they tuned for.
    ///
    /// Returns whether the new device opened. On failure the engine is
    /// left disconnected rather than silently still capturing the old
    /// device, so the panel can say so instead of showing meters that
    /// belong to something the user just switched away from.
    pub fn reopen(&mut self, device_name: Option<&str>) -> bool {
        // Stop the running analysis thread and let go of the stream before
        // opening another: two threads writing the same `AudioState` would
        // interleave two devices' readings.
        self.stop.store(true, Ordering::Relaxed);
        self._stream = None;
        self.state.connected.store(false, Ordering::Relaxed);
        // Meters must not keep showing the old device's levels while the
        // new one is opening, or worse, forever if it fails.
        for i in 0..BAND_COUNT {
            Self::reset_band(&self.state, i);
        }
        AudioState::store(&self.state.level, 0.0);

        // A fresh flag: reusing the old one would stop the new thread the
        // moment it started.
        self.stop = Arc::new(AtomicBool::new(false));
        match Self::open(device_name, &self.state, &self.settings, &self.stop) {
            Ok((stream, name)) => {
                log::info!("audio input: {name}");
                self.state.connected.store(true, Ordering::Relaxed);
                self.device_name = Some(name);
                self._stream = Some(stream);
                true
            }
            Err(e) => {
                log::warn!("could not open audio input {device_name:?}: {e}");
                self.device_name = None;
                false
            }
        }
    }

    fn reset_band(state: &AudioState, i: usize) {
        AudioState::store(&state.bands[i], 0.0);
        AudioState::store(&state.raw[i], 0.0);
        state.reset_peak(i);
    }

    fn open(
        want: Option<&str>,
        state: &Arc<AudioState>,
        settings: &Arc<Mutex<AudioSettings>>,
        stop: &Arc<AtomicBool>,
    ) -> anyhow::Result<(cpal::Stream, String)> {
        let host = cpal::default_host();
        let device = match want {
            Some(name) => host
                .input_devices()?
                // cpal 0.18 dropped `Device::name()`; Display is the
                // documented way to get the name as a string.
                .find(|d| d.to_string().contains(name))
                .ok_or_else(|| anyhow::anyhow!("no input device matching {name:?}"))?,
            None => host
                .default_input_device()
                .ok_or_else(|| anyhow::anyhow!("no default input device"))?,
        };
        let name = device.to_string();
        let config = device.default_input_config()?;
        // SampleRate is a plain u32 in cpal 0.18, not a newtype.
        let sample_rate = config.sample_rate() as f32;
        let channels = config.channels() as usize;

        // A second of headroom: far more than analysis needs, but it costs
        // 192 kB and makes a scheduling hiccup invisible instead of audible
        // as a gap in the modulation.
        let ring = Arc::new(Ring::new(sample_rate as usize));

        let producer = ring.clone();
        let dropped = state.clone();
        let mut mono = Vec::new();
        // The error callback is the one unambiguous "this device is gone"
        // signal cpal offers, and it used to only write a line to a log
        // nobody reads mid-set.
        let err_state = state.clone();
        let err_fn = move |e| {
            log::warn!("audio stream error: {e} — input marked disconnected");
            err_state.disconnect();
        };
        let stream = device.build_input_stream(
            config.into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Downmix in the callback so the ring holds mono and the
                // analysis thread never has to know the channel count.
                mono.clear();
                mono.extend(
                    data.chunks(channels)
                        .map(|f| f.iter().sum::<f32>() / channels as f32),
                );
                let lost = producer.push(&mono);
                if lost > 0 {
                    dropped.dropped.fetch_add(lost, Ordering::Relaxed);
                }
            },
            err_fn,
            None,
        )?;
        stream.play()?;

        let consumer = ring;
        let state = state.clone();
        let settings = settings.clone();
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("vizz-audio".into())
            .spawn(move || analysis_loop(consumer, state, settings, stop, sample_rate))?;

        Ok((stream, name))
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// How long an input may deliver nothing before it is treated as gone.
///
/// Comfortably longer than any scheduling hiccup — a device that is really
/// there never pauses for this long — and short enough that the panel is
/// telling the truth well before anyone has finished wondering why the
/// visuals stopped moving.
const SILENCE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

fn analysis_loop(
    ring: Arc<Ring>,
    state: Arc<AudioState>,
    settings: Arc<Mutex<AudioSettings>>,
    stop: Arc<AtomicBool>,
    sample_rate: f32,
) {
    use analysis::{Analyzer, FFT_SIZE, HOP};

    let mut analyzer = Analyzer::new(sample_rate);
    let mut detector = beat::BeatDetector::new(sample_rate);
    // Sliding window: keep FFT_SIZE samples, advance by HOP each frame, so
    // successive windows overlap and a transient cannot fall between them.
    let mut window = vec![0.0f32; FFT_SIZE];
    let mut incoming = vec![0.0f32; HOP];
    let dt = HOP as f32 / sample_rate;
    let mut last_samples = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        if ring.pop(&mut incoming) < HOP {
            // Starvation is not silence.
            //
            // A working input delivers samples continuously whether or not
            // there is any sound in them — a quiet room is a stream of
            // zeros, not an absent stream. So a gap this long means the
            // device stopped delivering: unplugged, put to sleep, or the
            // stream died without cpal raising an error, which is the
            // usual shape of a USB interface being pulled on macOS.
            //
            // Checked here because this is the one place that knows, and
            // it costs a clock read on a thread that was about to sleep.
            if state.connected() && last_samples.elapsed() > SILENCE_TIMEOUT {
                log::warn!("audio input stopped delivering — marking it disconnected");
                state.disconnect();
            }
            // Nothing ready. Sleeping a fraction of a hop keeps latency
            // well under a frame without spinning a core.
            std::thread::sleep(std::time::Duration::from_micros(
                (dt * 250_000.0).max(200.0) as u64,
            ));
            continue;
        }
        // An interface that wakes up, or comes back on the same port,
        // should light again without making the user re-pick it.
        last_samples = Instant::now();
        if !state.connected() {
            log::info!("audio input delivering again");
            state.reconnect();
        }
        window.copy_within(HOP.., 0);
        window[FFT_SIZE - HOP..].copy_from_slice(&incoming);

        let bands = settings.lock().map(|s| s.bands).unwrap_or_else(|e| e.into_inner().bands);
        let frame = analyzer.analyze(&window, &bands, dt);
        detector.push(frame.flux);

        for i in 0..BAND_COUNT {
            AudioState::store(&state.bands[i], frame.bands[i]);
            AudioState::store(&state.raw[i], frame.raw[i]);
            state.observe_peak(i, frame.raw[i], dt);
        }
        AudioState::store(&state.level, frame.level);
        AudioState::store(&state.bpm, detector.bpm());
        AudioState::store(&state.confidence, detector.confidence());
        state.frames.fetch_add(1, Ordering::Relaxed);
    }
}

/// Input devices available for selection in the UI. Empty on failure.
pub fn input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|ds| ds.map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring is the one piece of unsafe code here, and the property that
    /// matters is that a slow consumer costs dropped samples rather than a
    /// blocked audio callback or corrupted data.
    #[test]
    fn ring_round_trips_in_order() {
        let r = Ring::new(16);
        let src: Vec<f32> = (0..10).map(|i| i as f32).collect();
        assert_eq!(r.push(&src), 0);
        let mut out = vec![0.0; 10];
        assert_eq!(r.pop(&mut out), 10);
        assert_eq!(out, src);
    }

    #[test]
    fn ring_drops_rather_than_overwrites() {
        let r = Ring::new(8);
        let src: Vec<f32> = (0..20).map(|i| i as f32).collect();
        let lost = r.push(&src);
        // Capacity 8 holds 7 (one slot distinguishes full from empty).
        assert_eq!(lost, 13, "expected the overflow to be reported");

        let mut out = vec![0.0; 7];
        assert_eq!(r.pop(&mut out), 7);
        // The samples kept must be the *oldest* contiguous run, unbroken —
        // an overwriting ring would interleave old and new here.
        assert_eq!(out, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn ring_wraps_repeatedly() {
        let r = Ring::new(8);
        let mut out = [0.0; 4];
        for round in 0..50 {
            let batch: Vec<f32> = (0..4).map(|i| (round * 4 + i) as f32).collect();
            assert_eq!(r.push(&batch), 0, "round {round} should fit");
            assert_eq!(r.pop(&mut out), 4);
            assert_eq!(out.to_vec(), batch, "round {round}");
        }
    }

    /// The panel used to show a green dot and a device name for an
    /// interface that had been unplugged, with every audio-reactive
    /// parameter frozen at its last value — which mid-set reads as the app
    /// having broken rather than as a cable having come out.
    ///
    /// Drives the real analysis loop rather than calling `disconnect`
    /// directly, because the thing worth pinning is that starvation is
    /// *detected*: a working input delivers zeros when the room is quiet,
    /// so "no samples" and "no sound" are different conditions and only
    /// one of them means the device is gone.
    #[test]
    fn an_input_that_stops_delivering_stops_claiming_to_be_connected() {
        use analysis::{FFT_SIZE, HOP};

        let ring = Arc::new(Ring::new(FFT_SIZE * 4));
        let state = Arc::new(AudioState::default());
        let settings = Arc::new(Mutex::new(AudioSettings::default()));
        let stop = Arc::new(AtomicBool::new(false));
        state.reconnect();

        let loop_ring = ring.clone();
        let loop_state = state.clone();
        let loop_stop = stop.clone();
        let handle = std::thread::spawn(move || {
            analysis_loop(loop_ring, loop_state, settings, loop_stop, 48_000.0);
        });

        // Deliver a quiet room: real samples, all zero. This must NOT be
        // mistaken for a missing device.
        let quiet = vec![0.0f32; HOP];
        let fed_until = Instant::now();
        while fed_until.elapsed() < SILENCE_TIMEOUT * 2 {
            ring.push(&quiet);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            state.connected(),
            "a silent but working input was reported as disconnected"
        );

        // Now pull the interface: stop delivering anything at all.
        let pulled = Instant::now();
        while state.connected() && pulled.elapsed() < SILENCE_TIMEOUT * 4 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            !state.connected(),
            "an input that stopped delivering still claimed to be connected"
        );

        // And it comes back on its own when samples resume, rather than
        // needing the device to be picked again.
        ring.push(&quiet);
        let back = Instant::now();
        while !state.connected() && back.elapsed() < std::time::Duration::from_secs(2) {
            ring.push(&quiet);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.connected(), "the input did not come back when samples resumed");

        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    /// A missing device is a normal condition, not an error path.
    #[test]
    fn engine_survives_having_no_device() {
        let engine = AudioEngine::start(Some("definitely-not-a-real-device"));
        assert!(!engine.state.connected());
        assert_eq!(engine.state.band(0), 0.0);
        assert_eq!(engine.state.bpm(), 0.0);
    }
}
