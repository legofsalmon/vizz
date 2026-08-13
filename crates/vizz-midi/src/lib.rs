//! MIDI input: hardware controllers driving vizz parameters.
//!
//! Like OSC, MIDI is a control-thread citizen — it writes normalised
//! targets into the shared [`ParamRegistry`] and never touches the
//! renderer. Devices are treated as hot-pluggable: ports are rescanned
//! periodically, so plugging a controller in mid-set connects it, and
//! unplugging one does not disturb anything else.
//!
//! Shared state (mappings, learn target, connected devices) lives behind
//! one mutex. The GUI reads it with `try_lock` and falls back to its last
//! snapshot, so the render thread can never be blocked by MIDI traffic.

pub mod feedback;
pub mod mapping;
pub mod message;
pub mod profile;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context as _, Result};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use vizz_params::ParamRegistry;

pub use mapping::{Binding, Dispatcher, MidiMap, Source, Update};
pub use message::MidiEvent;

/// How often the lights are brought up to date.
///
/// Faster than a rescan and slower than a frame. The sender only writes
/// what changed, so this is the *latency* of a pad lighting up rather
/// than a rate the bus has to carry — 30ms is under the threshold where
/// a button feels like it answered late, and well clear of the clock.
const FEEDBACK_INTERVAL: Duration = Duration::from_millis(30);

/// How often to rescan for newly plugged-in controllers.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

/// What a pending learn will bind the next control to.
#[derive(Debug, Clone, PartialEq)]
pub struct LearnTarget {
    /// Parameter address to bind.
    pub param: String,
    /// The fixed value a press should send, for a control that addresses a
    /// slot rather than sweeping a range. See [`Binding::value`].
    pub value: Option<f32>,
    /// What to call this while the learn is waiting. The address is right
    /// for a slider — it is what the row is labelled with — but "waiting
    /// for a control to bind to /scene/fire = 5" is not how anyone thinks
    /// about the pad they just armed.
    pub label: String,
}

impl LearnTarget {
    /// Learn a control that sweeps the parameter's range.
    pub fn param(param: impl Into<String>) -> Self {
        let param = param.into();
        Self { label: param.clone(), param, value: None }
    }

    /// Learn a button that jumps the parameter to one value.
    pub fn value(param: impl Into<String>, value: f32, label: impl Into<String>) -> Self {
        Self { param: param.into(), value: Some(value), label: label.into() }
    }
}

/// Tempo from a stream of MIDI clock ticks (24 per quarter note).
///
/// The wire clock is jittery — USB scheduling alone moves individual
/// ticks by milliseconds — so the estimate is the **median** of the last
/// two beats' worth of intervals, which a scheduling spike cannot drag
/// the way a mean would be dragged. The estimate expires half a second
/// after the last tick: a silent sender means no opinion, not the last
/// tempo frozen forever.
#[derive(Default)]
pub struct ClockEstimator {
    /// Recent tick intervals, seconds, newest last. Capped at 48
    /// (two beats), which is enough to settle and short enough to track
    /// a pitch fader.
    intervals: Vec<f32>,
    last_tick: Option<std::time::Instant>,
    /// Start arrived since the last take: the next downbeat is now.
    started: bool,
}

impl ClockEstimator {
    /// Longest believable gap between ticks: 20 bpm. Anything longer is
    /// a pause, not a tempo.
    const MAX_INTERVAL: f32 = 60.0 / (20.0 * 24.0);
    /// The estimate needs a beat of ticks before it says anything.
    const MIN_TICKS: usize = 24;
    /// How long after the last tick the estimate keeps being believed.
    const FRESH: std::time::Duration = std::time::Duration::from_millis(500);

