//! Spectral analysis: windowed FFT, configurable bands, envelope following.
//!
//! Deliberately separate from device capture. Everything here is a pure
//! function of a sample slice, which means it can be tested against
//! synthetic signals — a sine at a known frequency, a click train at a
//! known tempo — on a machine with no sound card at all. Capture is the
//! thin untestable part; this is where the behaviour lives.

use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

/// 2048 at 48 kHz is a 43 ms window and 23 Hz per bin: fine enough to
/// separate a kick from a bassline, short enough that a transient does not
/// smear across several frames.
pub const FFT_SIZE: usize = 2048;
/// Advance per analysis frame. 512 gives ~94 frames/sec, which is enough
/// temporal resolution for onset detection to place a beat accurately.
pub const HOP: usize = 512;

/// One band of the spectrum, with its own gain and envelope timing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub lo_hz: f32,
    pub hi_hz: f32,
    /// Linear gain applied before the 0..1 clamp. This is the "sensitivity"
    /// control: recorded material varies over orders of magnitude, and no
    /// automatic normaliser survives a track that opens on silence.
    pub gain: f32,
    /// Seconds to rise. Short, so transients are not blunted.
    pub attack: f32,
    /// Seconds to fall. Longer than attack, or every band flickers.
    pub release: f32,
}

impl Band {
    pub const fn new(lo_hz: f32, hi_hz: f32) -> Self {
        Self { lo_hz, hi_hz, gain: 1.0, attack: 0.005, release: 0.12 }
    }

    /// A band whose sensitivity is given in decibels.
    pub fn at_db(lo_hz: f32, hi_hz: f32, db: f32) -> Self {
        Self { gain: db_to_linear(db), ..Self::new(lo_hz, hi_hz) }
    }

    pub fn gain_db(&self) -> f32 {
        linear_to_db(self.gain)
    }

    pub fn set_gain_db(&mut self, db: f32) {
        self.gain = db_to_linear(db);
    }
}

/// Quietest gain the control offers. A band turned fully down should be
/// off, not merely quiet, so this is far enough below unity to be silent
/// against anything.
pub const MIN_GAIN_DB: f32 = -24.0;
/// Loudest. A line input running at a fraction of full scale needs a great
/// deal of gain in the top band — see [`default_bands`] — and a ceiling
/// that cannot reach it is a ceiling that makes the feature look broken.
pub const MAX_GAIN_DB: f32 = 54.0;

/// Where a band should peak once its gain is set: high enough to use the
/// range, short of the clamp so transients still have somewhere to go.
pub const TARGET_PEAK: f32 = 0.9;

pub fn db_to_linear(db: f32) -> f32 {
    10.0f32.powf(db.clamp(MIN_GAIN_DB, MAX_GAIN_DB) / 20.0)
}

/// Linear gain as decibels. Guards zero, which has no logarithm and would
/// otherwise put `-inf` in a spin box.
pub fn linear_to_db(gain: f32) -> f32 {
    if gain <= 0.0 {
        return MIN_GAIN_DB;
    }
    (20.0 * gain.log10()).clamp(MIN_GAIN_DB, MAX_GAIN_DB)
}

/// Four bands rather than a full analyser: enough to drive different
/// parameters from different parts of a mix, few enough to stay playable.
///
/// The gains are in decibels because that is the unit a sensitivity
/// control is read in everywhere else in audio, and because it is the unit
/// that makes these numbers comparable to each other: the top band is not
/// "ten" against the kick band's "six", it is fifteen decibels hotter, and
/// that is a statement about how mixes are built rather than an arbitrary
/// multiplier.
///
/// They are set so a band peaks near [`TARGET_PEAK`] on a track at a
/// healthy input level. A band's RMS is a small fraction of full scale
/// once the spectrum is split four ways — a few percent for the top band —
/// so these are much larger than they look. Erring high is deliberate:
/// too much gain shows up as a clipped meter you turn down, while too
/// little shows up as visuals that barely move, which reads as the audio
/// input not working at all. The `fit` button sets them from what is
/// actually arriving, which is the answer for any particular rig.
pub fn default_bands() -> [Band; BAND_COUNT] {
    [
        // Kick and sub. Narrow on purpose — widening this is the fastest
        // way to make everything pump at once.
        Band::at_db(30.0, 110.0, 18.0),
        // Bassline and low body.
        Band::at_db(110.0, 400.0, 18.0),
        // Vocals, snare, most melodic content.
        Band::at_db(400.0, 2000.0, 24.0),
        // Hats and air. A few percent of full scale in most mixes, so it
        // needs far more than the rest — this was the band that looked
        // dead at the old defaults.
        Band::at_db(2000.0, 12000.0, 34.0),
    ]
}

