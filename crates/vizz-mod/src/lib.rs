//! Modulation: LFOs and a beat clock that make parameters move on their own.
//!
//! Modulation is applied as an **offset on top of** the base value, never
//! by writing into the parameter store. That is the difference between an
//! instrument and a toy: a fader you set stays where you set it, and the
//! LFO rides on top of it. Turning modulation off leaves your value intact.
//!
//! Offsets are in *normalised* units — a depth of 0.5 swings a parameter
//! across half its range, whatever that range happens to be — so a route
//! keeps its musical meaning if a parameter's bounds change.
//!
//! Everything here is pure arithmetic driven by `dt`, so the whole layer
//! is testable without a GPU, a clock, or a controller.

pub mod graph;

/// Serialises tests that redirect `XDG_CONFIG_HOME`.
///
/// `set_var` is unsafe precisely because it mutates process-global state:
/// two suites doing it under the default parallel test runner see each
/// other's directory and fail intermittently, which is exactly what
/// happened when the macros tests were added next to the patch tests.
#[cfg(test)]
pub(crate) mod test_env {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    /// Redirect config storage at a private directory for the duration of
    /// the returned guard.
    ///
    /// **Every test that reads a config path must take this**, not only
    /// the ones that write. A read-only test skipping it still calls
    /// `patch_dir()`, and a guarded test on another thread will change
    /// what that returns mid-test — which is a false failure that shows
    /// up in about one CI run in eight, on whichever machine is slowest.
    pub fn scoped(tag: &str) -> (MutexGuard<'static, ()>, std::path::PathBuf) {
        let guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("vizz-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the mutex makes this the only thread touching the
        // environment for as long as the guard is held.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        (guard, dir)
    }
}
pub mod library;
pub mod perform;

use serde::{Deserialize, Serialize};
use vizz_params::ParamRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Sine,
    Triangle,
    /// Ramps up, then jumps back — classic sweep.
    Saw,
    Square,
    /// Holds a new random value each cycle. The stepped, unpredictable
    /// one; the others are all continuous.
    SampleHold,
}

impl Shape {
    pub const ALL: [Shape; 5] =
        [Shape::Sine, Shape::Triangle, Shape::Saw, Shape::Square, Shape::SampleHold];

    pub fn label(&self) -> &'static str {
        match self {
            Shape::Sine => "sine",
            Shape::Triangle => "tri",
            Shape::Saw => "saw",
            Shape::Square => "square",
            Shape::SampleHold => "s&h",
        }
    }
}

/// How fast an LFO runs.
// Externally tagged (`{"hz": 2.0}`): serde cannot internally-tag a
// newtype variant holding a float.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rate {
    /// Free-running, in cycles per second.
    Hz(f32),
    /// Locked to the beat clock: one cycle every N beats. 4.0 is a bar in
    /// 4/4; 0.25 is a sixteenth.
    Beats(f32),
}

impl Rate {
    pub fn label(&self) -> String {
        match self {
            Rate::Hz(hz) => format!("{hz:.2} Hz"),
            Rate::Beats(b) => format!("{b:.2} beats"),
        }
    }
}

/// Musical time. Beat-synced LFOs advance from this rather than from
/// wall-clock seconds, so they stay locked to each other and to the music.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatClock {
    pub bpm: f32,
    pub running: bool,
    /// Position in beats since the last reset.
    #[serde(skip)]
    pub beats: f64,
}

impl Default for BeatClock {
    fn default() -> Self {
        Self { bpm: 120.0, running: true, beats: 0.0 }
    }
}

impl BeatClock {
    /// Advance and return how many beats elapsed.
    pub fn tick(&mut self, dt: f32) -> f64 {
        if !self.running {
            return 0.0;
        }
        let delta = dt as f64 * (self.bpm.max(0.0) as f64) / 60.0;
        self.beats += delta;
        delta
    }

    /// Restart on the downbeat — the "line it up with the track" button.
    pub fn reset(&mut self) {
        self.beats = 0.0;
    }