    pub fn on_event(&mut self, event: MidiEvent, now: std::time::Instant) {
        match event {
            MidiEvent::Clock => {
                if let Some(prev) = self.last_tick {
                    let dt = now.duration_since(prev).as_secs_f32();
                    if dt > 0.0 && dt <= Self::MAX_INTERVAL {
                        if self.intervals.len() >= 48 {
                            self.intervals.remove(0);
                        }
                        self.intervals.push(dt);
                    }
                }
                self.last_tick = Some(now);
            }
            MidiEvent::Start => {
                self.started = true;
                // A fresh transport is a fresh measurement; stale
                // intervals from before the pause would skew the median.
                self.intervals.clear();
                self.last_tick = None;
            }
            MidiEvent::Continue => {
                // Resuming after a pause: the gap since the last tick is
                // silence, not an interval.
                self.last_tick = None;
            }
            MidiEvent::Stop => {
                self.last_tick = None;
            }
            _ => {}
        }
    }

    /// The tempo, once enough ticks have arrived and the last one is
    /// recent. `None` is "no opinion", never "zero".
    pub fn bpm(&self, now: std::time::Instant) -> Option<f32> {
        let last = self.last_tick?;
        if now.duration_since(last) > Self::FRESH || self.intervals.len() < Self::MIN_TICKS {
            return None;
        }
        let mut sorted = self.intervals.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        // The true even-count median — mean of the two middles. Taking
        // the upper middle alone is off by half the jitter amplitude
        // whenever the jitter alternates (USB batching does exactly
        // that), which read 128 bpm as 116.
        let mid = sorted.len() / 2;
        let median = if sorted.len().is_multiple_of(2) {
            (sorted[mid - 1] + sorted[mid]) * 0.5
        } else {
            sorted[mid]
        };
        Some(60.0 / (median * 24.0))
    }

    /// Whether Start arrived since the last call, consuming it.
    pub fn take_started(&mut self) -> bool {
        std::mem::take(&mut self.started)
    }
}

/// State shared between the MIDI thread and the UI.
#[derive(Default)]
pub struct MidiState {
    pub map: MidiMap,
    /// What is awaiting a control, set by the GUI's Learn button.
    pub learn_target: Option<LearnTarget>,
    /// Names of currently connected input ports.
    pub connected: Vec<String>,
    /// Last source seen, for "is it even sending?" feedback while learning.
    pub last_source: Option<Source>,
    /// Bumped whenever a binding changes, so the app knows to save.
    pub revision: u64,
    /// Tempo heard on the wire, fed by the realtime stream.
    pub clock: ClockEstimator,
    /// What the controller's pads should be showing. Written by the app
    /// each frame and read by the output thread; see [`feedback`].
    pub surface: feedback::Surface,
    /// Devices whose shipped profile has already been offered, so
    /// plugging one in twice does not re-add bindings the user has since
    /// removed on purpose.
    pub profiled: Vec<String>,
}

impl MidiState {
    /// Bind the learn target if one is pending. Returns true if a binding
    /// was made.
    fn apply_learn(&mut self, source: Source) -> bool {
        let Some(target) = self.learn_target.take() else { return false };
        match target.value {
            Some(v) => self.map.bind_value(source, target.param, v),
            None => self.map.bind(source, target.param),
        }
        self.revision += 1;
        true
    }

    /// Is a sweep learn pending for this parameter? Used to light the
    /// control's own learn button.
    pub fn learning(&self, param: &str) -> bool {
        matches!(&self.learn_target, Some(t) if t.param == param && t.value.is_none())
    }

    /// Is a trigger learn pending for this value of this parameter?
    pub fn learning_value(&self, param: &str, value: f32) -> bool {
        matches!(&self.learn_target, Some(t) if t.param == param && t.value == Some(value))
    }
}

pub type SharedMidi = Arc<Mutex<MidiState>>;

/// Running MIDI input. Dropping it disconnects and stops the thread.
pub struct MidiEngine {
    shared: SharedMidi,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    out_thread: Option<JoinHandle<()>>,
}

