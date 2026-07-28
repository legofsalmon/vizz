//! Frame engine: everything one frame needs that isn't the GPU itself.
//! Shared verbatim between windowed and headless modes so a benchmark run
//! measures the same code a live set executes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use vizz_audio::{AudioEngine, BAND_COUNT};
use vizz_health::{HealthConfig, HealthMonitor, HealthSnapshot};
use vizz_mod::{AudioLevels, ModEngine};
use vizz_params::ParamSnapshot;
use vizz_render::camera::Camera;
use vizz_render::particles::Uniforms;
use vizz_render::post::PostUniforms;
use vizz_render::room::RoomUniforms;

use crate::params::AppParams;

pub struct FrameEngine {
    params: Arc<AppParams>,
    pub snapshot: ParamSnapshot,
    pub health: HealthMonitor,
    /// LFOs and the beat clock. Render-thread-owned: it ticks here and the
    /// panel (drawn on this thread) edits it directly.
    pub modulation: ModEngine,
    /// Audio capture and analysis. Always present — when no device could
    /// be opened it simply reports zeros, so nothing downstream needs a
    /// branch for "no audio".
    pub audio: AudioEngine,
    /// Scratch for this frame's band envelopes, so the tick borrows a
    /// slice rather than allocating.
    bands: [f32; BAND_COUNT],
    /// Visual time, pre-integrated so `/particles/speed` changes modulate
    /// the rate without jumping the phase.
    vis_time: f32,
    last_frame: Option<Instant>,
    last_log: Instant,
    /// Last `/preset/recall` index acted on. Presets fire on *change*, so
    /// a controller parked on an index does not re-apply it every frame
    /// and fight whatever you are adjusting by hand.
    last_preset: Option<usize>,
    /// The scene grid and the blend between its cells. Lives here rather
    /// than in the UI because it writes parameter targets every frame and
    /// has to keep doing so with the panel hidden.
    pub grid: vizz_mod::scene::Grid,
    /// Last `/scene/fire` slot acted on, edge-triggered like recall.
    last_scene: Option<usize>,
    /// The gravity layer's own grid, sequencing gravity presets on its own
    /// clock. A second instance of the same machine rather than a special
    /// case: the grid already takes its preset lookup as a parameter, so
    /// pointing one at a different library is all a second layer needs.
    pub gravity_grid: vizz_mod::scene::Grid,
    last_gravity: Option<usize>,
}

pub struct FrameInputs {
    pub uniforms: Uniforms,
    pub post: PostUniforms,
    pub room: RoomUniforms,
    /// Skip the room pass entirely when it is dark — it is off by default,
    /// and drawing invisible lines every frame is wasted work.
    pub room_visible: bool,
    pub count: u32,
    /// What an empty frame looks like, alpha included. At alpha 0 the
    /// field is delivered on a transparent background so vizz can be a
    /// layer in a mixer rather than the whole picture.
    pub background: wgpu::Color,
}

impl FrameEngine {
    pub fn new(params: Arc<AppParams>, audio: AudioEngine) -> Self {
        Self {
            snapshot: ParamSnapshot::new(&params.registry),
            params,
            health: HealthMonitor::new(HealthConfig::default()),
            // Whatever was on the rack last time. Every other piece of
            // user state comes back on the next launch; this one used to
            // be thrown away, routes and all.
            modulation: vizz_mod::library::load_session()
                .unwrap_or_else(ModEngine::with_defaults),
            audio,
            bands: [0.0; BAND_COUNT],
            vis_time: 0.0,
            last_frame: None,
            last_log: Instant::now(),
            last_preset: None,
            grid: vizz_mod::scene::Grid::new(),
            last_scene: None,
            gravity_grid: vizz_mod::scene::Grid::for_kind(vizz_mod::preset::Kind::Gravity),
            last_gravity: None,
        }
    }