    /// Position within the current bar, 0..1, for a visual beat indicator.
    pub fn bar_phase(&self, beats_per_bar: f64) -> f32 {
        if beats_per_bar <= 0.0 {
            return 0.0;
        }
        (self.beats.rem_euclid(beats_per_bar) / beats_per_bar) as f32
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lfo {
    pub shape: Shape,
    pub rate: Rate,
    /// 0..1 within the cycle. Not serialised: a saved patch should start
    /// from a known phase rather than resume mid-swing.
    #[serde(skip)]
    pub phase: f32,
    /// Current sample-and-hold value, and the cycle it was drawn in.
    #[serde(skip)]
    hold: f32,
    /// xorshift state. Skipped by serde like the rest of the live state,
    /// but it needs a non-zero default: xorshift on 0 stays 0 forever, so
    /// a reloaded sample-and-hold LFO would emit a constant.
    #[serde(skip, default = "seed_default")]
    seed: u32,
}

const fn seed_default() -> u32 {
    0x9E37_79B9
}

impl Default for Lfo {
    fn default() -> Self {
        Self { shape: Shape::Sine, rate: Rate::Beats(4.0), phase: 0.0, hold: 0.0, seed: seed_default() }
    }
}

impl Lfo {
    /// Advance by `dt` seconds / `beat_delta` beats and return -1..=1.
    pub fn tick(&mut self, dt: f32, beat_delta: f64) -> f32 {
        let advance = match self.rate {
            Rate::Hz(hz) => dt * hz.max(0.0),
            Rate::Beats(beats) if beats > 0.0 => (beat_delta / beats as f64) as f32,
            Rate::Beats(_) => 0.0,
        };
        let previous = self.phase;
        self.phase = (self.phase + advance).rem_euclid(1.0);
        // Wrapping means a new cycle began; sample-and-hold picks a new
        // value exactly then.
        if self.shape == Shape::SampleHold && (self.phase < previous || previous == 0.0) {
            self.hold = self.next_random();
        }
        self.value()
    }

    /// The current output without advancing.
    pub fn value(&self) -> f32 {
        let p = self.phase;
        match self.shape {
            Shape::Sine => (p * std::f32::consts::TAU).sin(),
            // Up for the first half, down for the second.
            Shape::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
            Shape::Saw => 2.0 * p - 1.0,
            Shape::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Shape::SampleHold => self.hold,
        }
    }

    /// xorshift: deterministic and dependency-free, which keeps the whole
    /// crate reproducible in tests.
    fn next_random(&mut self) -> f32 {
        // Belt and braces against a zero state reaching here from an older
        // patch: xorshift cannot escape 0.
        if self.seed == 0 {
            self.seed = seed_default();
        }
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Where a route's value comes from.
///
/// Externally tagged for the same reason `Rate` is. Generalising this out
/// of a bare LFO index is what lets audio drive parameters, and it is the
/// shape a node graph needs later — a node's input is a `Source`, not an
/// oscillator.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Index into `ModEngine::lfos`. Bipolar, -1..1.
    Lfo(usize),
    /// Audio band envelope. Unipolar, 0..1 — a kick pushes a parameter up
    /// from where it is set rather than swinging it either side, which is
    /// what you want from a transient.
    Audio(usize),
    /// Broadband RMS. Unipolar.
    Level,
}

impl Source {
    pub fn label(&self) -> String {
        match self {
            Source::Lfo(i) => format!("LFO {}", i + 1),
            Source::Audio(i) => format!("Band {}", i + 1),
            Source::Level => "Level".into(),
        }
    }

    /// Unipolar sources only ever add; bipolar ones swing both ways. The
    /// UI uses this to draw the right range on a depth control.
    pub fn is_bipolar(&self) -> bool {
        matches!(self, Source::Lfo(_))
    }
}

/// Live audio values for one tick. Passed in rather than pulled, so this
/// crate stays independent of whether audio capture exists at all.
#[derive(Debug, Clone, Copy)]
pub struct AudioLevels<'a> {
    pub bands: &'a [f32],
    pub level: f32,
}

impl Default for AudioLevels<'_> {
    fn default() -> Self {
        Self { bands: &[], level: 0.0 }
    }
}

/// One source driving one parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub source: Source,
    /// OSC-style parameter address.
    pub param: String,
    /// Fraction of the parameter's range the LFO swings it across.
    /// Negative inverts.
    pub depth: f32,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// The modulation layer: sources, routes, and the per-parameter offsets
/// they produce each frame.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModEngine {
    pub clock: BeatClock,
    pub lfos: Vec<Lfo>,
    pub routes: Vec<Route>,
    /// The node graph. Evaluated alongside the flat routes rather than
    /// replacing them, so the existing route list keeps working while the
    /// canvas is built; their offsets sum, as several routes onto one
    /// parameter already do.
    #[serde(default)]
    pub graph: graph::NodeGraph,
    #[serde(skip)]
    graph_offsets: Vec<f32>,
    /// Scratch offsets indexed by parameter, rebuilt every tick.
    #[serde(skip)]
    offsets: Vec<f32>,
}