impl MidiEngine {
    /// Start listening on every available input port.
    ///
    /// Failing to open MIDI at all is *not* fatal — the visuals and OSC
    /// keep working — so callers should log and continue.
    pub fn spawn(registry: Arc<ParamRegistry>, shared: SharedMidi) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_shared = Arc::clone(&shared);

        let thread = std::thread::Builder::new()
            .name("vizz-midi".into())
            .spawn(move || run(registry, thread_shared, thread_stop))?;

        // Output on its own thread: it ticks far faster than the input
        // rescan, and a device that will not take a note must not be
        // able to hold up the one that is delivering messages.
        let out_stop = Arc::clone(&stop);
        let out_shared = Arc::clone(&shared);
        let out_thread = std::thread::Builder::new()
            .name("vizz-midi-out".into())
            .spawn(move || run_out(out_shared, out_stop))?;

        Ok(Self { shared, stop, thread: Some(thread), out_thread: Some(out_thread) })
    }

    pub fn shared(&self) -> &SharedMidi {
        &self.shared
    }
}

impl Drop for MidiEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.out_thread.take() {
            let _ = t.join();
        }
    }
}

/// Owns the connections; rescans so controllers can be plugged in mid-set.
fn run(registry: Arc<ParamRegistry>, shared: SharedMidi, stop: Arc<AtomicBool>) {
    // Connections must be kept alive; dropping one closes the port.
    let mut open: Vec<(String, MidiInputConnection<()>)> = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        match scan_and_connect(&registry, &shared, &mut open) {
            Ok(()) => {}
            // A transient enumeration failure must not kill MIDI for the
            // rest of the session; try again on the next pass.
            Err(e) => log::debug!("MIDI rescan failed: {e:#}"),
        }
        // Sliced so quitting does not wait out the rescan pause: the join
        // in `Drop` was stalling the whole app for up to two seconds.
        let mut slept = Duration::ZERO;
        while slept < RESCAN_INTERVAL && !stop.load(Ordering::Relaxed) {
            let step = (RESCAN_INTERVAL - slept).min(Duration::from_millis(100));
            std::thread::sleep(step);
            slept += step;
        }
    }
    log::info!("MIDI input stopped");
}

/// Lay a recognised controller's default mapping over whatever is
/// already bound, the first time it appears in a session.
///
/// Once per session, not once per connection: a cable knocked out and
/// pushed back in mid-set must not re-add bindings somebody deliberately
/// removed ten minutes earlier. Nothing is ever overwritten either way —
/// see [`profile::apply`].
fn offer_profile(port_name: &str, shared: &SharedMidi) {
    let Some(profile) = profile::for_port(port_name) else { return };
    let Ok(mut state) = shared.lock() else { return };
    if state.profiled.iter().any(|p| p == port_name) {
        return;
    }
    state.profiled.push(port_name.to_string());
    let added = profile::apply(&mut state.map, profile);
    if added > 0 {
        state.revision += 1;
        log::info!("{}: added {added} default bindings", profile.name);
    }
}

