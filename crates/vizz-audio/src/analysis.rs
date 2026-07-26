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
}

/// Four bands rather than a full analyser: enough to drive different
/// parameters from different parts of a mix, few enough to stay playable.
pub fn default_bands() -> [Band; BAND_COUNT] {
    [
        // Kick and sub. Narrow on purpose — widening this is the fastest
        // way to make everything pump at once.
        Band { gain: 6.0, ..Band::new(30.0, 110.0) },
        // Bassline and low body.
        Band { gain: 4.0, ..Band::new(110.0, 400.0) },
        // Vocals, snare, most melodic content.
        Band { gain: 5.0, ..Band::new(400.0, 2000.0) },
        // Hats and air. Quiet in most mixes, so it gets more gain.
        Band { gain: 10.0, ..Band::new(2000.0, 12000.0) },
    ]
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
}