/// The gain that would put `peak` at [`TARGET_PEAK`], in decibels.
///
/// Returns `None` for a band with nothing in it: a silent input would
/// otherwise ask for infinite gain, and the useful behaviour is to leave
/// that band exactly as the performer set it.
pub fn fit_gain_db(peak: f32) -> Option<f32> {
    if !(peak > 1e-5) {
        return None;
    }
    Some(linear_to_db(TARGET_PEAK / peak))
}

pub const BAND_COUNT: usize = 4;

/// What one analysis frame produced.
#[derive(Debug, Clone, Copy, Default)]
pub struct Frame {
    /// Post-gain, post-envelope band levels, 0..1 — the modulation sources.
    pub bands: [f32; BAND_COUNT],
    /// Pre-gain band levels. The UI meters these so the gain can be set
    /// against what is actually arriving rather than against the clamp.
    pub raw: [f32; BAND_COUNT],
    /// Broadband RMS of the window, pre-gain.
    pub level: f32,
    /// Positive spectral flux: how much energy *appeared* since the last
    /// frame. The onset signal beat detection runs on.
    pub flux: f32,
}

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Parseval + window-power correction; see `new`.
    norm: f32,
    scratch: Vec<Complex<f32>>,
    mags: Vec<f32>,
    prev_mags: Vec<f32>,
    env: [f32; BAND_COUNT],
    sample_rate: f32,
    have_prev: bool,
}

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        // Periodic Hann, the correct variant for spectral analysis (the
        // symmetric one leaks slightly when used for overlapping frames).
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let x = std::f32::consts::TAU * i as f32 / FFT_SIZE as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        Self {
            fft,
            window,
            // Parseval, corrected for the window's power loss, so a band
            // reads the RMS of the signal within it. That makes band levels
            // and `level` the same kind of number, which is what lets a
            // meter and a gain control mean anything. Hann has a mean
            // square of 3/8, and the one-sided spectrum needs doubling for
            // the mirrored negative frequencies: sqrt(2/(3/8)) / N.
            norm: (16.0f32 / 3.0).sqrt() / FFT_SIZE as f32,
            scratch: vec![Complex { re: 0.0, im: 0.0 }; FFT_SIZE],
            mags: vec![0.0; FFT_SIZE / 2],
            prev_mags: vec![0.0; FFT_SIZE / 2],
            env: [0.0; BAND_COUNT],
            sample_rate: sample_rate.max(1.0),
            have_prev: false,
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Analyse one window. `samples` must be `FFT_SIZE` long; `dt` is the
    /// time this frame represents, for the envelope followers.
    pub fn analyze(&mut self, samples: &[f32], bands: &[Band; BAND_COUNT], dt: f32) -> Frame {
        debug_assert_eq!(samples.len(), FFT_SIZE);

        let mut sum_sq = 0.0f32;
        for (i, (&s, &w)) in samples.iter().zip(&self.window).enumerate() {
            self.scratch[i] = Complex { re: s * w, im: 0.0 };
            sum_sq += s * s;
        }
        self.fft.process(&mut self.scratch);

        std::mem::swap(&mut self.mags, &mut self.prev_mags);
        for (m, c) in self.mags.iter_mut().zip(&self.scratch[..FFT_SIZE / 2]) {
            *m = c.norm() * self.norm;
        }

        // Positive-only difference: energy leaving the spectrum is a note
        // ending, which is not an onset. Summing only increases is what
        // makes this track attacks rather than amplitude.
        let flux = if self.have_prev {
            self.mags
                .iter()
                .zip(&self.prev_mags)
                .map(|(&m, &p)| (m - p).max(0.0))
                .sum()
        } else {
            0.0
        };
        self.have_prev = true;

        let mut out = Frame {
            level: (sum_sq / FFT_SIZE as f32).sqrt(),
            flux,
            ..Default::default()
        };

        let hz_per_bin = self.sample_rate / FFT_SIZE as f32;
        for (i, band) in bands.iter().enumerate() {
            // Round both edges the same way and stop one bin short of the
            // next band's start. Rounding lo down and hi up instead makes
            // adjacent bands share their boundary bins, so a tone sitting
            // on a crossover drives both bands at once.
            let lo = (band.lo_hz / hz_per_bin).round().max(1.0) as usize;
            let hi = ((band.hi_hz / hz_per_bin).round() as usize)
                .saturating_sub(1)
                .min(self.mags.len() - 1);
            let raw = if hi >= lo {
                let energy: f32 = self.mags[lo..=hi].iter().map(|m| m * m).sum();
                energy.sqrt()
            } else {
                0.0
            };
            out.raw[i] = raw;

            let target = (raw * band.gain).clamp(0.0, 1.0);
            // Asymmetric one-pole: fast up so a kick lands on the frame it
            // happened, slow down so the value is usable as a modulator
            // rather than a strobe.
            let tau = if target > self.env[i] { band.attack } else { band.release };
            let k = if tau <= f32::EPSILON { 1.0 } else { 1.0 - (-dt / tau).exp() };
            self.env[i] += (target - self.env[i]) * k;
            out.bands[i] = self.env[i];
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(freq: f32, amp: f32) -> Vec<f32> {
        (0..FFT_SIZE)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * amp)
            .collect()
    }

    /// Run a signal long enough for the envelopes to settle.
    fn settle(a: &mut Analyzer, sig: &[f32], bands: &[Band; BAND_COUNT]) -> Frame {
        let mut f = Frame::default();
        for _ in 0..80 {
            f = a.analyze(sig, bands, HOP as f32 / SR);
        }
        f
    }

    /// A tone must light the band containing it and leave the others dark.
    /// This is the test that catches an off-by-one in bin/frequency
    /// conversion, which would otherwise just look like "the bands feel a
    /// bit wrong".
    #[test]
    fn a_tone_lights_only_its_own_band() {
        let bands = default_bands();
        // One frequency per band, comfortably inside it.
        for (i, freq) in [60.0, 200.0, 900.0, 6000.0].into_iter().enumerate() {
            let mut a = Analyzer::new(SR);
            let f = settle(&mut a, &sine(freq, 0.8), &bands);
            assert!(f.raw[i] > 0.3, "band {i} did not respond to {freq} Hz: {:?}", f.raw);
            for (j, &other) in f.raw.iter().enumerate() {
                if j != i {
                    assert!(
                        other < f.raw[i] * 0.1,
                        "{freq} Hz leaked into band {j}: {:?}",
                        f.raw
                    );
                }
            }
        }
    }

    /// A band should read the RMS of the signal inside it. Asserting that
    /// against the independently-computed broadband RMS pins the window
    /// correction and the Parseval scaling together — get either wrong and
    /// the two numbers diverge, which is what makes a gain control
    /// unpredictable across material.
    #[test]
    fn band_level_equals_signal_rms() {
        let mut a = Analyzer::new(SR);
        let bands = [Band::new(20.0, 20_000.0); BAND_COUNT];
        let f = settle(&mut a, &sine(1000.0, 1.0), &bands);
        // A full-scale sine has an RMS of 1/sqrt(2).
        assert!(
            (f.level - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01,
            "broadband RMS wrong: {}",
            f.level
        );
        assert!(
            (f.raw[0] - f.level).abs() < 0.03,
            "band {} does not agree with broadband RMS {}",
            f.raw[0],
            f.level
        );
    }

    #[test]
    fn silence_produces_nothing() {
        let mut a = Analyzer::new(SR);
        let bands = default_bands();
        let f = settle(&mut a, &vec![0.0; FFT_SIZE], &bands);
        assert!(f.level < 1e-6, "level {}", f.level);
        assert!(f.bands.iter().all(|&b| b < 1e-6), "{:?}", f.bands);
        assert!(f.flux < 1e-6, "flux {}", f.flux);
    }

    /// Gain is the user's sensitivity control, so it has to actually scale
    /// the output and the clamp has to hold at the top.
    #[test]
    fn gain_scales_and_clamps() {
        let bands_lo = [Band { gain: 0.5, ..Band::new(20.0, 20_000.0) }; BAND_COUNT];
        let bands_hi = [Band { gain: 8.0, ..Band::new(20.0, 20_000.0) }; BAND_COUNT];
        let sig = sine(1000.0, 0.5);

        let quiet = settle(&mut Analyzer::new(SR), &sig, &bands_lo).bands[0];
        let loud = settle(&mut Analyzer::new(SR), &sig, &bands_hi).bands[0];
        assert!(loud > quiet, "gain did not raise the level: {quiet} -> {loud}");
        assert!(loud <= 1.0, "gain escaped the clamp: {loud}");
        assert!(quiet < 0.5, "expected a low reading at 0.5 gain, got {quiet}");
    }

    /// Flux must respond to energy *arriving*, not to energy present, or
    /// beat detection would follow the envelope instead of the attacks.
    #[test]
    fn flux_marks_onsets_not_sustain() {
        let mut a = Analyzer::new(SR);
        let bands = default_bands();
        let quiet = sine(440.0, 0.05);
        let loud = sine(440.0, 1.0);

        for _ in 0..10 {
            a.analyze(&quiet, &bands, HOP as f32 / SR);
        }
        let onset = a.analyze(&loud, &bands, HOP as f32 / SR).flux;
        let sustain = a.analyze(&loud, &bands, HOP as f32 / SR).flux;
        // Falling back to quiet must not register as an onset either.
        let release = a.analyze(&quiet, &bands, HOP as f32 / SR).flux;

        assert!(onset > 0.1, "onset produced no flux: {onset}");
        assert!(sustain < onset * 0.2, "sustain looked like an onset: {sustain} vs {onset}");
        assert!(release < onset * 0.2, "release looked like an onset: {release} vs {onset}");
    }

    /// The anchors anyone reading a dB number relies on: unity is 0, twice
    /// is 6, ten times is 20. Get these wrong and every gain in the panel
    /// is quietly mislabelled.
    #[test]
    fn decibels_convert_the_way_decibels_do() {
        assert!((linear_to_db(1.0)).abs() < 1e-4, "unity is 0 dB");
        assert!((linear_to_db(2.0) - 6.0206).abs() < 1e-3);
        assert!((linear_to_db(10.0) - 20.0).abs() < 1e-3);
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-4);
        assert!((db_to_linear(20.0) - 10.0).abs() < 1e-3);
        for db in [MIN_GAIN_DB, -6.0, 0.0, 12.0, 33.0, MAX_GAIN_DB] {
            let round_trip = linear_to_db(db_to_linear(db));
            assert!((round_trip - db).abs() < 1e-3, "{db} dB came back as {round_trip}");
        }
    }

    /// Zero has no logarithm. A band dragged to the bottom, or one that has
    /// never seen signal, must not put `-inf` in a spin box.
    #[test]
    fn a_silent_gain_has_a_finite_label() {
        assert_eq!(linear_to_db(0.0), MIN_GAIN_DB);
        assert!(linear_to_db(-1.0).is_finite());
        assert!(db_to_linear(f32::NEG_INFINITY).is_finite());
        assert!(db_to_linear(1e9).is_finite());
    }

    /// `fit` has to actually land the band where it says: feed it a peak,
    /// apply the gain it asks for, and the peak should sit at the target.
    #[test]
    fn fitting_a_gain_puts_the_peak_where_it_belongs() {
        for peak in [0.5f32, 0.12, 0.02, 0.004] {
            let db = fit_gain_db(peak).expect("a real peak should fit");
            let landed = peak * db_to_linear(db);
            assert!(
                (landed - TARGET_PEAK).abs() < 0.02,
                "peak {peak} fitted to {db} dB landed at {landed}"
            );
        }
    }

    /// Silence asks for infinite gain. The useful answer is to leave the
    /// band alone rather than to drive noise up to full scale.
    #[test]
    fn fitting_silence_changes_nothing() {
        assert_eq!(fit_gain_db(0.0), None);
        assert_eq!(fit_gain_db(1e-9), None);
        assert_eq!(fit_gain_db(-1.0), None);
        assert_eq!(fit_gain_db(f32::NAN), None);
    }

    /// The complaint that started this: at the shipped gains the top band
    /// barely moved on real material, so anything routed from it looked
    /// broken. A band fed a level typical of its part of the spectrum has
    /// to reach a useful fraction of the range — a modulation source stuck
    /// near zero is one you cannot hear working.
    #[test]
    fn every_default_band_reaches_a_useful_level_on_a_typical_mix() {
        // Rough per-band RMS for a track at a healthy input level. The top
        // band really is this quiet, which is the whole point.
        let typical = [0.10f32, 0.085, 0.055, 0.012];
        for (i, band) in default_bands().iter().enumerate() {
            let level = (typical[i] * band.gain).clamp(0.0, 1.0);
            assert!(
                level > 0.45,
                "band {i} ({}–{} Hz, {:.0} dB) only reaches {level:.2}",
                band.lo_hz,
                band.hi_hz,
                band.gain_db()
            );
            // And not so hot that it is pinned before the music starts.
            assert!(level < 1.0, "band {i} is already clipped at typical levels");
        }
    }
}
