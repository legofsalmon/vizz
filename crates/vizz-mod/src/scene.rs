//! The scene grid: sixteen looks in a row, and a blend between them.
//!
//! A preset recalled by number is a cut. This is the other thing you want
//! during a set — a row of looks you can walk along, where moving from one
//! to the next takes musical time rather than a frame.
//!
//! Sixteen wide because that is how a step sequencer is laid out, and a VJ
//! standing behind a sixteen-pad controller should be able to map one row
//! of pads to one row of scenes without arithmetic.
//!
//! # Blending in the data, not in the picture
//!
//! The obvious way to cross between two looks is to render both and
//! dissolve the textures. That is not what this does, and the difference
//! is the whole point. Compositing two pictures of a particle field gives
//! you two particle fields at half opacity — a double image that reads as
//! a mistake. Blending the *data* gives you one field whose parameters and
//! whose geometry are somewhere between the two: the particles are still
//! particles, there is still one of everything, and the transition looks
//! like the material moving rather than like a mixer.
//!
//! So a transition interpolates parameter values, and hands the point
//! clouds to the pair-morph the shader already has, which mixes vertex
//! positions and colours per particle. The renderer never learns that a
//! transition is happening.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use vizz_params::{ParamDef, ParamRegistry};

use crate::preset::Preset;

/// Cells in the grid. Sixteen, to match a sequencer row and a pad
/// controller's top row.
pub const SLOTS: usize = 16;

/// The parameters a transition drives itself rather than interpolating.
///
/// Lerping a cloud *slot* is meaningless — half way between slot 0 and
/// slot 2 is slot 1, a different cloud entirely, which would flash on
/// screen part-way through every transition. The three of them are driven
/// together instead: see [`Transition::cloud`].
const CLOUD_A: &str = "/cloud/a";
const CLOUD_B: &str = "/cloud/b";
const CLOUD_MORPH: &str = "/cloud/morph";

/// Where a switch flips during a transition.
///
/// Half way is the only defensible answer. Past that point the frame is
/// more the incoming scene than the outgoing one, so a mirror that appears
/// there reads as part of the look arriving rather than as a glitch in the
/// one leaving.
const SNAP_AT: f32 = 0.5;

/// Shortest transition that is still a transition rather than a cut.
/// Below this the smoothing in the parameter store dominates anyway.
const MIN_DURATION: f32 = 0.0;
const MAX_DURATION: f32 = 60.0;

/// How a transition travels from one scene to the next.
///
/// These are the shapes worth having in a room, not a general easing
/// library: something even, something that starts and lands gently, and
/// one of each single-ended curve for the two musical cases — a look that
/// creeps in and snaps, and one that leaves immediately and settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Curve {
    /// Constant rate. Reads as mechanical, which is sometimes what you
    /// want against a metronomic track.
    Linear,
    /// Ease in and out. The default, because it is the one that does not
    /// draw attention to the transition itself.
    #[default]
    Smooth,
    /// Slow to leave, fast to arrive.
    EaseIn,
    /// Fast to leave, slow to settle.
    EaseOut,
    /// No transition: the new scene is simply there. Kept as a curve
    /// rather than a separate mode so the grid has one code path.
    Cut,
}

impl Curve {
    pub const ALL: &'static [Curve] = &[
        Curve::Linear,
        Curve::Smooth,
        Curve::EaseIn,
        Curve::EaseOut,
        Curve::Cut,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Curve::Linear => "linear",
            Curve::Smooth => "smooth",
            Curve::EaseIn => "ease in",
            Curve::EaseOut => "ease out",
            Curve::Cut => "cut",
        }
    }

    /// Shape a 0..1 ramp. Every curve holds the endpoints exactly, so a
    /// finished transition lands on the scene's values and not near them.
    pub fn shape(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => t,
            Curve::Smooth => t * t * (3.0 - 2.0 * t),
            Curve::EaseIn => t * t,
            Curve::EaseOut => t * (2.0 - t),
            Curve::Cut => 1.0,
        }
    }
}

/// One cell of the grid: a look, under a name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub name: String,
    pub preset: Preset,
}