    /// Adopt a grid loaded from disk, pushing its stored transition
    /// settings out to the parameters that drive them.
    ///
    /// Those settings live in two places by necessity — in the grid,
    /// because a `Grid` has to be usable and testable on its own, and in
    /// the parameter store, because that is what gives them OSC, MIDI
    /// learn and a fader. The parameters are the authority from here on,
    /// so the saved values have to be written *into* them at load or they
    /// would be silently replaced by the defaults on the first frame.
    pub fn adopt_grid(&mut self, grid: vizz_mod::scene::Grid) {
        let reg = &self.params.registry;
        reg.set(self.params.scene_time, grid.duration);
        reg.set(
            self.params.scene_curve,
            vizz_mod::scene::Curve::ALL
                .iter()
                .position(|c| *c == grid.curve)
                .unwrap_or(1) as f32,
        );
        reg.set(
            self.params.scene_auto,
            if grid.autopilot.enabled { 1.0 } else { 0.0 },
        );
        reg.set(self.params.scene_bars, grid.autopilot.bars);
        self.grid = grid;
    }

    /// As [`Self::adopt_grid`], for the gravity layer.
    pub fn adopt_gravity_grid(&mut self, grid: vizz_mod::scene::Grid) {
        let reg = &self.params.registry;
        reg.set(self.params.gravity_time, grid.duration);
        reg.set(
            self.params.gravity_curve,
            vizz_mod::scene::Curve::ALL
                .iter()
                .position(|c| *c == grid.curve)
                .unwrap_or(1) as f32,
        );
        reg.set(
            self.params.gravity_auto,
            if grid.autopilot.enabled { 1.0 } else { 0.0 },
        );
        reg.set(self.params.gravity_bars, grid.autopilot.bars);
        self.gravity_grid = grid;
    }

    /// Fire a scene when `/scene/fire` has moved, then advance the blend.
    ///
    /// Edge-triggered for the same reason recall is: a pad controller
    /// holding a slot down must not restart the transition every frame.
    /// Slot 0 is "nothing selected" and the grid runs from 1, so a fresh
    /// start cannot fire cell 0 over your defaults.
    ///
    /// The transition settings are read from the parameters every frame,
    /// so a knob or an OSC message changes them mid-set. `duration` is
    /// only read *between* transitions — changing the blend time while one
    /// is running would make the transition already in flight jump.
    fn tick_grid(&mut self, dt: f32) {
        use vizz_mod::scene::Curve;
        let p = Arc::clone(&self.params);
        let reg = &p.registry;

        if self.grid.in_flight().is_none() {
            self.grid.duration = reg.target(p.scene_time);
        }
        let curve = reg.target(p.scene_curve).round().max(0.0) as usize;
        self.grid.curve = Curve::ALL.get(curve).copied().unwrap_or_default();
        self.grid.autopilot.enabled = reg.target(p.scene_auto) >= 0.5;
        self.grid.autopilot.bars = reg.target(p.scene_bars);

        // Scenes name presets rather than carrying copies, so firing one
        // resolves through the library here. Built-ins and saved presets
        // are equally addressable, which is what lets a set be prepared
        // from either.
        let presets = |name: &str| vizz_mod::preset::by_name(name);
        let slot = reg.target(p.scene_fire).round().max(0.0) as usize;
        if self.last_scene != Some(slot) {
            self.last_scene = Some(slot);
            if let Some(index) = slot.checked_sub(1) {
                self.grid.fire(index, reg, &presets);
            }
        }
        self.grid
            .tick(dt, self.modulation.clock.beats, reg, &presets);

        // The gravity layer, on its own transport and its own library.
        // Independent all the way down: a look firing cannot disturb the
        // wells and a well firing cannot disturb the look, which is the
        // whole reason the two are separate grids rather than one.
        let gravity_presets =
            |name: &str| vizz_mod::preset::load_kind(vizz_mod::preset::Kind::Gravity, name).ok();
        if self.gravity_grid.in_flight().is_none() {
            self.gravity_grid.duration = reg.target(p.gravity_time);
        }
        let gcurve = reg.target(p.gravity_curve).round().max(0.0) as usize;
        self.gravity_grid.curve = Curve::ALL.get(gcurve).copied().unwrap_or_default();
        self.gravity_grid.autopilot.enabled = reg.target(p.gravity_auto) >= 0.5;
        self.gravity_grid.autopilot.bars = reg.target(p.gravity_bars);
        let gslot = reg.target(p.gravity_fire).round().max(0.0) as usize;
        if self.last_gravity != Some(gslot) {
            self.last_gravity = Some(gslot);
            if let Some(index) = gslot.checked_sub(1) {
                self.gravity_grid.fire(index, reg, &gravity_presets);
            }
        }
        self.gravity_grid
            .tick(dt, self.modulation.clock.beats, reg, &gravity_presets);
    }

