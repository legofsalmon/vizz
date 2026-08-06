//! Tempo estimation from the onset signal, and tap tempo.
//!
//! Autocorrelation of spectral flux: if the music has a pulse, the onset
//! signal correlates with itself at the beat period. Peak-picking the
//! correlation over the lags that correspond to plausible tempos gives the
//! BPM without ever having to decide what "a beat" is.
//!
//! The classic failure is the octave error — reporting 180 for a 90 BPM
//! track, or 70 for 140 — because a signal that correlates at one period
//! also correlates at its multiples. Two things guard against it here: a
//! prior that prefers the middle of the range, and a check for whether
//! half the candidate tempo explains the signal comparably well.

use std::time::{Duration, Instant};

use crate::analysis::HOP;

/// Tempo range considered. Wider than most sets need, deliberately: it is
/// better to report an unusual tempo than to fold it into the range.
pub const MIN_BPM: f32 = 60.0;
pub const MAX_BPM: f32 = 200.0;

/// Seconds of onset history. Long enough to hold several bars at the slow
/// end, short enough to follow a track change within a phrase.
const HISTORY_SECS: f32 = 6.0;

pub struct BeatDetector {
    /// Onset strength, one entry per analysis frame.
    flux: Vec<f32>,
    write: usize,
    filled: usize,
    frame_rate: f32,
    bpm: f32,
    confidence: f32,
}

impl BeatDetector {
    pub fn new(sample_rate: f32) -> Self {
        let frame_rate = sample_rate / HOP as f32;
        let len = (frame_rate * HISTORY_SECS).ceil() as usize;
        Self {
            flux: vec![0.0; len.max(64)],
            write: 0,
            filled: 0,
            frame_rate,
            bpm: 0.0,
            confidence: 0.0,
        }
    }

    /// Last accepted estimate, or 0 if nothing is confident yet.
    pub fn bpm(&self) -> f32 {
        self.bpm
    }

    /// 0..1. Below ~0.2 the estimate should not be trusted — ambient
    /// material with no pulse will still produce *a* peak.
    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    pub fn reset(&mut self) {
        self.flux.fill(0.0);
        self.filled = 0;
        self.write = 0;
        self.bpm = 0.0;
        self.confidence = 0.0;
    }

    /// Feed one analysis frame's flux. Returns true when a new estimate was
    /// produced (which is far less often than this is called).
    pub fn push(&mut self, flux: f32) -> bool {
        self.flux[self.write] = flux;
        self.write = (self.write + 1) % self.flux.len();
        self.filled = (self.filled + 1).min(self.flux.len());

        // Re-estimating every frame would be wasted work; the tempo cannot
        // meaningfully change at 94 Hz. Once every ~0.5 s is plenty.
        let period = (self.frame_rate * 0.5) as usize;
        if self.filled < self.flux.len() || period == 0 || !self.write.is_multiple_of(period) {
            return false;
        }
        self.estimate();
        true
    }