/// Walking the grid on its own, in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Autopilot {
    pub enabled: bool,
    /// Bars between steps. Fractional on purpose — half a bar is a
    /// legitimate rate and forcing whole bars would rule it out.
    pub bars: f32,
    pub beats_per_bar: f32,
}

impl Default for Autopilot {
    fn default() -> Self {
        Self {
            enabled: false,
            bars: 4.0,
            beats_per_bar: 4.0,
        }
    }
}

impl Autopilot {
    /// Beats between steps, floored to something sane so a zero cannot
    /// divide by zero or fire every frame.
    fn step_beats(&self) -> f64 {
        (self.bars.max(0.25) as f64) * (self.beats_per_bar.max(1.0) as f64)
    }
}

/// A transition in flight.
#[derive(Debug, Clone)]
struct Transition {
    /// The cell being moved to, for the UI and for `current` when it ends.
    to_slot: usize,
    /// Where every parameter was when the transition started. Captured
    /// from the live values rather than from the outgoing cell, so firing
    /// a scene mid-transition, or after moving things by hand, starts from
    /// what is actually on screen.
    from: BTreeMap<String, f32>,
    /// Where they are going. The outgoing values with the cell's written
    /// over the top, so a scene that does not name a parameter leaves it
    /// alone instead of dragging it to a default.
    to: BTreeMap<String, f32>,
    /// The two cloud slots to morph between, outgoing then incoming.
    cloud: (f32, f32),
    elapsed: f32,
    duration: f32,
    curve: Curve,
}

/// Sixteen scenes, the blend between them, and the autopilot that walks
/// them without you.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grid {
    /// Always [`SLOTS`] long. A `Vec` rather than an array so serde does
    /// not need const-generic help, with the length restored on load.
    cells: Vec<Option<Cell>>,
    /// Transition length in seconds.
    pub duration: f32,
    pub curve: Curve,
    pub autopilot: Autopilot,
    /// The cell that finished arriving, if any.
    #[serde(skip)]
    current: Option<usize>,
    #[serde(skip)]
    transition: Option<Transition>,
    /// Which autopilot step the clock was in last tick. `None` until the
    /// first tick, so enabling autopilot never fires immediately — it
    /// waits for the next boundary, which is what "in time" means.
    #[serde(skip)]
    last_step: Option<i64>,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            cells: vec![None; SLOTS],
            duration: 2.0,
            curve: Curve::default(),
            autopilot: Autopilot::default(),
            current: None,
            transition: None,
            last_step: None,
        }
    }
}

impl Grid {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cells(&self) -> &[Option<Cell>] {
        &self.cells
    }

    pub fn cell(&self, slot: usize) -> Option<&Cell> {
        self.cells.get(slot)?.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(|c| c.is_none())
    }

    /// The cell fully arrived at, if the last transition finished.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// The cell being moved to and how far along, for the UI to draw.
    pub fn in_flight(&self) -> Option<(usize, f32)> {
        let t = self.transition.as_ref()?;
        Some((t.to_slot, t.progress()))
    }

    /// Capture the live parameter values into a slot.
    pub fn store(&mut self, slot: usize, name: impl Into<String>, reg: &ParamRegistry) {
        if slot >= self.cells.len() {
            return;
        }
        self.cells[slot] = Some(Cell {
            name: name.into(),
            preset: Preset::capture(reg),
        });
    }

    /// Put an existing preset into a slot — how a built-in or a saved
    /// preset gets onto the grid without being recalled first.
    pub fn put(&mut self, slot: usize, name: impl Into<String>, preset: Preset) {
        if slot >= self.cells.len() {
            return;
        }
        self.cells[slot] = Some(Cell {
            name: name.into(),
            preset,
        });
    }

    /// Empty a slot. A transition already heading there is abandoned:
    /// finishing a move to a scene that no longer exists would leave the
    /// grid claiming to be somewhere it is not.
    pub fn clear(&mut self, slot: usize) {
        if slot >= self.cells.len() {
            return;
        }
        self.cells[slot] = None;
        if self.transition.as_ref().is_some_and(|t| t.to_slot == slot) {
            self.transition = None;
        }
        if self.current == Some(slot) {
            self.current = None;
        }
    }

