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
pub mod deck;
pub mod library;
pub mod perform;
pub mod preset;
pub mod project;
pub mod ranges;
pub mod scene;
pub mod sets;
pub mod shapes;

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
    /// The parameter this LFO belongs to, or `None` for one in the shared
    /// rack.
    ///
    /// An owned LFO is not a new kind of thing — it is an ordinary LFO
    /// with exactly one route, presented on the parameter's own row
    /// instead of in the rack. That is deliberate: making it a separate
    /// concept would have meant a second evaluation path, a second thing
    /// for a patch to serialise and a second answer to "what is moving
    /// this", all to express something the existing model already says.
    ///
    /// What the field buys is the *rack* staying a rack. Without it,
    /// giving forty parameters their own modulator would put forty LFOs
    /// in a list meant for the handful you route by hand.
    #[serde(default)]
    pub owner: Option<String>,
}

const fn seed_default() -> u32 {
    0x9E37_79B9
}

impl Default for Lfo {
    fn default() -> Self {
        Self {
            shape: Shape::Sine,
            rate: Rate::Beats(4.0),
            phase: 0.0,
            hold: 0.0,
            seed: seed_default(),
            owner: None,
        }
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
    /// Negative inverts. Ignored when [`Route::span`] is set.
    pub depth: f32,
    /// An explicit low and high, as fractions of the parameter's range,
    /// that the source is mapped between.
    ///
    /// `None` is the original behaviour and the default: the source is an
    /// *offset* riding on top of whatever the fader says, so moving the
    /// fader moves the whole swing with it. That is the right model for
    /// "wobble this a bit" and the wrong one for "this should travel
    /// between a half and three quarters", which is a statement about
    /// where the value goes rather than about how far it strays.
    ///
    /// With a span the route delivers whatever offset lands the value
    /// inside it, so the endpoints are what you asked for and the fader
    /// no longer moves the result. Opt-in per route, because that is a
    /// real trade rather than an improvement: the offset model is what
    /// keeps a modulated fader still worth touching.
    #[serde(default)]
    pub span: Option<[f32; 2]>,
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
            match route.span {
                None => self.offsets[id.index()] += value * route.depth,
                Some([low, high]) => {
                    let def = &registry.defs()[id.index()];
                    let width = def.max - def.min;
                    if width.abs() < f32::EPSILON {
                        continue;
                    }
                    // Where the fader is, as a fraction of the range —
                    // because what is delivered is still an offset, and
                    // the offset that lands on `want` depends on where it
                    // is starting from.
                    let base = (registry.target(id) - def.min) / width;
                    // Sources differ in what they swing across: an LFO is
                    // bipolar and an audio band is not. Both have to
                    // arrive as 0..1 or the span's endpoints would mean
                    // different things depending on what is driving it.
                    let unit = if route.source.is_bipolar() {
                        (value + 1.0) * 0.5
                    } else {
                        value
                    };
                    let want = low + (high - low) * unit;
                    self.offsets[id.index()] += want - base;
                }
            }
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
        self.routes.push(Route { source, param: param.into(), depth, enabled: true, span: None });
    }

    /// Whether this exact source already drives this parameter.
    /// The LFO this parameter owns, if it has one.
    pub fn own_modulator(&self, param: &str) -> Option<usize> {
        self.lfos
            .iter()
            .position(|l| l.owner.as_deref() == Some(param))
    }

    /// The rack: the LFOs anybody can route to, with their indices.
    pub fn shared_lfos(&self) -> impl Iterator<Item = (usize, &Lfo)> {
        self.lfos
            .iter()
            .enumerate()
            .filter(|(_, l)| l.owner.is_none())
    }

    /// Give this parameter an LFO of its own, and route it here.
    ///
    /// Returns the LFO's index. Idempotent: a parameter that already has
    /// one keeps it, so the button that calls this can be pressed twice
    /// without collecting a second.
    pub fn attach_modulator(&mut self, param: &str, depth: f32) -> usize {
        if let Some(i) = self.own_modulator(param) {
            return i;
        }
        // A gentle default that visibly does something: a slow sine is
        // the shape somebody reaches for first, and an inaudible depth
        // would read as the button not working.
        self.lfos.push(Lfo {
            shape: Shape::Sine,
            rate: Rate::Beats(4.0),
            owner: Some(param.to_string()),
            ..Default::default()
        });
        let i = self.lfos.len() - 1;
        self.add_route(Source::Lfo(i), param, depth);
        i
    }

    /// Take away this parameter's own LFO, and the route from it.
    ///
    /// Removing an LFO shifts every index above it, and routes name their
    /// source *by* index — so this renumbers them. Getting that wrong
    /// does not fail: it silently re-points somebody else's route at a
    /// different LFO, which is a parameter that starts moving to a shape
    /// nobody chose.
    pub fn detach_modulator(&mut self, param: &str) {
        let Some(i) = self.own_modulator(param) else {
            return;
        };
        self.routes.retain(|r| r.source != Source::Lfo(i));
        self.lfos.remove(i);
        for route in &mut self.routes {
            if let Source::Lfo(n) = route.source
                && n > i
            {
                route.source = Source::Lfo(n - 1);
            }
        }
    }