impl ModEngine {
    /// A useful starting point rather than an empty rack: two beat-synced
    /// LFOs, unrouted, so the first thing a user does is pick a target.
    pub fn with_defaults() -> Self {
        Self {
            clock: BeatClock::default(),
            lfos: vec![
                Lfo { shape: Shape::Sine, rate: Rate::Beats(4.0), ..Default::default() },
                Lfo { shape: Shape::Triangle, rate: Rate::Beats(1.0), ..Default::default() },
            ],
            routes: Vec::new(),
            graph: graph::NodeGraph::default(),
            graph_offsets: Vec::new(),
            offsets: Vec::new(),
        }
    }

    /// Advance every source and accumulate routed offsets.
    ///
    /// The returned slice is indexed by parameter position and is in
    /// normalised units, ready for `ParamSnapshot::advance_modulated`.
    pub fn tick(
        &mut self,
        dt: f32,
        registry: &ParamRegistry,
        audio: AudioLevels<'_>,
    ) -> &[f32] {
        self.offsets.clear();
        self.offsets.resize(registry.len(), 0.0);

        let beat_delta = self.clock.tick(dt);
        // Sources advance even when nothing is routed, so enabling a route
        // mid-set drops in at the phase the LFO has been keeping rather
        // than restarting from zero.
        let values: Vec<f32> = self.lfos.iter_mut().map(|l| l.tick(dt, beat_delta)).collect();

        for route in &self.routes {
            if !route.enabled {
                continue;
            }
            let value = match route.source {
                Source::Lfo(i) => match values.get(i) {
                    Some(&v) => v,
                    None => continue,
                },
                // A missing band reads as silence rather than skipping the
                // route, so unplugging the audio device parks modulated
                // parameters at their set value instead of freezing them
                // wherever the last sample left them.
                Source::Audio(i) => audio.bands.get(i).copied().unwrap_or(0.0),
                Source::Level => audio.level,
            };
            let Some(id) = registry.id(&route.param) else { continue };
            // Several routes may target one parameter; they sum, and the
            // snapshot clamps the total.
            self.offsets[id.index()] += value * route.depth;
        }

        self.graph.tick(
            dt,
            beat_delta,
            self.clock.beats,
            audio,
            registry,
            &mut self.graph_offsets,
        );
        for (o, g) in self.offsets.iter_mut().zip(&self.graph_offsets) {
            *o += g;
        }
        &self.offsets
    }

    /// Current offsets without advancing (for the UI between frames).
    pub fn offsets(&self) -> &[f32] {
        &self.offsets
    }

    pub fn add_route(&mut self, source: Source, param: impl Into<String>, depth: f32) {
        self.routes.push(Route { source, param: param.into(), depth, enabled: true });
    }