    /// Start moving to a scene. Firing an empty slot does nothing, so a
    /// pad controller with sixteen buttons and four scenes is safe.
    ///
    /// Firing during a transition re-aims from wherever the blend has
    /// reached, which is what makes the grid playable: you are never
    /// locked out until the last move finishes.
    pub fn fire(&mut self, slot: usize, reg: &ParamRegistry) {
        let Some(cell) = self.cells.get(slot).and_then(|c| c.as_ref()) else {
            return;
        };
        let from = Preset::capture(reg).values;
        let mut to = from.clone();
        for (addr, value) in &cell.preset.values {
            to.insert(addr.clone(), *value);
        }
        let cloud = (effective_cloud(&from), effective_cloud(&to));
        let duration = self.duration.clamp(MIN_DURATION, MAX_DURATION);
        self.transition = Some(Transition {
            to_slot: slot,
            from,
            to,
            cloud,
            elapsed: 0.0,
            duration,
            curve: self.curve,
        });
        // A zero-length or cut transition should land on this frame rather
        // than on the next one, so a cut is genuinely a cut.
        let clock = 0.0;
        self.advance(clock, reg);
    }

    /// Advance the blend and the autopilot, writing the interpolated
    /// values into the registry.
    ///
    /// `beats` is the musical clock, monotonic since the last reset.
    /// Values go in as parameter *targets*, so the store's own smoothing
    /// still rides on top; that softens the very start and end of a
    /// transition and is why the curve does not need to.
    pub fn tick(&mut self, dt: f32, beats: f64, reg: &ParamRegistry) {
        self.autopilot_step(beats, reg);
        self.advance(dt, reg);
    }

    fn autopilot_step(&mut self, beats: f64, reg: &ParamRegistry) {
        if !self.autopilot.enabled {
            // Forget where we were, so re-enabling waits for the next
            // boundary rather than firing on a stale step count.
            self.last_step = None;
            return;
        }
        let step = (beats / self.autopilot.step_beats()).floor() as i64;
        match self.last_step {
            None => self.last_step = Some(step),
            Some(last) if step != last => {
                self.last_step = Some(step);
                if let Some(next) = self.next_filled() {
                    self.fire(next, reg);
                }
            }
            Some(_) => {}
        }
    }

    /// How far through the current autopilot step the clock is, 0..1.
    /// `None` when the autopilot is off.
    ///
    /// For the UI. A bright button says the autopilot is on; a button
    /// filling towards the next fire says it is *working*, which is the
    /// thing you actually want to know when nothing has changed on screen
    /// for eight bars. It reads the clock rather than being told by the
    /// tick, so it is right on a frame where nothing fired.
    pub fn autopilot_phase(&self, beats: f64) -> Option<f32> {
        if !self.autopilot.enabled {
            return None;
        }
        let step = self.autopilot.step_beats();
        // Same divisor `autopilot_step` uses, so the sweep reaches the end
        // exactly when the fire happens rather than near it.
        let phase = (beats / step).rem_euclid(1.0);
        Some(phase as f32)
    }

    /// The next filled slot after the one showing, wrapping. `None` when
    /// the grid is empty — an autopilot with nothing to play stays put
    /// instead of firing into space.
    ///
    /// Public so the UI can name the pad the autopilot will move to next:
    /// during a set the useful question is "what is coming", and the grid
    /// is the only thing that knows.
    pub fn upcoming(&self) -> Option<usize> {
        self.next_filled()
    }

    fn next_filled(&self) -> Option<usize> {
        let from = self.transition.as_ref().map(|t| t.to_slot).or(self.current);
        let start = from.map_or(0, |s| s + 1);
        (0..self.cells.len())
            .map(|i| (start + i) % self.cells.len())
            .find(|i| self.cells[*i].is_some())
    }

    fn advance(&mut self, dt: f32, reg: &ParamRegistry) {
        let Some(t) = self.transition.as_mut() else {
            return;
        };
        t.elapsed += dt.max(0.0);
        let done = t.progress() >= 1.0;
        let shaped = t.curve.shape(t.progress());
        t.write(reg, shaped, done);
        if done {
            self.current = Some(t.to_slot);
            self.transition = None;
        }
    }