    pub fn has_route(&self, source: Source, param: &str) -> bool {
        self.routes.iter().any(|r| r.source == source && r.param == param)
    }

    /// Route this source to this parameter, or unroute it if it is already
    /// routed. Returns whether it is routed afterwards.
    ///
    /// The panel's `mod` button reads as a toggle, so it has to be one.
    /// Before this it called `add_route` unconditionally: every press
    /// stacked another identical route, each adding its own depth, so the
    /// modulation deepened with each click and nothing in the panel could
    /// undo it.
    ///
    /// Unrouting removes *every* matching route rather than the first, so
    /// a parameter that already accumulated duplicates is cleaned up by
    /// one press rather than needing one press per hidden duplicate.
    pub fn toggle_route(&mut self, source: Source, param: &str, depth: f32) -> bool {
        if self.has_route(source, param) {
            self.routes
                .retain(|r| !(r.source == source && r.param == param));
            false
        } else {
            self.routes.push(Route {
                source,
                param: param.to_string(),
                depth,
                enabled: true,
            span: None,
            });
            true
        }
    }

    /// Total modulation currently applied to a parameter, for showing the
    /// modulated position on its slider.
    /// Whether anything is driving this parameter — an enabled route or a
    /// graph sink pointed at it.
    ///
    /// The panel marks these, because a slider that will not stay where
    /// you put it is otherwise indistinguishable from a broken one. The
    /// value still belongs to you; modulation is an offset on top.
    pub fn drives(&self, addr: &str) -> bool {
        self.routes
            .iter()
            .any(|r| r.enabled && r.param == addr)
            || self.graph.nodes.iter().any(|n| {
                !n.bypass && matches!(&n.kind, graph::NodeKind::Param { addr: a, .. } if a == addr)
            })
    }

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

    /// A span puts the value where it says, whatever the fader says.
    ///
    /// That is the whole difference from depth, and the reason it is a
    /// separate mode rather than a better one: depth rides on top of the
    /// fader, a span replaces what the fader was doing.
    #[test]
    fn a_span_lands_the_value_on_its_endpoints() {
        let reg = registry();
        let id = reg.id("/a").expect("/a");
        // `/a` is 0..10. Ask for a swing between 2 and 8.
        let mut engine = ModEngine::with_defaults();
        engine.attach_modulator("/a", 1.0);
        let lfo = engine.own_modulator("/a").expect("owned");
        if let Some(r) = engine.routes.iter_mut().find(|r| r.param == "/a") {
            r.span = Some([0.2, 0.8]);
        }

        let at = |engine: &mut ModEngine, phase: f32, base: f32| {
            reg.set(id, base);
            engine.lfos[lfo].phase = phase;
            let offsets = engine.tick(0.0, &reg, AudioLevels::default());
            // Offsets are normalised; put it back in the parameter's own
            // units so the assertion reads in the terms it was asked in.
            base + offsets[id.index()] * 10.0
        };

        // A sine peaks a quarter of the way in and troughs three quarters.
        assert!((at(&mut engine, 0.25, 5.0) - 8.0).abs() < 0.01);
        assert!((at(&mut engine, 0.75, 5.0) - 2.0).abs() < 0.01);
        // And the same endpoints from a different fader position, which
        // is exactly what depth cannot do.
        assert!((at(&mut engine, 0.25, 1.0) - 8.0).abs() < 0.01);
        assert!((at(&mut engine, 0.75, 9.0) - 2.0).abs() < 0.01);
    }

    /// Without a span, the fader still moves the swing with it — the
    /// behaviour every existing patch was written against.
    #[test]
    fn depth_still_rides_on_top_of_the_fader() {
        let reg = registry();
        let id = reg.id("/a").expect("/a");
        let mut engine = ModEngine::with_defaults();
        engine.attach_modulator("/a", 0.2);
        let lfo = engine.own_modulator("/a").expect("owned");
        assert!(engine.routes.iter().all(|r| r.span.is_none()));

        let at = |engine: &mut ModEngine, base: f32| {
            reg.set(id, base);
            engine.lfos[lfo].phase = 0.25;
            let offsets = engine.tick(0.0, &reg, AudioLevels::default());
            base + offsets[id.index()] * 10.0
        };
        // A fifth of a 0..10 range, at the peak: two above wherever it is.
        assert!((at(&mut engine, 5.0) - 7.0).abs() < 0.01);
        assert!((at(&mut engine, 1.0) - 3.0).abs() < 0.01);
    }

