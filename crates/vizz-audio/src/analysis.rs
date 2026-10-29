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
    // Written as a negated `>` on purpose, and not the `<=` clippy would
    // rather see: those differ for NaN. `!(NaN > x)` is true, so a NaN peak
    // is rejected here; `NaN <= x` is false, so it would sail through and
    // come out the other side as a NaN gain applied to every band.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(peak > 1e-5) {
        return None;
    }
    Some(linear_to_db(TARGET_PEAK / peak))
}

pub const BAND_COUNT: usize = 4;

/// Bands in the fine spectrum: log-spaced, so each covers the same
/// fraction of an octave — about a ninth — from [`SPECTRUM_LO_HZ`] to
/// [`SPECTRUM_HI_HZ`]. Enough for a picture of the mix's shape, which the
/// four modulation bands are too coarse to give.
pub const SPECTRUM: usize = 64;
pub const SPECTRUM_LO_HZ: f32 = 30.0;
pub const SPECTRUM_HI_HZ: f32 = 16_000.0;

/// What one analysis frame produced.
#[derive(Debug, Clone, Copy)]
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
    /// The fine spectrum, post-gain and post-envelope, 0..1, lowest first.
    ///
    /// No gain control of its own: each band takes the gain and timing of
    /// the modulation band its centre falls in, scaled up for how much
    /// narrower it is, so `fit` and the sensitivity controls set it too
    /// and a flat mix reads about the same here as in the four meters.
    pub spectrum: [f32; SPECTRUM],
}

// Written out because `Default` stops at arrays of 32.
impl Default for Frame {
    fn default() -> Self {
        Self { bands: [0.0; BAND_COUNT], raw: [0.0; BAND_COUNT], level: 0.0, flux: 0.0, spectrum: [0.0; SPECTRUM] }
    }
}

/// Where each fine band's edges fall, `SPECTRUM + 1` of them.
fn spectrum_edges() -> [f32; SPECTRUM + 1] {
    let ratio = SPECTRUM_HI_HZ / SPECTRUM_LO_HZ;
    std::array::from_fn(|j| SPECTRUM_LO_HZ * ratio.powf(j as f32 / SPECTRUM as f32))
}