    /// Recall a preset when `/preset/recall` has moved to a new index.
    ///
    /// Edge-triggered, not level-triggered. A MIDI button or an OSC client
    /// parked on an index would otherwise re-apply that preset every
    /// frame, which pins every parameter it names and makes them
    /// impossible to adjust by hand — the control would feel broken rather
    /// than latched.
    ///
    /// Slot 0 means "nothing selected", and presets start at 1. That is
    /// what makes startup safe *by construction*: the control rests at 0,
    /// so the first frame has nothing to recall and cannot stamp preset 0
    /// over the defaults. It also keeps the first preset reachable —
    /// number it from 0 and it can never be fired from a fresh start,
    /// because the control is already sitting on it.
    ///
    /// A slot past the end of the list is ignored, and still recorded, so
    /// a fader swept across the full range settles quietly instead of
    /// retrying every frame.
    fn apply_pending_preset(&mut self) {
        let reg = &self.params.registry;
        let slot = reg.target(self.params.preset_recall).round().max(0.0) as usize;
        if self.last_preset == Some(slot) {
            return;
        }
        self.last_preset = Some(slot);
        let Some(index) = slot.checked_sub(1) else {
            return;
        };
        match vizz_mod::preset::by_index(index) {
            Some((name, preset)) => {
                let applied = preset.apply(reg);
                log::info!("recalled preset {slot}: {name} ({applied} parameters)");
            }
            None => log::debug!("no preset in slot {slot}"),
        }
    }