    /// An audio band is unipolar and an LFO is not, so both have to be
    /// mapped into the span the same way or the endpoints would mean
    /// different things depending on what was driving them.
    #[test]
    fn a_unipolar_source_reaches_both_ends_of_a_span() {
        let reg = registry();
        let id = reg.id("/a").expect("/a");
        let mut engine = ModEngine::with_defaults();
        engine.add_route(Source::Audio(0), "/a", 1.0);
        if let Some(r) = engine.routes.iter_mut().find(|r| r.param == "/a") {
            r.span = Some([0.2, 0.8]);
        }
        reg.set(id, 5.0);
        let quiet = engine.tick(0.0, &reg, AudioLevels { bands: &[0.0], level: 0.0 });
        assert!((5.0 + quiet[id.index()] * 10.0 - 2.0).abs() < 0.01, "silence is not the low end");
        reg.set(id, 5.0);
        let loud = engine.tick(0.0, &reg, AudioLevels { bands: &[1.0], level: 0.0 });
        assert!((5.0 + loud[id.index()] * 10.0 - 8.0).abs() < 0.01, "full scale is not the high end");
    }

    /// A parameter's own modulator is an ordinary LFO with one route.
    #[test]
    fn a_parameter_can_own_its_modulator() {
        let mut engine = ModEngine::with_defaults();
        let shared = engine.lfos.len();
        let i = engine.attach_modulator("/a", 0.5);
        assert_eq!(i, shared, "the owned LFO went somewhere unexpected");
        assert_eq!(engine.own_modulator("/a"), Some(i));
        assert!(engine.has_route(Source::Lfo(i), "/a"));
        // And it is not in the rack, or forty of these would fill it.
        assert_eq!(engine.shared_lfos().count(), shared);

        // Pressing the button again keeps the one it has.
        assert_eq!(engine.attach_modulator("/a", 0.9), i);
        assert_eq!(engine.lfos.len(), shared + 1);
    }

    /// Removing an LFO renumbers every route above it.
    ///
    /// This is the one that fails silently: routes name their source by
    /// index, so a missed renumber does not error — it points somebody
    /// else's route at a different LFO, and a parameter starts moving to
    /// a shape nobody chose.
    #[test]
    fn detaching_renumbers_the_routes_above_it() {
        let mut engine = ModEngine::with_defaults();
        engine.lfos.truncate(2);
        engine.routes.clear();
        // Rack routes either side of the one about to go.
        engine.add_route(Source::Lfo(0), "/first", 0.1);
        engine.add_route(Source::Lfo(1), "/second", 0.2);
        let owned = engine.attach_modulator("/mine", 0.3);
        assert_eq!(owned, 2);
        // Another owned one above it, which is the case that moves.
        let above = engine.attach_modulator("/later", 0.4);
        assert_eq!(above, 3);

        engine.detach_modulator("/mine");

        assert_eq!(engine.own_modulator("/mine"), None);
        assert!(!engine.routes.iter().any(|r| r.param == "/mine"));
        // The rack routes are untouched…
        assert!(engine.has_route(Source::Lfo(0), "/first"));
        assert!(engine.has_route(Source::Lfo(1), "/second"));
        // …and the one that was above the hole moved down with its LFO,
        // rather than being left pointing at whatever now sits there.
        let moved = engine.own_modulator("/later").expect("the later modulator");
        assert_eq!(moved, 2);
        assert!(
            engine.has_route(Source::Lfo(moved), "/later"),
            "the route did not follow its LFO: {:?}",
            engine.routes
        );
    }

    /// An owned modulator moves its parameter like any other route, so
    /// nothing about evaluation is special-cased.
    #[test]
    fn an_owned_modulator_actually_moves_the_parameter() {
        let reg = registry();
        let mut engine = ModEngine::with_defaults();
        engine.attach_modulator("/a", 1.0);
        // A quarter of a cycle in, a sine is at its peak.
        let lfo = engine.own_modulator("/a").expect("owned");
        engine.lfos[lfo].phase = 0.25;
        let offsets = engine.tick(0.0, &reg, AudioLevels::default());
        let id = reg.id("/a").expect("/a");
        assert!(
            offsets[id.index()] > 0.5,
            "an owned modulator produced {} — it is not being evaluated",
            offsets[id.index()]
        );
    }

    /// A patch written before parameters could own modulators still
    /// loads, and everything in it is shared.
    #[test]
    fn a_patch_without_owners_loads_as_a_rack() {
        let engine = ModEngine::with_defaults();
        let json = serde_json::to_string(&engine).expect("serialise");
        // Strip the field the way a file written before it existed would.
        let older = json.replace(",\"owner\":null", "");
        let back: ModEngine = serde_json::from_str(&older).expect("an older patch must still load");
        assert_eq!(back.lfos.len(), engine.lfos.len());
        assert!(back.lfos.iter().all(|l| l.owner.is_none()));
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