/// Which modulation band governs the fine band centred at `hz`: the one
/// containing it, or the nearest if it lies outside them all.
fn governing_band(hz: f32, bands: &[Band; BAND_COUNT]) -> usize {
    if let Some(i) = bands.iter().position(|b| hz >= b.lo_hz && hz < b.hi_hz) {
        return i;
    }
    // Nearest by octaves, which is how far apart frequencies sound.
    let off = |b: &Band| {
        let d = if hz < b.lo_hz { b.lo_hz / hz } else { hz / b.hi_hz.max(1.0) };
        d.max(1.0).log2()
    };
    (0..BAND_COUNT)
        .min_by(|&a, &b| off(&bands[a]).total_cmp(&off(&bands[b])))
        .unwrap_or(0)
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
    /// Each fine band's share of the FFT bins: `(bin, weight)` runs,
    /// indexed by `spec_at`.
    spec_bins: Vec<(usize, f32)>,
    spec_at: [usize; SPECTRUM + 1],
    /// Each fine band's centre frequency.
    spec_hz: [f32; SPECTRUM],
    spec_env: [f32; SPECTRUM],
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
        // A fine band takes each FFT bin in proportion to how much of the
        // bin's width it covers. At the bottom a band is narrower than a
        // bin — 3 Hz against 23 — so neighbouring bands share one, and the
        // lowest octave or so is smooth rather than resolved; at 2048
        // points that is all the window can tell apart.
        let sample_rate = sample_rate.max(1.0);
        let hz_per_bin = sample_rate / FFT_SIZE as f32;
        let edges = spectrum_edges();
        let mut spec_bins = Vec::new();
        let mut spec_at = [0; SPECTRUM + 1];
        for j in 0..SPECTRUM {
            spec_at[j] = spec_bins.len();
            let (lo, hi) = (edges[j] / hz_per_bin, edges[j + 1] / hz_per_bin);
            let first = (lo + 0.5).floor().max(1.0) as usize;
            let last = ((hi + 0.5).floor() as usize).min(FFT_SIZE / 2 - 1);
            for k in first..=last {
                let cover = (hi.min(k as f32 + 0.5) - lo.max(k as f32 - 0.5)).max(0.0);
                if cover > 0.0 {
                    spec_bins.push((k, cover));
                }
            }
        }
        spec_at[SPECTRUM] = spec_bins.len();
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
            spec_bins,
            spec_at,
            spec_hz: std::array::from_fn(|j| (edges[j] * edges[j + 1]).sqrt()),
            spec_env: [0.0; SPECTRUM],
            sample_rate,
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

        // How many fine bands each modulation band covers. A fine band
        // holds about 1/n of its band's energy on a mix that is flat per
        // octave, so its reading is scaled by √n to come out level with
        // the band's own.
        let governs: [usize; SPECTRUM] = std::array::from_fn(|j| governing_band(self.spec_hz[j], bands));
        let mut count = [0usize; BAND_COUNT];
        for &g in &governs {
            count[g] += 1;
        }
        for (j, &g) in governs.iter().enumerate() {
            let energy: f32 =
                self.spec_bins[self.spec_at[j]..self.spec_at[j + 1]].iter().map(|&(k, w)| w * self.mags[k] * self.mags[k]).sum();
            let band = &bands[g];
            let target = (energy.sqrt() * (count[g] as f32).sqrt() * band.gain).clamp(0.0, 1.0);
            let tau = if target > self.spec_env[j] { band.attack } else { band.release };
            let k = if tau <= f32::EPSILON { 1.0 } else { 1.0 - (-dt / tau).exp() };
            self.spec_env[j] += (target - self.spec_env[j]) * k;
        }
        out.spectrum = self.spec_env;
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

    /// The fine spectrum's whole job is to say where in the mix the sound
    /// is: a tone has to light the band it falls in, and anything an
    /// octave or more away has to stay dark.
    #[test]
    fn a_tone_lights_its_place_in_the_spectrum() {
        let bands = [Band::new(20.0, 20_000.0); BAND_COUNT];
        let edges = spectrum_edges();
        for freq in [80.0, 250.0, 1000.0, 4000.0, 12_000.0] {
            let mut a = Analyzer::new(SR);
            // Quiet, because all 64 fine bands fall in the one wide band
            // here and each is scaled up eightfold for it.
            let f = settle(&mut a, &sine(freq, 0.03), &bands);
            let at = (0..SPECTRUM).find(|&j| freq >= edges[j] && freq < edges[j + 1]).unwrap();
            let peak = (0..SPECTRUM).max_by(|&a, &b| f.spectrum[a].total_cmp(&f.spectrum[b])).unwrap();
            assert!(peak.abs_diff(at) <= 1, "{freq} Hz peaked in band {peak}, not {at}: {:?}", f.spectrum);
            // Lower at the bottom, where one FFT bin is shared by several
            // fine bands and a tone's light is split between them.
            assert!(f.spectrum[peak] > 0.04, "{freq} Hz barely registered: {}", f.spectrum[peak]);
            assert!(f.spectrum[peak] < 1.0, "{freq} Hz clipped, which hides the leakage being tested");
            for (j, &v) in f.spectrum.iter().enumerate() {
                // An octave away, or outside the window's main lobe of
                // two bins either side, whichever is further: at 80 Hz
                // two bins is more than an octave, and no FFT this long
                // can keep a tone out of its own main lobe.
                let centre = (edges[j] * edges[j + 1]).sqrt();
                let far = (centre / freq).log2().abs() > 1.0 && (centre - freq).abs() > 3.0 * SR / FFT_SIZE as f32;
                if far {
                    assert!(v < 0.1 * f.spectrum[peak], "{freq} Hz leaked into band {j} ({v})");
                }
            }
        }
    }

    /// The spectrum has no sensitivity of its own: it follows the bands',
    /// so turning a band up turns its stretch of the spectrum up, and
    /// leaves the rest alone.
    #[test]
    fn the_spectrum_takes_its_gain_from_the_bands() {
        let sig: Vec<f32> = sine(200.0, 0.05).iter().zip(sine(6000.0, 0.05)).map(|(a, b)| a + b).collect();
        let flat = default_bands().map(|b| Band { gain: 1.0, ..b });
        let mut loud = flat;
        loud[1].gain = 4.0;
        let before = settle(&mut Analyzer::new(SR), &sig, &flat).spectrum;
        let after = settle(&mut Analyzer::new(SR), &sig, &loud).spectrum;
        let edges = spectrum_edges();
        let at = |hz: f32| (0..SPECTRUM).find(|&j| hz >= edges[j] && hz < edges[j + 1]).unwrap();
        let (bass, hats) = (at(200.0), at(6000.0));
        assert!((after[bass] / before[bass] - 4.0).abs() < 0.05, "{} -> {}", before[bass], after[bass]);
        assert!((after[hats] - before[hats]).abs() < 1e-4, "the hats moved: {} -> {}", before[hats], after[hats]);
    }

    #[test]
    fn silence_produces_nothing() {
        let mut a = Analyzer::new(SR);
        let bands = default_bands();
        let f = settle(&mut a, &vec![0.0; FFT_SIZE], &bands);
        assert!(f.level < 1e-6, "level {}", f.level);
        assert!(f.bands.iter().all(|&b| b < 1e-6), "{:?}", f.bands);
        assert!(f.flux < 1e-6, "flux {}", f.flux);
        assert!(f.spectrum.iter().all(|&b| b < 1e-6), "{:?}", f.spectrum);
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