    /// Advance time and parameters; returns everything the scene needs.
    /// `fixed_dt` pins the timestep (headless benchmarking); `None` uses
    /// wall-clock time (live).
    pub fn begin_frame(&mut self, aspect: f32, fixed_dt: Option<Duration>) -> FrameInputs {
        let now = Instant::now();
        let dt = fixed_dt.unwrap_or_else(|| {
            let dt = self
                .last_frame
                .map_or(Duration::from_nanos(16_666_667), |t| now - t);
            // Clamp: a debugger pause or window drag must not fast-forward
            // the visuals or the parameter smoothing.
            dt.min(Duration::from_millis(100))
        });
        self.last_frame = Some(now);

        let dt_s = dt.as_secs_f32();
        // Recall before taking a borrow of `params`, and before smoothing
        // advances, so a preset's values are targets for this same frame
        // and the glide starts immediately rather than one frame late.
        self.apply_pending_preset();
        // Then the grid, so a scene fired on this frame starts blending on
        // it — and so a transition in flight wins over a preset recalled
        // underneath it, which is the order you would expect from the
        // thing you most recently touched.
        self.tick_grid(dt_s);
        let p = Arc::clone(&self.params);
        let p = &*p;
        for (i, b) in self.bands.iter_mut().enumerate() {
            *b = self.audio.state.band(i);
        }
        // Detected tempo drives the clock only when asked and only when the
        // detector is sure. Ambient material still produces *a* peak, and
        // letting that retune the clock mid-set is worse than a tempo that
        // is slightly stale.
        if let Ok(settings) = self.audio.settings.lock() {
            if settings.auto_bpm {
                let bpm = self.audio.state.bpm();
                if bpm > 0.0 && self.audio.state.confidence() >= settings.min_confidence {
                    self.modulation.clock.bpm = bpm;
                }
            }
        }
        // Modulation is an offset on top of the stored targets, so a value
        // set by hand or by MIDI is never overwritten.
        let levels = AudioLevels {
            bands: &self.bands,
            level: self.audio.state.level(),
        };
        let offsets = self.modulation.tick(dt_s, &p.registry, levels);
        self.snapshot.advance_modulated(&p.registry, dt_s, offsets);
        self.vis_time += dt_s * self.snapshot.get(p.speed);

        // Master dim multiplies *everything* that emits light. It is the
        // fader you grab when something is wrong on a big screen, so
        // anything it does not reach is a thing still lit when you have
        // asked for black — the room used to be exactly that.
        let dim = self.snapshot.get(p.dim);
        let brightness = self.snapshot.get(p.brightness) * dim;

        let camera = Camera {
            distance: self.snapshot.get(p.cam_dist),
            orbit: self.snapshot.get(p.cam_orbit),
            elevation: self.snapshot.get(p.cam_elev),
            fov: self.snapshot.get(p.cam_fov),
            aspect,
            focus: self.snapshot.get(p.cam_focus),
            defocus: self.snapshot.get(p.cam_defocus),
            pan_x: self.snapshot.get(p.cam_pan_x),
            pan_y: self.snapshot.get(p.cam_pan_y),
        };
        let cam = camera.uniforms();
        let room_brightness = self.snapshot.get(p.room) * dim;
        // The opening sits a little in front of the origin so the cloud is
        // inside the room rather than pressed against its face.
        let room = RoomUniforms::for_camera(
            &camera,
            (camera.distance - 1.6).max(0.3),
            self.snapshot.get(p.room_depth),
            room_brightness,
            self.snapshot.get(p.room_fade),
            self.snapshot.get(p.room_converge),
            self.snapshot.get(p.room_vanish_x),
            self.snapshot.get(p.room_vanish_y),
        );
        // Placement is derived even when the room is invisible: embedding
        // the cloud in a room you have not turned up is a legitimate look,
        // and it costs a struct copy.
        let placement = room.placement(
            self.snapshot.get(p.room_anchor),
            self.snapshot.get(p.room_embed),
        );

        FrameInputs {
            uniforms: Uniforms {
                view_proj: cam.view_proj,
                cam_right: cam.right,
                focus: camera.focus,
                cam_up: cam.up,
                defocus: camera.defocus,
                cam_position: cam.position,
                _pad_cam: 0.0,
                time: self.vis_time,
                aspect,
                size: self.snapshot.get(p.size),
                spread: self.snapshot.get(p.spread),
                hue: self.snapshot.get(p.hue),
                saturation: self.snapshot.get(p.saturation),
                brightness,
                shape: self.snapshot.get(p.shape),
                morph: self.snapshot.get(p.morph),
                twist: self.snapshot.get(p.twist),
                palette: self.snapshot.get(p.palette),
                color_spread: self.snapshot.get(p.color_spread),
                // Stepped like mirror: a value sliding between two drive
                // modes is not a crossfade, it is a wrong third thing.
                color_drive: self.snapshot.get(p.color_drive).round(),
                gravity: std::array::from_fn(|i| match p.gravity.get(i) {
                    Some(w) => [
                        self.snapshot.get(w.x),
                        self.snapshot.get(w.y),
                        self.snapshot.get(w.z),
                        self.snapshot.get(w.strength),
                    ],
                    None => [0.0; 4],
                }),
                gravity_radius: std::array::from_fn(|i| {
                    p.gravity.get(i).map_or(1.0, |w| self.snapshot.get(w.radius))
                }),
                // The master dim does not scale gravity: it is a shape
                // control, and fading the output to black should not also
                // straighten the cloud out on the way down.
                gravity_amount: [self.snapshot.get(p.gravity_amount), 0.0, 0.0, 0.0],
                // Filled by the caller, which owns the palette bank.
                palette_rows: [4.0, 0.0, 0.0, 0.0],
                // Slot choice is stepped; the morph between them is not.
                cloud_a: self.snapshot.get(p.cloud_a).round(),
                cloud_b: self.snapshot.get(p.cloud_b).round(),
                cloud_morph: self.snapshot.get(p.cloud_morph),
                room: placement,
            },
            post: PostUniforms {
                trail: self.snapshot.get(p.trail),
                zoom: self.snapshot.get(p.zoom),
                spin: self.snapshot.get(p.spin),
                // Rounded: mirror modes are discrete, and a smoothed value
                // sliding between them would flicker between folds.
                mirror: self.snapshot.get(p.mirror).round(),
                glow: self.snapshot.get(p.glow),
                aspect,
                shift: self.snapshot.get(p.shift),
                _pad0: 0.0,
            },
            room,
            room_visible: room_brightness > 0.002,
            count: self.snapshot.get(p.count).max(0.0) as u32,
            background: wgpu::Color {
                // The master dim scales the background as well as the
                // field. Pulling the master and being left with a lit
                // backdrop would make the one emergency control not work.
                r: (self.snapshot.get(p.bg_r) * dim) as f64,
                g: (self.snapshot.get(p.bg_g) * dim) as f64,
                b: (self.snapshot.get(p.bg_b) * dim) as f64,
                // Alpha is *not* dimmed: transparency is a routing
                // decision, not a brightness one, and fading the master
                // should not quietly make the output opaque.
                a: self.snapshot.get(p.bg_a) as f64,
            },
        }
    }