    /// Abandon a transition where it stands. The parameters keep the
    /// values they had reached — stopping a blend should not snap the
    /// picture anywhere.
    pub fn halt(&mut self) {
        self.transition = None;
    }
}

impl Transition {
    /// Raw 0..1 position, before the curve.
    ///
    /// A cut is a transition of no length, expressed here rather than by
    /// special-casing the curve everywhere downstream: it is finished on
    /// the frame it is fired, so it never appears in flight and the grid
    /// records it as arrived immediately.
    fn progress(&self) -> f32 {
        if self.curve == Curve::Cut || self.duration <= f32::EPSILON {
            return 1.0;
        }
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// Write the blended values as parameter targets.
    ///
    /// `done` writes the destination exactly rather than the curve's value
    /// at 1.0, so floating point cannot leave a parameter a hair short of
    /// where the scene said it should be — over a set those would
    /// accumulate.
    fn write(&self, reg: &ParamRegistry, t: f32, done: bool) {
        for (addr, to) in &self.to {
            if is_cloud(addr) {
                continue;
            }
            let Some(id) = reg.id(addr) else { continue };
            let def = &reg.defs()[id.index()];
            let from = self.from.get(addr).copied().unwrap_or(*to);
            let value = if done {
                *to
            } else if snaps(def) {
                if t >= SNAP_AT { *to } else { from }
            } else {
                from + (to - from) * t
            };
            reg.set(id, value);
        }
        self.write_cloud(reg, t, done);
    }

    /// Drive the cloud pair.
    ///
    /// The shader already mixes two clouds' vertex positions and colours
    /// by `/cloud/morph`, so a scene change that swaps the cloud is
    /// expressed as: pin A to the outgoing cloud, B to the incoming one,
    /// and sweep the morph across. That is the geometry blend — every
    /// particle travels from where it sat in one cloud to where it sits in
    /// the other, rather than one cloud fading out through another.
    ///
    /// When the transition finishes the incoming scene's own pair is
    /// restored, which is the same geometry it was already showing.
    fn write_cloud(&self, reg: &ParamRegistry, t: f32, done: bool) {
        let (a, b, morph) = if done {
            (
                self.to.get(CLOUD_A).copied().unwrap_or(self.cloud.1),
                self.to.get(CLOUD_B).copied().unwrap_or(self.cloud.1),
                self.to.get(CLOUD_MORPH).copied().unwrap_or(0.0),
            )
        } else {
            (self.cloud.0, self.cloud.1, t)
        };
        set_if_present(reg, CLOUD_A, a);
        set_if_present(reg, CLOUD_B, b);
        set_if_present(reg, CLOUD_MORPH, morph);
    }
}

fn set_if_present(reg: &ParamRegistry, addr: &str, value: f32) {
    if let Some(id) = reg.id(addr) {
        reg.set(id, value);
    }
}

fn is_cloud(addr: &str) -> bool {
    addr == CLOUD_A || addr == CLOUD_B || addr == CLOUD_MORPH
}

/// Which cloud a parameter set is actually showing.
///
/// A scene stores a pair and a morph between them, but only one of the two
/// is on screen at either end of that morph, and that is the cloud a
/// transition has to blend from or to.
fn effective_cloud(values: &BTreeMap<String, f32>) -> f32 {
    let a = values.get(CLOUD_A).copied().unwrap_or(0.0);
    let b = values.get(CLOUD_B).copied().unwrap_or(1.0);
    let morph = values.get(CLOUD_MORPH).copied().unwrap_or(0.0);
    if morph >= 0.5 { b } else { a }
}

/// Parameters that jump rather than sweep.
///
/// A parameter that declares no smoothing *and* names its positions is a
/// switch: `/fx/mirror` has an off, an x and a quad, and nothing sensible
/// between them. Sweeping one would spend the transition showing states
/// neither scene asked for. Everything else interpolates, including
/// `/shape/mode`, which is declared as a sweep precisely because the
/// shader blends adjacent forms.
fn snaps(def: &ParamDef) -> bool {
    def.smooth <= f32::EPSILON && def.labels.is_some()
}

/// Where grids live, beside patches, presets and the MIDI map.
pub fn grid_path() -> PathBuf {
    crate::library::patch_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
        .join("grid.json")
}

/// Save the grid. Written and renamed, so a crash part-way cannot destroy
/// the grid that was already there.
pub fn save(grid: &Grid) -> Result<()> {
    let path = grid_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(grid)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// Load the grid, or a fresh empty one. A missing file is the normal case
/// on a first run and never an error; a corrupt one is logged and
/// replaced, because refusing to start over a bad grid file would be the
/// worst possible time to find out.
pub fn load() -> Grid {
    let path = grid_path();
    let Ok(bytes) = std::fs::read(&path) else {
        return Grid::new();
    };
    match serde_json::from_slice::<Grid>(&bytes) {
        Ok(mut grid) => {
            // A file written by another build could be any length.
            grid.cells.resize(SLOTS, None);
            grid.cells.truncate(SLOTS);
            grid.duration = grid.duration.clamp(MIN_DURATION, MAX_DURATION);
            grid
        }
        Err(e) => {
            log::error!(
                "could not read {}: {e:#} — starting with an empty grid",
                path.display()
            );
            Grid::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizz_params::ParamId;

    fn registry() -> (ParamRegistry, ParamId, ParamId, ParamId) {
        let mut b = ParamRegistry::builder();
        let hue = b.add(ParamDef::new("/particles/hue", 0.0, 1.0, 0.0).smooth(0.1));
        let mirror =
            b.add(ParamDef::new("/fx/mirror", 0.0, 3.0, 0.0).labels(&["off", "x", "y", "quad"]));
        let dim = b.add(ParamDef::new("/master/dim", 0.0, 1.0, 1.0).smooth(0.05));
        b.add(ParamDef::new(CLOUD_A, 0.0, 3.0, 0.0));
        b.add(ParamDef::new(CLOUD_B, 0.0, 3.0, 1.0));
        b.add(ParamDef::new(CLOUD_MORPH, 0.0, 1.0, 0.0).smooth(0.5));
        (b.build(), hue, mirror, dim)
    }

    /// The property the whole feature rests on: half way through, the
    /// parameters are half way between the two scenes. Not the outgoing
    /// look at half opacity over the incoming one — actually between.
    #[test]
    fn a_transition_lands_between_the_two_scenes() {
        let (reg, hue, _, _) = registry();
        let mut grid = Grid::new();
        reg.set(hue, 0.0);
        grid.store(0, "dark", &reg);
        reg.set(hue, 1.0);
        grid.store(1, "bright", &reg);

        reg.set(hue, 0.0);
        grid.duration = 1.0;
        grid.curve = Curve::Linear;
        grid.fire(1, &reg);
        grid.tick(0.5, 0.0, &reg);
        let mid = reg.target(hue);
        assert!(
            (mid - 0.5).abs() < 1e-4,
            "half way should be half way, got {mid}"
        );
    }

    #[test]
    fn a_transition_ends_exactly_on_the_scene() {
        let (reg, hue, _, _) = registry();
        let mut grid = Grid::new();
        reg.set(hue, 0.37);
        grid.store(3, "target", &reg);
        reg.set(hue, 0.9);

        grid.duration = 0.5;
        grid.fire(3, &reg);
        for _ in 0..40 {
            grid.tick(1.0 / 60.0, 0.0, &reg);
        }
        assert_eq!(
            reg.target(hue),
            0.37,
            "must land on the stored value, not near it"
        );
        assert_eq!(grid.current(), Some(3));
        assert!(grid.in_flight().is_none());
    }

    /// A cut is a cut: the values are there on the frame it is fired,
    /// not one frame later.
    #[test]
    fn a_cut_arrives_immediately() {
        let (reg, hue, _, _) = registry();
        let mut grid = Grid::new();
        reg.set(hue, 0.8);
        grid.store(0, "look", &reg);
        reg.set(hue, 0.1);

        grid.curve = Curve::Cut;
        grid.fire(0, &reg);
        assert_eq!(reg.target(hue), 0.8);
        assert_eq!(grid.current(), Some(0));
    }

    /// A switch has no half way. Sweeping `/fx/mirror` across a transition
    /// would put an x mirror and a y mirror on screen that neither scene
    /// asked for.
    #[test]
    fn a_stepped_parameter_flips_rather_than_sweeping() {
        let (reg, _, mirror, _) = registry();
        let mut grid = Grid::new();
        reg.set(mirror, 3.0);
        grid.store(0, "quad", &reg);
        reg.set(mirror, 0.0);

        grid.duration = 1.0;
        grid.curve = Curve::Linear;
        grid.fire(0, &reg);
        grid.tick(0.25, 0.0, &reg);
        assert_eq!(
            reg.target(mirror),
            0.0,
            "still the outgoing switch position"
        );
        grid.tick(0.3, 0.0, &reg);
        assert_eq!(
            reg.target(mirror),
            3.0,
            "past half way it is the incoming one"
        );
        // And never anything in between on any frame.
        reg.set(mirror, 0.0);
        grid.fire(0, &reg);
        for _ in 0..60 {
            grid.tick(1.0 / 60.0, 0.0, &reg);
            let v = reg.target(mirror);
            assert!(v == 0.0 || v == 3.0, "mirror passed through {v}");
        }
    }

    /// The geometry blend: while a transition is running the two clouds
    /// are pinned to the pair and the morph sweeps, which is what makes
    /// the shader interpolate vertex positions.
    #[test]
    fn different_clouds_are_morphed_rather_than_switched() {
        let (reg, _, _, _) = registry();
        let a = reg.id(CLOUD_A).unwrap();
        let b = reg.id(CLOUD_B).unwrap();
        let morph = reg.id(CLOUD_MORPH).unwrap();
        let mut grid = Grid::new();

        // Scene 1 shows cloud 2, scene 0 shows cloud 0.
        reg.set(a, 2.0);
        reg.set(morph, 0.0);
        grid.store(1, "two", &reg);
        reg.set(a, 0.0);
        grid.store(0, "zero", &reg);

        grid.duration = 1.0;
        grid.curve = Curve::Linear;
        grid.fire(1, &reg);
        grid.tick(0.5, 0.0, &reg);
        assert_eq!(reg.target(a), 0.0, "A stays on the outgoing cloud");
        assert_eq!(reg.target(b), 2.0, "B is the incoming cloud");
        assert!(
            (reg.target(morph) - 0.5).abs() < 1e-4,
            "the morph is the blend, got {}",
            reg.target(morph)
        );

        grid.tick(0.6, 0.0, &reg);
        assert_eq!(
            reg.target(a),
            2.0,
            "the arrived scene's own pair is restored"
        );
        assert_eq!(reg.target(morph), 0.0);
    }

    /// The cloud slot must never take a fractional value: the shader
    /// truncates it to an index, so 0.5 between slot 0 and slot 1 would
    /// show slot 0 while claiming to be between.
    #[test]
    fn a_cloud_slot_is_never_a_fraction_of_one() {
        let (reg, _, _, _) = registry();
        let a = reg.id(CLOUD_A).unwrap();
        let b = reg.id(CLOUD_B).unwrap();
        let mut grid = Grid::new();
        reg.set(a, 3.0);
        reg.set(b, 1.0);
        grid.store(0, "far", &reg);
        reg.set(a, 0.0);
        reg.set(b, 2.0);

        grid.duration = 1.0;
        grid.fire(0, &reg);
        for _ in 0..70 {
            grid.tick(1.0 / 60.0, 0.0, &reg);
            for id in [a, b] {
                let v = reg.target(id);
                assert_eq!(v, v.round(), "cloud slot landed on {v}");
            }
        }
    }

    /// The panic fader is not a scene parameter. A grid that restored it
    /// could undo a blackout someone reached for.
    #[test]
    fn a_scene_never_touches_the_master_dim() {
        let (reg, _, _, dim) = registry();
        let mut grid = Grid::new();
        reg.set(dim, 1.0);
        grid.store(0, "full", &reg);
        reg.set(dim, 0.0);

        grid.duration = 0.2;
        grid.fire(0, &reg);
        for _ in 0..30 {
            grid.tick(1.0 / 60.0, 0.0, &reg);
            assert_eq!(reg.target(dim), 0.0, "the blackout was undone");
        }
    }

    /// Firing an empty pad must do nothing at all — not jump, not clear,
    /// not cancel what is running. Sixteen pads and four scenes is the
    /// normal state of a grid.
    #[test]
    fn firing_an_empty_slot_does_nothing() {
        let (reg, hue, _, _) = registry();
        let mut grid = Grid::new();
        reg.set(hue, 0.25);
        grid.fire(7, &reg);
        assert!(grid.in_flight().is_none());
        assert_eq!(grid.current(), None);
        assert_eq!(reg.target(hue), 0.25);
        grid.fire(SLOTS + 5, &reg);
        assert_eq!(reg.target(hue), 0.25);
    }

    /// Re-aiming mid-blend starts from what is on screen, so a fired scene
    /// never snaps back to where the last transition began.
    #[test]
    fn firing_mid_transition_starts_from_where_the_blend_reached() {
        let (reg, hue, _, _) = registry();
        let mut grid = Grid::new();
        reg.set(hue, 0.0);
        grid.store(0, "a", &reg);
        reg.set(hue, 1.0);
        grid.store(1, "b", &reg);

        reg.set(hue, 0.0);
        grid.duration = 1.0;
        grid.curve = Curve::Linear;
        grid.fire(1, &reg);
        grid.tick(0.5, 0.0, &reg);
        assert!((reg.target(hue) - 0.5).abs() < 1e-4);

        // Turn round and head back: the first frame must stay at 0.5,
        // not jump to either end.
        grid.fire(0, &reg);
        grid.tick(0.0, 0.0, &reg);
        assert!(
            (reg.target(hue) - 0.5).abs() < 1e-4,
            "re-aiming jumped to {}",
            reg.target(hue)
        );
        grid.tick(1.0, 0.0, &reg);
        assert_eq!(reg.target(hue), 0.0);
    }

    /// Autopilot fires on the boundary, not on the frame it was switched
    /// on. Switching it on mid-bar and having the scene change instantly
    /// is exactly the thing that makes it unusable in time.
    #[test]
    fn autopilot_waits_for_the_next_boundary() {
        let (reg, _, _, _) = registry();
        let mut grid = Grid::new();
        grid.store(0, "a", &reg);
        grid.store(4, "b", &reg);
        grid.curve = Curve::Cut;
        grid.autopilot = Autopilot {
            enabled: true,
            bars: 1.0,
            beats_per_bar: 4.0,
        };

        // Switched on part-way through a bar.
        grid.tick(0.0, 2.5, &reg);
        assert_eq!(grid.current(), None, "fired the moment it was enabled");
        grid.tick(0.0, 3.9, &reg);
        assert_eq!(grid.current(), None, "fired before the boundary");
        grid.tick(0.0, 4.1, &reg);
        assert_eq!(grid.current(), Some(0), "missed the boundary");
        grid.tick(0.0, 8.1, &reg);
        assert_eq!(grid.current(), Some(4), "did not walk on");
        grid.tick(0.0, 12.1, &reg);
        assert_eq!(grid.current(), Some(0), "did not wrap");
    }

    #[test]
    fn autopilot_with_an_empty_grid_stays_put() {
        let (reg, hue, _, _) = registry();
        let mut grid = Grid::new();
        grid.autopilot = Autopilot {
            enabled: true,
            bars: 1.0,
            beats_per_bar: 4.0,
        };
        reg.set(hue, 0.6);
        for i in 0..40 {
            grid.tick(1.0 / 60.0, i as f64, &reg);
        }
        assert_eq!(grid.current(), None);
        assert_eq!(reg.target(hue), 0.6);
    }

    /// A rate of zero would divide by zero, and a rate that rounds to zero
    /// beats would fire every frame. Both come from a slider someone can
    /// drag to the bottom.
    #[test]
    fn an_absurd_autopilot_rate_cannot_fire_every_frame() {
        let (reg, _, _, _) = registry();
        let mut grid = Grid::new();
        grid.store(0, "a", &reg);
        grid.autopilot = Autopilot {
            enabled: true,
            bars: 0.0,
            beats_per_bar: 0.0,
        };
        assert!(grid.autopilot.step_beats() >= 0.25);
        let mut fired = 0;
        let mut last = None;
        for i in 0..120 {
            grid.tick(1.0 / 60.0, i as f64 * 0.001, &reg);
            if grid.current() != last {
                fired += 1;
                last = grid.current();
            }
        }
        assert!(fired <= 1, "fired {fired} times in 0.12 beats");
    }

    /// Clearing the slot a transition is heading for must abandon it. The
    /// alternative is a grid that reports it has arrived at a scene that
    /// no longer exists.
    #[test]
    fn clearing_the_destination_abandons_the_transition() {
        let (reg, hue, _, _) = registry();
        let mut grid = Grid::new();
        reg.set(hue, 1.0);
        grid.store(2, "going", &reg);
        reg.set(hue, 0.0);
        grid.duration = 1.0;
        grid.fire(2, &reg);
        grid.tick(0.25, 0.0, &reg);
        grid.clear(2);
        assert!(grid.in_flight().is_none());
        let held = reg.target(hue);
        grid.tick(0.5, 0.0, &reg);
        assert_eq!(
            reg.target(hue),
            held,
            "an abandoned transition kept writing"
        );
    }

    /// Every curve has to hold both ends, or a transition either starts
    /// with a jump or never quite arrives.
    #[test]
    fn every_curve_holds_its_endpoints() {
        for c in Curve::ALL {
            if *c != Curve::Cut {
                assert_eq!(c.shape(0.0), 0.0, "{} moved at t=0", c.name());
            }
            assert_eq!(c.shape(1.0), 1.0, "{} did not arrive", c.name());
            assert_eq!(c.shape(-5.0), c.shape(0.0), "{} did not clamp", c.name());
            assert_eq!(c.shape(5.0), 1.0, "{} did not clamp", c.name());
        }
    }

    /// Monotonic: a transition that backs up part-way through looks like a
    /// bug even when the endpoints are right.
    #[test]
    fn every_curve_only_moves_forward() {
        for c in Curve::ALL {
            let mut last = c.shape(0.0);
            for i in 0..=100 {
                let v = c.shape(i as f32 / 100.0);
                assert!(v >= last - 1e-6, "{} went backwards at {i}", c.name());
                last = v;
            }
        }
    }

    #[test]
    fn a_grid_survives_a_round_trip_through_json() {
        let (reg, hue, _, _) = registry();
        let mut grid = Grid::new();
        reg.set(hue, 0.42);
        grid.store(9, "saved", &reg);
        grid.duration = 3.5;
        grid.curve = Curve::EaseIn;
        grid.autopilot = Autopilot {
            enabled: true,
            bars: 2.0,
            beats_per_bar: 3.0,
        };

        let json = serde_json::to_vec(&grid).unwrap();
        let back: Grid = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.cells.len(), SLOTS);
        assert_eq!(back.cell(9).map(|c| c.name.as_str()), Some("saved"));
        assert_eq!(back.cell(9).unwrap().preset.values["/particles/hue"], 0.42);
        assert_eq!(back.duration, 3.5);
        assert_eq!(back.curve, Curve::EaseIn);
        assert_eq!(back.autopilot.bars, 2.0);
    }

    /// A file from another build can have any number of cells. Trusting
    /// the length would index out of bounds on the first pad press.
    #[test]
    fn a_grid_file_with_the_wrong_number_of_cells_is_repaired() {
        let json = br#"{"cells":[null,null],"duration":900.0,"curve":"linear",
            "autopilot":{"enabled":false,"bars":4.0,"beats_per_bar":4.0}}"#;
        let mut grid: Grid = serde_json::from_slice(json).unwrap();
        grid.cells.resize(SLOTS, None);
        grid.cells.truncate(SLOTS);
        grid.duration = grid.duration.clamp(MIN_DURATION, MAX_DURATION);
        assert_eq!(grid.cells.len(), SLOTS);
        assert_eq!(grid.duration, MAX_DURATION);
        // And it must be safe to fire every pad.
        let (reg, _, _, _) = registry();
        for slot in 0..SLOTS + 4 {
            grid.fire(slot, &reg);
        }
    }
}