    fn estimate(&mut self) {
        // Unwrap the ring into time order, then remove the mean: an offset
        // correlates with itself at every lag and would swamp the pulse.
        let n = self.flux.len();
        let mut x: Vec<f32> = (0..n).map(|i| self.flux[(self.write + i) % n]).collect();
        let mean = x.iter().sum::<f32>() / n as f32;
        for v in &mut x {
            *v -= mean;
        }

        let energy: f32 = x.iter().map(|v| v * v).sum();
        if energy <= f32::EPSILON {
            self.confidence = 0.0;
            return;
        }

        let lag_of = |bpm: f32| (self.frame_rate * 60.0 / bpm).round() as usize;
        let min_lag = lag_of(MAX_BPM).max(2);
        let max_lag = lag_of(MIN_BPM).min(n / 2);
        if max_lag <= min_lag {
            self.confidence = 0.0;
            return;
        }

        let corr = |lag: usize| -> f32 {
            let s: f32 = (0..n - lag).map(|i| x[i] * x[i + lag]).sum();
            s / energy
        };

        let mut scores = vec![0.0f32; max_lag + 1];
        let mut best = min_lag;
        for lag in min_lag..=max_lag {
            let bpm = self.frame_rate * 60.0 / lag as f32;
            // Log-normal prior around 120 BPM. Weak enough that a genuine
            // 75 or 170 still wins on its own merits, strong enough to
            // break ties between a tempo and its octave.
            let prior = (-0.5 * ((bpm / 120.0).ln() / 0.9).powi(2)).exp();
            scores[lag] = corr(lag) * prior;
            if scores[lag] > scores[best] {
                best = lag;
            }
        }

        // Octave check: if double the winning lag (half the tempo) also
        // correlates strongly, the faster reading was probably counting
        // off-beats. Prefer the slower one only when it is genuinely
        // comparable, so a real fast track is not halved.
        let mut chosen = best;
        let double = best * 2;
        if double <= max_lag && scores[double] > scores[best] * 0.85 {
            chosen = double;
        }

        let raw = corr(chosen);
        self.confidence = raw.clamp(0.0, 1.0);
        if self.confidence > 0.05 {
            self.bpm = self.frame_rate * 60.0 / chosen as f32;
        }
    }
}

/// Tap tempo: the manual half. Averages recent intervals and throws the
/// series away after a pause, so a fresh set of taps is never contaminated
/// by the last one.
pub struct TapTempo {
    taps: Vec<Instant>,
}

/// Longer than the slowest tap we would accept (60 BPM = 1 s), with room
/// for an unsteady hand.
const TAP_TIMEOUT: Duration = Duration::from_millis(2200);

impl Default for TapTempo {
    fn default() -> Self {
        Self::new()
    }
}

impl TapTempo {
    pub fn new() -> Self {
        Self { taps: Vec::new() }
    }

    /// Register a tap. Returns a BPM once there are enough to mean
    /// something (two taps is one interval, which is too jittery to use).
    pub fn tap(&mut self) -> Option<f32> {
        self.tap_at(Instant::now())
    }

    /// Injectable clock, so the averaging and timeout can be tested without
    /// sleeping through them.
    pub fn tap_at(&mut self, now: Instant) -> Option<f32> {
        if let Some(&last) = self.taps.last()
            && now.duration_since(last) > TAP_TIMEOUT {
                self.taps.clear();
            }
        self.taps.push(now);
        // Keep the last few only: a tempo set eight taps ago should not
        // drag against where the user is tapping now.
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        if self.taps.len() < 3 {
            return None;
        }
        let span = self.taps.last()?.duration_since(self.taps[0]).as_secs_f32();
        let intervals = (self.taps.len() - 1) as f32;
        if span <= 0.0 {
            return None;
        }
        let bpm = 60.0 * intervals / span;
        (MIN_BPM..=MAX_BPM).contains(&bpm).then_some(bpm)
    }

    pub fn clear(&mut self) {
        self.taps.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Analyzer, FFT_SIZE, default_bands};

    const SR: f32 = 48_000.0;

    /// Synthesise a click track: short broadband bursts at a fixed tempo,
    /// run through the real analyser so the detector sees exactly what it
    /// would see live.
    fn detect(bpm: f32, secs: f32) -> (f32, f32) {
        let mut a = Analyzer::new(SR);
        let mut d = BeatDetector::new(SR);
        let bands = default_bands();
        let dt = HOP as f32 / SR;
        let period = SR * 60.0 / bpm;

        let frames = (secs * SR / HOP as f32) as usize;
        let mut buf = vec![0.0f32; FFT_SIZE];
        // Deterministic pseudo-noise for the click body — a bare impulse is
        // too short to survive windowing reliably.
        let mut seed = 0x12345678u32;
        let mut rng = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
        };