    /// Record the finished frame; returns a snapshot when the periodic
    /// health log is due (every 2s) so callers can print/display it.
    pub fn end_frame(&mut self, frame_time: Duration) -> Option<HealthSnapshot> {
        self.health.on_frame(frame_time);
        if self.last_log.elapsed() >= Duration::from_secs(2) {
            self.last_log = Instant::now();
            Some(self.health.snapshot())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> FrameEngine {
        // "\0none" matches no device, so the engine takes its normal
        // unavailable path and reports zeros. No GPU is involved here.
        FrameEngine::new(
            Arc::new(AppParams::build()),
            vizz_audio::AudioEngine::start(Some("\0none")),
        )
    }

    /// Starting up must not recall anything. `/preset/recall` defaults to
    /// 0, and treating the first frame as a change fires preset 0 over the
    /// defaults before the window is even on screen — which is how this
    /// was found: every headless render logged "recalled preset 0".
    #[test]
    fn startup_does_not_recall_a_preset() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let glow = reg.id("/fx/glow").unwrap();
        let before = reg.target(glow);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(
            reg.target(glow),
            before,
            "startup recalled a preset over the defaults"
        );
    }

    /// Moving the control recalls; staying put does not. Level-triggering
    /// would re-apply every frame and pin every parameter the preset
    /// names, so adjusting one by hand afterwards would be impossible.
    #[test]
    fn recall_fires_on_change_and_only_on_change() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        let glow = reg.id("/fx/glow").unwrap();

        // Slot 1 is the first built-in, so this must change something.
        reg.set_by_addr("/preset/recall", 1.0);
        e.begin_frame(16.0 / 9.0, dt);
        let recalled = reg.target(glow);
        let expected = vizz_mod::preset::BUILTINS[0]
            .preset()
            .values
            .get("/fx/glow")
            .copied()
            .expect("the first built-in should set glow");
        assert!((recalled - expected).abs() < 1e-6, "preset did not apply");

        // Now move it by hand with recall parked. A second frame must not
        // stamp the preset back over the top.
        reg.set_by_addr("/fx/glow", 0.02);
        e.begin_frame(16.0 / 9.0, dt);
        assert!(
            (reg.target(glow) - 0.02).abs() < 1e-6,
            "a parked recall re-applied its preset and fought the user"
        );
    }

    /// The master dim is the fader you reach for when something is wrong
    /// in front of an audience. Anything it fails to reach is still lit
    /// when you have asked for black — the room was, until this test.
    #[test]
    fn the_master_dim_blacks_out_the_room_too() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        reg.set_by_addr("/room/brightness", 1.0);
        // Dim is smoothed, so run long enough for it to arrive.
        reg.set_by_addr("/master/dim", 0.0);
        for _ in 0..120 {
            e.begin_frame(16.0 / 9.0, dt);
        }
        let f = e.begin_frame(16.0 / 9.0, dt);
        assert!(
            f.room.brightness < 0.01,
            "room still lit at {} with the master dim down",
            f.room.brightness
        );
        assert!(
            !f.room_visible,
            "room pass still running with the master dim down"
        );
    }

    /// An index past the end is ignored rather than applying whatever is
    /// nearest, and sweeping a fader through empty indices must not spam
    /// or reset anything.
    #[test]
    fn an_empty_index_changes_nothing() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        let glow = reg.id("/fx/glow").unwrap();
        reg.set_by_addr("/fx/glow", 0.33);
        for index in [40.0, 41.0, 63.0] {
            reg.set_by_addr("/preset/recall", index);
            e.begin_frame(16.0 / 9.0, dt);
            assert!(
                (reg.target(glow) - 0.33).abs() < 1e-6,
                "index {index} disturbed a parameter"
            );
        }
    }
}