/// Owns the output connections and keeps the lights honest.
///
/// Entirely best-effort. Every failure here — no output port, a device
/// that refuses a note, a port that vanishes mid-send — costs lights and
/// nothing else. Input keeps working, and so does the show.
fn run_out(shared: SharedMidi, stop: Arc<AtomicBool>) {
    // Port name, connection, the profile that names its pads, and what
    // is currently lit on it.
    let mut open: Vec<(String, MidiOutputConnection, &'static profile::Profile, feedback::Surface)> =
        Vec::new();
    let mut since_scan = RESCAN_INTERVAL;

    while !stop.load(Ordering::Relaxed) {
        if since_scan >= RESCAN_INTERVAL {
            since_scan = Duration::ZERO;
            if let Err(e) = scan_outputs(&mut open) {
                log::debug!("MIDI output rescan failed: {e:#}");
            }
        }
        // One lock, one copy, then out of the way: the render thread
        // writes this every frame and must never wait on a port.
        let wanted = match shared.lock() {
            Ok(state) => state.surface,
            Err(_) => return,
        };
        for (name, conn, profile, lit) in &mut open {
            for msg in feedback::diff(lit, &wanted, profile) {
                if let Err(e) = conn.send(&msg) {
                    log::debug!("could not light {name}: {e}");
                    // Do not update `lit` — the next tick retries what
                    // this one failed to say, rather than believing a
                    // message that never arrived.
                    break;
                }
            }
            *lit = wanted;
        }
        std::thread::sleep(FEEDBACK_INTERVAL);
        since_scan += FEEDBACK_INTERVAL;
    }

    // Hand the devices back dark. Leaving a grid lit after quitting is
    // leaving the room with the lights on: nothing else can clear it.
    for (_, conn, profile, _) in &mut open {
        for msg in feedback::blackout(profile) {
            let _ = conn.send(&msg);
        }
    }
    log::info!("MIDI output stopped");
}

fn scan_outputs(
    open: &mut Vec<(String, MidiOutputConnection, &'static profile::Profile, feedback::Surface)>,
) -> Result<()> {
    let out = MidiOutput::new("vizz")?;
    let ports = out.ports();
    let names: Vec<String> = ports.iter().filter_map(|p| out.port_name(p).ok()).collect();
    open.retain(|(name, ..)| names.contains(name));

    for port in &ports {
        let Ok(name) = out.port_name(port) else { continue };
        if open.iter().any(|(n, ..)| n == &name) {
            continue;
        }
        // Only devices vizz knows the pad layout of. Blasting notes at
        // an unrecognised output is how you make somebody's synth play
        // a chord every time they load a scene.
        let Some(profile) = profile::for_port(&name) else { continue };
        if profile.lights.is_none() {
            continue;
        }
        let conn_out = MidiOutput::new("vizz")?;
        match conn_out.connect(port, "vizz-out") {
            Ok(mut conn) => {
                log::info!("lighting {name} as {}", profile.name);
                // Start from dark and from *knowing* it is dark, so the
                // first diff paints the true state onto a known ground
                // rather than onto whatever the last host left behind.
                for msg in feedback::blackout(profile) {
                    let _ = conn.send(&msg);
                }
                open.push((name, conn, profile, feedback::Surface::default()));
            }
            Err(e) => log::debug!("could not open MIDI output {name}: {e}"),
        }
    }
    Ok(())
}

fn scan_and_connect(
    registry: &Arc<ParamRegistry>,
    shared: &SharedMidi,
    open: &mut Vec<(String, MidiInputConnection<()>)>,
) -> Result<()> {
    let input = MidiInput::new("vizz")?;
    let ports = input.ports();
    let names: Vec<String> = ports
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect();

    // Drop connections whose device disappeared.
    let before = open.len();
    open.retain(|(name, _)| {
        let still_there = names.contains(name);
        if !still_there {
            log::info!("MIDI device disconnected: {name}");
        }
        still_there
    });
    // A learn armed when a device vanishes is a trap: it survives the
    // unplug-replug it was probably armed for, and the first stray event
    // from *any* device — a drifting fader on a controller across the
    // room — silently takes the binding. Disarm instead; re-arming is
    // one click, un-learning a wrong control is a hunt.
    if open.len() < before
        && let Ok(mut state) = shared.lock()
        && state.learn_target.take().is_some()
    {
        log::info!("MIDI learn cancelled — a device disconnected while it was armed");
    }

    for port in &ports {
        let Ok(name) = input.port_name(port) else { continue };
        if open.iter().any(|(n, _)| n == &name) {
            continue;
        }
        // Before the connection is built, because building it moves the
        // shared handle into the callback.
        offer_profile(&name, shared);
        // Each connection consumes a MidiInput, so build a fresh one.
        let conn_input = MidiInput::new("vizz")?;
        let registry = Arc::clone(registry);
        let shared = Arc::clone(shared);
        let mut dispatcher = Dispatcher::default();
        let label = name.clone();

        match conn_input.connect(
            port,
            "vizz-in",
            move |_stamp, bytes, _| {
                handle_message(bytes, &registry, &shared, &mut dispatcher);
            },
            (),
        ) {
            Ok(conn) => {
                log::info!("MIDI device connected: {label}");
                open.push((name, conn));
            }
            Err(e) => log::debug!("could not open MIDI port {label}: {e}"),
        }
    }

    if let Ok(mut state) = shared.lock() {
        state.connected = open.iter().map(|(n, _)| n.clone()).collect();
    }
    Ok(())
}

/// Called from midir's callback thread for every incoming message.
fn handle_message(
    bytes: &[u8],
    registry: &ParamRegistry,
    shared: &SharedMidi,
    dispatcher: &mut Dispatcher,
) {
    let Some(event) = message::parse(bytes) else { return };

    // Poisoned mutex would mean a panic elsewhere; dropping the message is
    // better than propagating the panic into the MIDI callback thread.
    let Ok(mut state) = shared.lock() else { return };
    // The realtime stream feeds the clock estimator and goes no further:
    // at 24 ticks a beat it must never arm a learn, land in last_source,
    // or reach the bindings.
    let Some(source) = Dispatcher::learn_source(event) else {
        state.clock.on_event(event, std::time::Instant::now());
        return;
    };
    state.last_source = Some(source);

    if state.apply_learn(source) {
        log::info!("MIDI learned: {} -> {:?}", source.label(), state.map.param_for(&source));
        return;
    }

    let resolved = dispatcher.resolve(event, &state.map);
    drop(state); // release before touching the registry

    if let Some((param, update)) = resolved
        && let Some(id) = registry.id(&param)
    {
        match update {
            Update::Range(t) => registry.set_normalized(id, t),
            // Straight through. `set` clamps to the parameter's range, so
            // a binding left behind by a parameter that has since shrunk
            // lands somewhere valid rather than being dropped.
            Update::Absolute(v) => registry.set(id, v),
        }
    }
}

/// Where mappings live by default.
pub fn default_map_path() -> PathBuf {
    // The same resolution the rest of the user state uses (patches,
    // presets, settings all honour XDG_CONFIG_HOME). This file was the
    // one hold-out hardcoding ~/.config, so a user with XDG set had
    // every file in one directory except the most laborious one to
    // recreate — and "back up the vizz folder" silently omitted it.
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    let path = base.join("vizz").join("midi.json");
    // Bring an existing map along. Before this, setting XDG_CONFIG_HOME
    // made a learned setup appear to vanish.
    if !path.exists()
        && let Some(legacy) = std::env::home_dir().map(|h| h.join(".config/vizz/midi.json"))
        && legacy != path
        && legacy.exists()
    {
        let moved = path
            .parent()
            .map(|dir| std::fs::create_dir_all(dir).is_ok())
            .unwrap_or(false)
            && std::fs::rename(&legacy, &path).is_ok();
        if moved {
            log::info!("moved the MIDI map from {} to {}", legacy.display(), path.display());
        } else {
            log::warn!("could not move the MIDI map from {}", legacy.display());
        }
    }
    path
}

/// Load a mapping file. A missing file is not an error — it just means no
/// mappings yet.
pub fn load_map(path: &Path) -> Result<MidiMap> {
    if !path.exists() {
        return Ok(MidiMap::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    match serde_json::from_str(&text) {
        Ok(map) => Ok(map),
        Err(e) => {
            // Two mitigations before giving up, because this file is the
            // most laborious setup in the app.
            //
            // Salvage: parsing was all-or-nothing, so one malformed
            // binding — a schema change, a hand-edit — cost every other
            // binding in the file. If the JSON itself parses, every
            // binding that still deserialises is kept.
            //
            // Quarantine: whatever happens, the damaged original is set
            // aside rather than left in place, because the next save
            // would overwrite it and turn corruption into permanent loss.
            let salvaged = salvage_map(&text);
            let mut broken = path.as_os_str().to_owned();
            broken.push(".broken");
            match std::fs::rename(path, &broken) {
                Ok(()) => log::warn!(
                    "set the unreadable MIDI map aside as {} — it may be hand-recoverable",
                    Path::new(&broken).display()
                ),
                Err(re) => log::warn!("could not set {} aside: {re}", path.display()),
            }
            match salvaged {
                Some((map, lost)) => {
                    log::warn!(
                        "{} was damaged ({e}) — recovered {} bindings, lost {lost}",
                        path.display(),
                        map.bindings.len()
                    );
                    Ok(map)
                }
                None => Err(e).with_context(|| format!("parsing {}", path.display())),
            }
        }
    }
}

/// Keep every binding that still deserialises from a damaged map file.
/// `None` when the text is not JSON at all — nothing to walk.
fn salvage_map(text: &str) -> Option<(MidiMap, usize)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let rows = value.get("bindings")?.as_array()?;
    let mut map = MidiMap::default();
    let mut lost = 0;
    for row in rows {
        match serde_json::from_value::<Binding>(row.clone()) {
            Ok(b) => map.bindings.push(b),
            Err(_) => lost += 1,
        }
    }
    Some((map, lost))
}

pub fn save_map(path: &Path, map: &MidiMap) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(map)?;
    // Temp file then rename, matching every other persisted artefact in
    // the workspace. A plain write truncates first, so a crash, a power
    // loss or a full disk during it leaves an empty or half-written file,
    // `load_map` fails, and the app starts with no mappings — losing the
    // most laborious setup in the app to the narrowest of windows.
    // Unique per process: two instances sharing one tmp name could
    // truncate each other mid-write and rename a torn file into place.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp.{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizz_params::ParamDef;

    fn registry() -> Arc<ParamRegistry> {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/master/dim", 0.0, 1.0, 1.0));
        b.add(ParamDef::new("/particles/hue", 0.0, 1.0, 0.5));
        // A slot parameter, to stand in for the real `/scene/fire`.
        b.add(ParamDef::new("/scene/fire", 0.0, 16.0, 0.0));
        Arc::new(b.build())
    }

    /// The estimator reads the wire tempo through realistic per-tick
    /// jitter — the median is what a USB scheduling spike cannot drag —
    /// and expires to no-opinion when the ticks stop.
    #[test]
    fn the_clock_estimator_reads_tempo_through_jitter() {
        let mut c = ClockEstimator::default();
        // 128 bpm: 19.53 ms per tick.
        let tick = std::time::Duration::from_secs_f64(60.0 / (128.0 * 24.0));
        let jitter = std::time::Duration::from_millis(2);
        let mut now = std::time::Instant::now();
        assert!(c.bpm(now).is_none(), "an empty estimator had an opinion");
        for i in 0..60 {
            now += if i % 2 == 0 { tick + jitter } else { tick - jitter };
            c.on_event(MidiEvent::Clock, now);
        }
        let bpm = c.bpm(now).expect("sixty ticks were not enough");
        assert!((bpm - 128.0).abs() < 0.5, "estimate off: {bpm}");
        // Silence is no opinion, never the last tempo frozen forever.
        assert!(
            c.bpm(now + std::time::Duration::from_secs(1)).is_none(),
            "estimate survived a second of silence"
        );
    }

    /// Start flags the downbeat exactly once and clears the measurement:
    /// intervals from before the pause must not skew the fresh transport.
    #[test]
    fn start_flags_the_downbeat_once_and_resets_the_measurement() {
        let mut c = ClockEstimator::default();
        let tick = std::time::Duration::from_secs_f64(60.0 / (120.0 * 24.0));
        let mut now = std::time::Instant::now();
        for _ in 0..30 {
            now += tick;
            c.on_event(MidiEvent::Clock, now);
        }
        assert!(c.bpm(now).is_some());
        assert!(!c.take_started(), "started before any Start arrived");

        c.on_event(MidiEvent::Start, now);
        assert!(c.take_started(), "Start was not flagged");
        assert!(!c.take_started(), "Start flagged twice for one event");
        assert!(c.bpm(now).is_none(), "old intervals survived the Start");
    }

    /// An armed learn must survive a storm of clock ticks unbound — at
    /// 24 ticks a beat the first tick would otherwise take the binding
    /// before the user's hand reached the control it was armed for.
    #[test]
    fn an_armed_learn_survives_a_clock_storm() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        shared.lock().unwrap().learn_target = Some(LearnTarget::param("/master/dim"));
        let mut d = Dispatcher::default();

        for _ in 0..100 {
            handle_message(&[0xF8], &reg, &shared, &mut d);
        }
        handle_message(&[0xFA], &reg, &shared, &mut d);
        {
            let state = shared.lock().unwrap();
            assert!(state.learn_target.is_some(), "clock stole the learn");
            assert!(state.last_source.is_none(), "clock landed in last_source");
        }

        // The control the learn was actually waiting for still binds.
        handle_message(&[0xB0, 7, 64], &reg, &shared, &mut d);
        let state = shared.lock().unwrap();
        assert!(state.learn_target.is_none(), "the real control did not bind");
    }

    /// The whole path a real message takes, minus the hardware: parse ->
    /// resolve -> write a normalised value into the registry.
    #[test]
    fn a_bound_cc_moves_the_parameter() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        shared
            .lock()
            .unwrap()
            .map
            .bind(Source::ControlChange { channel: 0, controller: 7 }, "/master/dim");
        let mut d = Dispatcher::default();

        handle_message(&[0xB0, 7, 0], &reg, &shared, &mut d);
        assert_eq!(reg.target(reg.id("/master/dim").unwrap()), 0.0);

        handle_message(&[0xB0, 7, 127], &reg, &shared, &mut d);
        assert_eq!(reg.target(reg.id("/master/dim").unwrap()), 1.0);
    }

    #[test]
    fn learn_binds_the_next_control_and_does_not_also_move_it() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        shared.lock().unwrap().learn_target = Some(LearnTarget::param("/particles/hue"));
        let mut d = Dispatcher::default();

        let before = reg.target(reg.id("/particles/hue").unwrap());
        handle_message(&[0xB2, 20, 100], &reg, &shared, &mut d);

        let state = shared.lock().unwrap();
        assert_eq!(
            state.map.param_for(&Source::ControlChange { channel: 2, controller: 20 }),
            Some("/particles/hue")
        );
        assert!(state.learn_target.is_none(), "learn mode must clear itself");
        assert_eq!(state.revision, 1);
        // The message that taught the binding should not also jump the
        // value — otherwise learning snaps the parameter wherever the
        // knob happened to be.
        assert_eq!(reg.target(reg.id("/particles/hue").unwrap()), before);
    }

    /// The whole path, because the bug lived in the seam between
    /// resolving an event and writing it: `resolve` returned a position
    /// and the writer spread it across the range, so a button on a slot
    /// parameter could only ever reach the top.
    #[test]
    fn a_learned_pad_writes_its_own_slot_and_not_the_top_of_the_range() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        let fire = reg.id("/scene/fire").unwrap();
        shared.lock().unwrap().learn_target =
            Some(LearnTarget::value("/scene/fire", 3.0, "scene 3"));
        let mut d = Dispatcher::default();

        // Learn from note 36, full velocity.
        handle_message(&[0x90, 36, 127], &reg, &shared, &mut d);
        assert!(shared.lock().unwrap().learn_target.is_none());

        // Now press it for real. A plain note binding would land on 16.
        handle_message(&[0x90, 36, 127], &reg, &shared, &mut d);
        assert_eq!(reg.target(fire), 3.0);
        handle_message(&[0x80, 36, 0], &reg, &shared, &mut d);
        assert_eq!(reg.target(fire), 0.0, "release should rest at nothing-selected");
    }

    #[test]
    fn unknown_parameter_addresses_are_ignored() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        // A mapping file naming a parameter this build no longer has must
        // not panic or abort — it is simply inert.
        shared
            .lock()
            .unwrap()
            .map
            .bind(Source::ControlChange { channel: 0, controller: 7 }, "/gone/away");
        let mut d = Dispatcher::default();
        handle_message(&[0xB0, 7, 64], &reg, &shared, &mut d);
    }

    #[test]
    fn unmappable_traffic_is_ignored() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        let mut d = Dispatcher::default();
        // Clock floods at ~24 messages per beat; it must never set learn
        // state or reach the registry.
        for _ in 0..100 {
            handle_message(&[0xF8], &reg, &shared, &mut d);
        }
        assert!(shared.lock().unwrap().last_source.is_none());
    }

    #[test]
    fn map_file_round_trips_and_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("vizz-midi-test-{}", std::process::id()));
        let path = dir.join("midi.json");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(load_map(&path).unwrap().bindings.is_empty(), "missing file");

        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 1, controller: 7 }, "/master/dim");
        // Also proves the parent directory is created.
        save_map(&path, &map).unwrap();
        assert_eq!(load_map(&path).unwrap(), map);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Parsing was all-or-nothing: one malformed binding — a schema
    /// change, a hand-edit gone wrong — cost every other binding in the
    /// file, which is the most laborious setup in the app. Every binding
    /// that still deserialises is kept, and the damaged original is set
    /// aside where the next save cannot clobber it.
    #[test]
    fn a_damaged_map_keeps_its_good_bindings_and_the_original_is_set_aside() {
        let dir = std::env::temp_dir().join(format!("vizz-midi-salvage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("midi.json");
        let damaged = r#"{"bindings":[
            {"source":{"kind":"control_change","channel":0,"controller":7},"param":"/master/dim"},
            {"source":{"kind":"note","channel":"NOT A NUMBER","note":36},"param":"/scene/fire"},
            {"source":{"kind":"note","channel":9,"note":36},"param":"/particles/speed"}
        ]}"#;
        std::fs::write(&path, damaged).unwrap();

        let map = load_map(&path).expect("salvage should succeed");
        assert_eq!(map.bindings.len(), 2, "both intact bindings kept");
        assert_eq!(map.bindings[0].param, "/master/dim");
        assert_eq!(map.bindings[1].param, "/particles/speed");

        let broken = dir.join("midi.json.broken");
        assert_eq!(
            std::fs::read_to_string(&broken).expect("original set aside"),
            damaged
        );
        assert!(!path.exists(), "the damaged file must not stay in the save path");
    }

    #[test]
    fn corrupt_map_file_reports_an_error_rather_than_panicking() {
        let dir = std::env::temp_dir().join(format!("vizz-midi-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("midi.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        assert!(load_map(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Quitting joins the rescan thread, and the rescan pause is two
    /// seconds. Dropping the engine mid-pause used to wait the pause out,
    /// which read as the app hanging on quit. The bound here is generous
    /// on purpose — half the old stall, several times the sliced sleep —
    /// so it fails on the bug, not on a slow CI machine.
    #[test]
    fn quitting_does_not_wait_out_the_rescan_pause() {
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        let engine = MidiEngine::spawn(registry(), shared).expect("spawn");
        // Let the thread get past the first scan and into the pause.
        std::thread::sleep(Duration::from_millis(300));
        let quit = std::time::Instant::now();
        drop(engine);
        assert!(
            quit.elapsed() < Duration::from_secs(1),
            "quit stalled {:?} joining the MIDI thread",
            quit.elapsed()
        );
    }
}