        for f in 0..frames {
            let start = f * HOP;
            for (i, s) in buf.iter_mut().enumerate() {
                let t = (start + i) as f32;
                let since = t % period;
                // 12 ms decaying burst.
                *s = if since < SR * 0.012 {
                    rng() * (1.0 - since / (SR * 0.012))
                } else {
                    0.0
                };
            }
            let frame = a.analyze(&buf, &bands, dt);
            d.push(frame.flux);
        }
        (d.bpm(), d.confidence())
    }

    /// The headline behaviour: a click track at a known tempo must come
    /// back as that tempo.
    #[test]
    fn detects_tempo_of_a_click_track() {
        for target in [90.0f32, 120.0, 128.0, 174.0] {
            let (bpm, conf) = detect(target, 12.0);
            assert!(conf > 0.2, "{target} BPM: no confidence ({conf})");
            // Lag quantisation alone is worth ~1 BPM at the fast end.
            assert!(
                (bpm - target).abs() < 3.0,
                "expected ~{target} BPM, got {bpm} (confidence {conf})"
            );
        }
    }

    /// Octave errors are the characteristic failure of this method, so
    /// assert specifically that a slow track is not reported at double.
    #[test]
    fn does_not_double_a_slow_tempo() {
        let (bpm, _) = detect(75.0, 14.0);
        assert!(
            (bpm - 75.0).abs() < 4.0,
            "octave error: reported {bpm} for a 75 BPM track"
        );
    }

    /// Material with no pulse must report low confidence rather than a
    /// confident wrong answer — the UI uses this to decide whether to let
    /// auto-BPM drive the clock.
    #[test]
    fn unpulsed_material_is_not_confident() {
        let mut a = Analyzer::new(SR);
        let mut d = BeatDetector::new(SR);
        let bands = default_bands();
        // A steady tone: plenty of energy, no onsets.
        let sig: Vec<f32> = (0..FFT_SIZE)
            .map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.5)
            .collect();
        for _ in 0..1400 {
            let f = a.analyze(&sig, &bands, HOP as f32 / SR);
            d.push(f.flux);
        }
        assert!(
            d.confidence() < 0.2,
            "sustained tone reported confidence {}",
            d.confidence()
        );
    }

    #[test]
    fn tap_tempo_averages_intervals() {
        let mut t = TapTempo::new();
        let t0 = Instant::now();
        // 500 ms apart = 120 BPM.
        let step = Duration::from_millis(500);
        assert!(t.tap_at(t0).is_none(), "one tap is not a tempo");
        assert!(t.tap_at(t0 + step).is_none(), "two taps is too jittery");
        let bpm = t.tap_at(t0 + step * 2).expect("three taps should give a tempo");
        assert!((bpm - 120.0).abs() < 0.5, "got {bpm}");
    }

    #[test]
    fn tap_tempo_restarts_after_a_pause() {
        let mut t = TapTempo::new();
        let t0 = Instant::now();
        let fast = Duration::from_millis(400);
        t.tap_at(t0);
        t.tap_at(t0 + fast);
        t.tap_at(t0 + fast * 2);
        // Long gap, then a new series at a different tempo. The old taps
        // must not drag the answer.
        let t1 = t0 + Duration::from_secs(10);
        let slow = Duration::from_millis(600);
        assert!(t.tap_at(t1).is_none(), "series should have been cleared");
        assert!(t.tap_at(t1 + slow).is_none());
        let bpm = t.tap_at(t1 + slow * 2).expect("new series should resolve");
        assert!((bpm - 100.0).abs() < 0.5, "stale taps leaked in: {bpm}");
    }

    #[test]
    fn tap_tempo_rejects_implausible_rates() {
        let mut t = TapTempo::new();
        let t0 = Instant::now();
        let frantic = Duration::from_millis(50); // 1200 BPM
        t.tap_at(t0);
        t.tap_at(t0 + frantic);
        assert!(t.tap_at(t0 + frantic * 2).is_none(), "accepted 1200 BPM");
    }
}