    /// Total modulation currently applied to a parameter, for showing the
    /// modulated position on its slider.
    pub fn offset_for(&self, registry: &ParamRegistry, param: &str) -> f32 {
        registry
            .id(param)
            .and_then(|id| self.offsets.get(id.index()))
            .copied()
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizz_params::{ParamDef, ParamSnapshot};

    fn registry() -> ParamRegistry {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/a", 0.0, 10.0, 5.0));
        b.add(ParamDef::new("/b", -1.0, 1.0, 0.0));
        b.build()
    }

    #[test]
    fn shapes_have_the_expected_extremes() {
        let mut lfo = Lfo { shape: Shape::Sine, rate: Rate::Hz(1.0), ..Default::default() };
        assert!(lfo.value().abs() < 1e-6, "sine starts at zero");
        lfo.phase = 0.25;
        assert!((lfo.value() - 1.0).abs() < 1e-5, "sine peaks at a quarter cycle");

        let mut tri = Lfo { shape: Shape::Triangle, rate: Rate::Hz(1.0), ..Default::default() };
        tri.phase = 0.5;
        assert!((tri.value() - 1.0).abs() < 1e-5, "triangle peaks mid-cycle");
        tri.phase = 0.0;
        assert!((tri.value() + 1.0).abs() < 1e-5, "triangle starts at its trough");

        let mut saw = Lfo { shape: Shape::Saw, rate: Rate::Hz(1.0), ..Default::default() };
        saw.phase = 0.0;
        assert_eq!(saw.value(), -1.0);
        saw.phase = 1.0 - f32::EPSILON;
        assert!(saw.value() > 0.99);

        let mut sq = Lfo { shape: Shape::Square, rate: Rate::Hz(1.0), ..Default::default() };
        sq.phase = 0.25;
        assert_eq!(sq.value(), 1.0);
        sq.phase = 0.75;
        assert_eq!(sq.value(), -1.0);
    }

    #[test]
    fn every_shape_stays_within_range() {
        for shape in Shape::ALL {
            let mut lfo = Lfo { shape, rate: Rate::Hz(3.3), ..Default::default() };
            for _ in 0..2000 {
                let v = lfo.tick(1.0 / 60.0, 0.0);
                assert!((-1.0..=1.0).contains(&v), "{shape:?} produced {v}");
            }
        }
    }

    #[test]
    fn hz_rate_completes_a_cycle_per_second() {
        let mut lfo = Lfo { shape: Shape::Saw, rate: Rate::Hz(1.0), ..Default::default() };
        for _ in 0..60 {
            lfo.tick(1.0 / 60.0, 0.0);
        }
        // One second at 1 Hz lands back at the start.
        assert!(lfo.phase < 0.02 || lfo.phase > 0.98, "phase {}", lfo.phase);
    }

    #[test]
    fn beat_synced_lfos_ignore_wall_clock_and_follow_tempo() {
        let mut lfo = Lfo { shape: Shape::Saw, rate: Rate::Beats(4.0), ..Default::default() };
        // dt is irrelevant for a beat-synced source; only beats matter.
        lfo.tick(999.0, 0.0);
        assert_eq!(lfo.phase, 0.0, "no beats elapsed means no movement");
        lfo.tick(0.0, 2.0);
        assert!((lfo.phase - 0.5).abs() < 1e-6, "two of four beats is half a cycle");
    }

    #[test]
    fn clock_tempo_controls_beat_rate_and_stopping_freezes_it() {
        let mut clock = BeatClock { bpm: 120.0, running: true, beats: 0.0 };
        // 120 bpm = 2 beats per second.
        let delta = clock.tick(1.0);
        assert!((delta - 2.0).abs() < 1e-9, "got {delta}");

        clock.running = false;
        assert_eq!(clock.tick(1.0), 0.0);
        assert!((clock.beats - 2.0).abs() < 1e-9, "stopped clock must not drift");

        clock.reset();
        assert_eq!(clock.beats, 0.0);
    }

    #[test]
    fn sample_and_hold_steps_once_per_cycle() {
        let mut lfo = Lfo { shape: Shape::SampleHold, rate: Rate::Hz(1.0), ..Default::default() };
        lfo.tick(0.1, 0.0);
        let held = lfo.value();
        // Mid-cycle it must not move.
        for _ in 0..5 {
            lfo.tick(0.1, 0.0);
            assert_eq!(lfo.value(), held, "s&h changed mid-cycle");
        }
        // Crossing the cycle boundary draws a new value.
        lfo.tick(0.6, 0.0);
        assert_ne!(lfo.value(), held, "s&h did not step at the cycle boundary");
    }

    /// Graph and flat routes must both reach the parameter and sum, since
    /// the two coexist while the canvas is being built. If one silently
    /// won, half a patch would stop working with no error anywhere.
    #[test]
    fn graph_and_flat_routes_sum_into_the_same_parameter() {
        use graph::NodeKind;
        let reg = registry();
        let mut engine = ModEngine { lfos: vec![Lfo::default()], ..Default::default() };
        engine.lfos[0].shape = Shape::Square;
        engine.lfos[0].rate = Rate::Hz(0.0);
        engine.lfos[0].phase = 0.25; // parked at +1
        engine.add_route(Source::Lfo(0), "/a", 0.25);

        let src = engine.graph.add(NodeKind::Constant(0.5), [0.0, 0.0]);
        let sink = engine.graph.add(NodeKind::Param { addr: "/a".into(), depth: 0.5 }, [1.0, 0.0]);
        engine.graph.connect(src, sink, 0);

        let offsets = engine.tick(1.0 / 60.0, &reg, AudioLevels::default());
        let a = reg.id("/a").unwrap().index();
        // 0.25 from the route plus 0.25 from the graph.
        assert!((offsets[a] - 0.5).abs() < 1e-5, "got {}", offsets[a]);
    }

    /// A patch reloaded from disk must still produce varying sample-and-hold
    /// values. The seed is skipped by serde, and xorshift started from zero
    /// stays at zero forever, so without a non-zero default a reloaded S&H
    /// LFO emits a constant — silently, and only for that one shape.
    #[test]
    fn deserialised_sample_hold_still_varies() {
        let lfo = Lfo { shape: Shape::SampleHold, rate: Rate::Hz(30.0), ..Default::default() };
        let mut back: Lfo = serde_json::from_str(&serde_json::to_string(&lfo).unwrap()).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..40 {
            seen.insert(back.tick(1.0 / 60.0, 0.0).to_bits());
        }
        assert!(seen.len() > 3, "sample-and-hold froze after a reload: {seen:?}");
    }

    #[test]
    fn routes_produce_normalised_offsets_and_sum() {
        let reg = registry();
        let mut engine = ModEngine { lfos: vec![Lfo::default(), Lfo::default()], ..Default::default() };
        // Park both LFOs at full positive output.
        for lfo in &mut engine.lfos {
            lfo.shape = Shape::Square;
            lfo.rate = Rate::Hz(0.0);
            lfo.phase = 0.25;
        }
        engine.add_route(Source::Lfo(0), "/a", 0.25);
        engine.add_route(Source::Lfo(1), "/a", 0.25);

        let offsets = engine.tick(1.0 / 60.0, &reg, AudioLevels::default());
        let a = reg.id("/a").unwrap().index();
        assert!((offsets[a] - 0.5).abs() < 1e-5, "two routes should sum: {}", offsets[a]);
        let b = reg.id("/b").unwrap().index();
        assert_eq!(offsets[b], 0.0, "unrouted parameters stay untouched");
    }

    #[test]
    fn disabled_routes_and_unknown_targets_are_inert() {
        let reg = registry();
        let mut engine = ModEngine { lfos: vec![Lfo::default()], ..Default::default() };
        engine.lfos[0].shape = Shape::Square;
        engine.lfos[0].rate = Rate::Hz(0.0);
        engine.lfos[0].phase = 0.25;

        engine.add_route(Source::Lfo(0), "/a", 1.0);
        engine.routes[0].enabled = false;
        // A patch naming a parameter this build no longer has must not
        // panic or shift the wrong slider.
        engine.add_route(Source::Lfo(0), "/gone", 1.0);
        // A route pointing at a missing LFO likewise.
        engine.add_route(Source::Lfo(99), "/b", 1.0);

        let offsets = engine.tick(1.0 / 60.0, &reg, AudioLevels::default());
        assert!(offsets.iter().all(|&o| o == 0.0), "got {offsets:?}");
    }

    /// The core promise: modulation rides on top of the base value and
    /// leaves it intact.
    #[test]
    fn modulation_offsets_the_base_without_overwriting_it() {
        let reg = registry();
        let id = reg.id("/a").unwrap();
        reg.set(id, 5.0); // user parks the fader mid-range
        let mut snap = ParamSnapshot::new(&reg);

        let mut engine = ModEngine { lfos: vec![Lfo::default()], ..Default::default() };
        engine.lfos[0].shape = Shape::Square;
        engine.lfos[0].rate = Rate::Hz(0.0);
        engine.lfos[0].phase = 0.25; // pinned at +1
        engine.add_route(Source::Lfo(0), "/a", 0.25); // quarter of a 0..10 range = +2.5

        let offsets = engine.tick(1.0 / 60.0, &reg, AudioLevels::default()).to_vec();
        snap.advance_modulated(&reg, 1.0, &offsets);
        assert!((snap.get(id) - 7.5).abs() < 0.01, "got {}", snap.get(id));

        // The stored target is untouched: removing modulation restores it.
        assert_eq!(reg.target(id), 5.0);
        snap.advance_modulated(&reg, 1.0, &[]);
        assert!((snap.get(id) - 5.0).abs() < 0.01, "got {}", snap.get(id));
    }

    #[test]
    fn modulation_cannot_push_a_parameter_out_of_range() {
        let reg = registry();
        let id = reg.id("/a").unwrap();
        reg.set(id, 9.0);
        let mut snap = ParamSnapshot::new(&reg);

        let mut engine = ModEngine { lfos: vec![Lfo::default()], ..Default::default() };
        engine.lfos[0].shape = Shape::Square;
        engine.lfos[0].rate = Rate::Hz(0.0);
        engine.lfos[0].phase = 0.25;
        engine.add_route(Source::Lfo(0), "/a", 5.0); // absurd depth on purpose

        let offsets = engine.tick(1.0 / 60.0, &reg, AudioLevels::default()).to_vec();
        snap.advance_modulated(&reg, 1.0, &offsets);
        assert_eq!(snap.get(id), 10.0, "must clamp to the parameter's max");
    }

    #[test]
    fn engine_round_trips_through_json() {
        let mut engine = ModEngine::with_defaults();
        engine.add_route(Source::Lfo(0), "/a", 0.5);
        engine.clock.bpm = 128.0;
        let json = serde_json::to_string(&engine).unwrap();
        let back: ModEngine = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clock.bpm, 128.0);
        assert_eq!(back.routes.len(), 1);
        assert_eq!(back.lfos.len(), 2);
        // Phase is deliberately not persisted: a recalled patch starts
        // from a known position rather than mid-swing.
        assert_eq!(back.lfos[0].phase, 0.0);
    }
}
