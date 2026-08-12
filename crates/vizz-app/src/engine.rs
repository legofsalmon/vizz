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
    ///
    /// `f64` because this is a running sum with a small increment. In
    /// `f32` the increment starts being lost into the accumulator's own
    /// rounding as the total grows: with a 60 Hz frame and speed 1, the
    /// step is about a sixtieth of a second against a value whose spacing
    /// has widened to a comparable size after a few hours, and additions
    /// go missing entirely at around six days of running. An installation
    /// left up overnight is inside the first of those.
    ///
    /// This fixes the accumulator, not the shader. What is handed to the
    /// GPU is still an `f32` of the same magnitude, so its own resolution
    /// at large times is unchanged. Wrapping it would fix that too, and is
    /// not available: the per-particle spin rate is a continuous range, so
    /// the field has no period to wrap at and any wrap would jump it. That
    /// is a look change rather than a bug fix, so it is not made here.
    vis_time: f64,
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
    /// A zero-second scene change landed this frame and the smoothing
    /// has to be skipped once, or "cut" fades like everything else.
    cut_pending: bool,
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
    /// The vector layer stack, packed for the shader. The caller fills
    /// the render height into `bg[3]` before drawing — the engine does
    /// not know the target size, and the shader derives its analytic
    /// pixel footprint from it.
    pub vector: vizz_render::vector::StackU,
    /// Whether any layer is on. When false the vector pass is skipped
    /// entirely and the frame is byte-identical to one rendered before
    /// vector layers existed — the guarantee that makes shipping an
    /// experimental renderer inside the live instrument tolerable.
    pub vector_active: bool,
    /// True when `/vec/place` says "print": the stack draws after the
    /// post chain instead of into it.
    pub vector_print: bool,
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
            cut_pending: false,
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
                // A zero-second blend is a cut, and has to reach the
                // picture as one. The grid writes its targets instantly,
                // but every parameter then crossed over its own
                // smoothing constant — so "cut" faded, and the one
                // control whose whole job is to be instant was the one
                // that could not be. Marked here and honoured after the
                // slew runs, which is the only place it can be undone.
                if self.grid.duration <= f32::EPSILON {
                    self.cut_pending = true;
                }
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
                // A look transition in flight would rewrite these same
                // parameters on its very next frame, silently eating the
                // recall — the number key would appear to do nothing.
                // The recall is edge-triggered, so reaching here means it
                // is the thing most recently touched; it wins.
                self.grid.halt();
                log::info!("recalled preset {slot}: {name} ({applied} parameters)");
            }
            None => log::debug!("no preset in slot {slot}"),
        }
    }

    /// Forget the last recall edge, so the next tick re-applies whatever
    /// slot `/preset/recall` is sitting on. The number keys call this on
    /// every press: recall is edge-triggered (see `apply_pending_preset`),
    /// so without it pressing the key for the preset already showing does
    /// nothing — and "press the number again to snap back after tweaking"
    /// is exactly what the keys are for.
    pub fn retrigger_preset(&mut self) {
        self.last_preset = None;
    }

    /// The `/preset/recall` slot last acted on, 1-based; `None` when
    /// nothing has been recalled. For the preset row, which had no way to
    /// show *where you are* — every button looked identical whether its
    /// look was on screen or not.
    pub fn current_preset(&self) -> Option<usize> {
        self.last_preset.filter(|s| *s > 0)
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
        // Then the grid, so a scene fired on this frame starts blending
        // on it. A recall on this frame has already halted any transition
        // in flight — both are edge-triggered, so whichever the user
        // touched last is the one writing the parameters.
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
        // try_lock: the analysis thread takes this ~94 times a second,
        // and a render thread parked behind a preempted holder is a
        // missed vsync for nothing — a one-frame-stale reading serves
        // auto-bpm exactly as well.
        if let Ok(settings) = self.audio.settings.try_lock()
            && settings.auto_bpm {
                let bpm = self.audio.state.bpm();
                if bpm > 0.0 && self.audio.state.confidence() >= settings.min_confidence {
                    self.modulation.clock.bpm = bpm;
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
        // After the slew, not instead of it: modulation still rides on
        // top, and a cut lands the *set* value rather than freezing
        // whatever an LFO happened to be adding.
        if std::mem::take(&mut self.cut_pending) {
            self.snapshot.snap(&p.registry);
        }
        // Freeze holds the picture: visual time stops advancing, and the
        // feedback pass is pinned to full trail below so the last frame
        // survives unchanged. Parameters keep moving underneath — a
        // transition in flight lands while frozen and shows on release,
        // which is the gesture's contract: hold the picture, not the set.
        let frozen = self.snapshot.get(p.punch_freeze) >= 0.5;
        if !frozen {
            self.vis_time += (dt_s * self.snapshot.get(p.speed)) as f64;
        }
        // The strobe's dark phase rides the same darkening as the black
        // punch, so blackout and strobe cost one uniform between them.
        // Computed from the beat clock after modulation ticked it, so the
        // flashes land on the divisions the clock is actually on.
        // A strobe with the clock stopped does nothing rather than
        // parking wherever the beat froze — stuck in the dark phase it
        // would read as a blackout that no button explains.
        let strobe = self.snapshot.get(p.punch_strobe);
        let strobe_dark = if strobe > 0.001 && self.modulation.clock.running {
            let div = self.snapshot.get(p.punch_strobe_div).max(0.05) as f64;
            let lit = (self.modulation.clock.beats / div).fract() < 0.3;
            if lit { 0.0 } else { strobe }
        } else {
            0.0
        };

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

        // The vector stack. Paper shares the /bg colour the clear uses,
        // sRGB-encoded as registered, and rides the master dim inside
        // the shader (globals lane) rather than pre-multiplied — the
        // shader dims the composited page, which is what a printed tint
        // under a fader should do. Alpha is not carried: the vector
        // page is opaque by construction, and the transparent-master
        // routing feature applies only while the stack is off.
        let mut vector = vizz_render::vector::StackU {
            globals: [aspect, self.vis_time as f32, dim, p.vector_layers.len() as f32],
            bg: [
                self.snapshot.get(p.bg_r),
                self.snapshot.get(p.bg_g),
                self.snapshot.get(p.bg_b),
                0.0, // render height, filled by the caller
            ],
            ..Default::default()
        };
        for (slot, ids) in p.vector_palette.iter().enumerate() {
            vector.palette[slot] = [
                self.snapshot.get(ids[0]),
                self.snapshot.get(ids[1]),
                self.snapshot.get(ids[2]),
                1.0,
            ];
        }
        let mut vector_active = false;
        for (i, l) in p.vector_layers.iter().enumerate() {
            let kind = self.snapshot.get(l.kind).round();
            vector_active |= kind >= 0.5;
            vector.layers[i] = vizz_render::vector::LayerU {
                xform: [
                    self.snapshot.get(l.x),
                    self.snapshot.get(l.y),
                    self.snapshot.get(l.rot),
                    self.snapshot.get(l.scale),
                ],
                // Phase advances with visual time so the whole stack
                // drifts at /particles/speed's rate like everything
                // else; the parameter is the offset on top.
                pat: [
                    kind,
                    self.snapshot.get(l.freq),
                    self.snapshot.get(l.phase) + self.vis_time as f32 * 0.1,
                    self.snapshot.get(l.duty),
                ],
                shape: [
                    self.snapshot.get(l.sides),
                    self.snapshot.get(l.inset),
                    self.snapshot.get(l.fold),
                    self.snapshot.get(l.invert).round(),
                ],
                style: [
                    self.snapshot.get(l.blend).round(),
                    self.snapshot.get(l.opacity),
                    self.snapshot.get(l.color).round(),
                    0.0,
                ],
            };
        }

        FrameInputs {
            uniforms: Uniforms {
                view_proj: cam.view_proj,
                cam_right: cam.right,
                focus: camera.focus,
                cam_up: cam.up,
                defocus: camera.defocus,
                cam_position: cam.position,
                _pad_cam: 0.0,
                time: self.vis_time as f32,
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
                // Presence and aspect are the renderer's to know — it
                // holds the texture — so they are filled in by the
                // caller alongside the palette count. Only the two
                // controls come from the parameter table here.
                video: [
                    0.0,
                    1.0,
                    self.snapshot.get(p.video_depth),
                    self.snapshot.get(p.video_relief).round(),
                ],
                // Slot choice is stepped; the morph between them is not.
                cloud_a: self.snapshot.get(p.cloud_a).round(),
                cloud_b: self.snapshot.get(p.cloud_b).round(),
                cloud_morph: self.snapshot.get(p.cloud_morph),
                room: placement,
            },
            post: PostUniforms {
                // At trail 1.0 the feedback lerp passes history through
                // unchanged — a genuine frame hold. Zoom and spin still
                // apply, which is a look (a frozen frame you can tunnel).
                trail: if frozen { 1.0 } else { self.snapshot.get(p.trail) },
                zoom: self.snapshot.get(p.zoom),
                spin: self.snapshot.get(p.spin),
                // Rounded: mirror modes are discrete, and a smoothed value
                // sliding between them would flicker between folds.
                mirror: self.snapshot.get(p.mirror).round(),
                glow: self.snapshot.get(p.glow),
                aspect,
                shift: self.snapshot.get(p.shift),
                flash: self.snapshot.get(p.punch_flash),
                invert: self.snapshot.get(p.punch_invert),
                black: self.snapshot.get(p.punch_black).max(strobe_dark),
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            },
            room,
            room_visible: room_brightness > 0.002,
            vector,
            vector_active,
            vector_print: self.snapshot.get(p.vec_place).round() >= 0.5,
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
    /// Record how long the frame spent in the UI, before `end_frame`.
    ///
    /// Separate from `end_frame` because headless has no UI at all and
    /// must not report a zero that reads as "the UI is free" — it
    /// reports nothing, and the health line omits the field.
    pub fn end_ui(&mut self, ui_time: Duration) {
        self.health.on_ui(ui_time);
    }

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

    /// Visual time is a running sum of a small increment, which is the
    /// shape that goes wrong quietly. In `f32` the additions start being
    /// swallowed by the accumulator's own rounding as it grows, and an
    /// installation left running overnight is inside the range where it
    /// matters — but nothing shows it in the first minute of a test.
    ///
    /// So this integrates the arithmetic directly, at a magnitude it takes
    /// hours to reach, and asserts the result is the sum rather than
    /// something that stopped moving.
    #[test]
    fn visual_time_still_advances_after_days_of_running() {
        // Six days at 60 Hz. Chosen because that is where an f32
        // accumulator stops advancing at all.
        const HZ: f64 = 60.0;
        const HOURS: f64 = 146.0;
        let step = 1.0 / HZ;

        let mut f64_time: f64 = 0.0;
        let mut f32_time: f32 = 0.0;
        let frames = (HOURS * 3600.0 * HZ) as u64;
        for _ in 0..frames {
            f64_time += step;
            f32_time += step as f32;
        }
        let want = frames as f64 * step;

        // The accumulator this code uses lands on the answer.
        assert!(
            (f64_time - want).abs() < 1.0,
            "f64 drifted: {f64_time} vs {want}"
        );

        // The one it replaced has stopped dead. Not merely inaccurate —
        // frozen, pinned at a power of two, taking additions that change
        // nothing. Another second of frames moves one and not the other,
        // which is the whole failure in one assertion: on screen it is
        // animation that simply stops while the app carries on running.
        let (f64_before, f32_before) = (f64_time, f32_time);
        for _ in 0..(HZ as u64) {
            f64_time += step;
            f32_time += step as f32;
        }
        assert!(f64_time > f64_before, "f64 stopped advancing");
        assert_eq!(
            f32_time, f32_before,
            "f32 was expected to be frozen by now; it still moved"
        );
    }

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

    /// Freeze holds the picture: visual time must stop dead the frame
    /// the gesture lands and resume from the same phase on release —
    /// while the feedback pass is pinned to full trail so the last frame
    /// survives.
    #[test]
    fn freeze_stops_visual_time_and_pins_the_trail() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        e.begin_frame(16.0 / 9.0, dt);

        reg.set_by_addr("/punch/freeze", 1.0);
        let a = e.begin_frame(16.0 / 9.0, dt);
        let b = e.begin_frame(16.0 / 9.0, dt);
        assert_eq!(a.uniforms.time, b.uniforms.time, "time advanced while frozen");
        assert_eq!(b.post.trail, 1.0, "trail not pinned while frozen");

        reg.set_by_addr("/punch/freeze", 0.0);
        let c = e.begin_frame(16.0 / 9.0, dt);
        assert!(c.uniforms.time > b.uniforms.time, "time did not resume");
        assert!(c.post.trail < 1.0, "trail still pinned after release");
    }

    /// A flash that fades in is not a flash: the full value must reach
    /// the GPU on the very frame it was set. This is the test that fails
    /// if someone gives the punch params a smoothing constant.
    #[test]
    fn a_flash_lands_at_full_strength_on_the_same_frame() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        e.begin_frame(16.0 / 9.0, dt);

        reg.set_by_addr("/punch/flash", 1.0);
        let f = e.begin_frame(16.0 / 9.0, dt);
        assert_eq!(f.post.flash, 1.0, "flash was smoothed on its way to the GPU");

        reg.set_by_addr("/punch/flash", 0.0);
        let f = e.begin_frame(16.0 / 9.0, dt);
        assert_eq!(f.post.flash, 0.0, "flash lingered after release");
    }

    /// The strobe alternates lit and dark phases on the beat clock, and
    /// the dark phase rides the same uniform as the blackout.
    #[test]
    fn the_strobe_alternates_on_the_beat() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        // 60 bpm = one beat per second; a quarter-beat division makes a
        // full strobe cycle every 250 ms, sampled well by 16 ms frames.
        e.modulation.clock.bpm = 60.0;
        let dt = Some(Duration::from_millis(16));
        reg.set_by_addr("/punch/strobe", 1.0);
        reg.set_by_addr("/punch/strobe_div", 0.25);

        let (mut lit, mut dark) = (0, 0);
        for _ in 0..60 {
            let f = e.begin_frame(16.0 / 9.0, dt);
            if f.post.black > 0.5 {
                dark += 1;
            } else {
                lit += 1;
            }
        }
        assert!(lit > 5, "strobe never lit ({lit} lit / {dark} dark)");
        assert!(dark > 5, "strobe never went dark ({lit} lit / {dark} dark)");

        reg.set_by_addr("/punch/strobe", 0.0);
        let f = e.begin_frame(16.0 / 9.0, dt);
        assert_eq!(f.post.black, 0.0, "strobe left the black uniform up");
    }

    /// Recalling a look must never replay somebody's blackout: no punch
    /// gesture may be captured into a preset.
    #[test]
    fn a_captured_preset_carries_no_punch_state() {
        let e = engine();
        let reg = &e.params.registry;
        reg.set_by_addr("/punch/black", 1.0);
        reg.set_by_addr("/punch/freeze", 1.0);
        let look = vizz_mod::preset::Preset::capture(reg);
        assert!(
            !look.values.keys().any(|a| a.starts_with("/punch/")),
            "a preset captured punch state: {:?}",
            look.values.keys().filter(|a| a.starts_with("/punch/")).collect::<Vec<_>>()
        );
    }

    /// A recall must survive a look transition in flight. The grid
    /// writes its blend into the same parameters every frame, so without
    /// halting it the recalled preset was on screen for one frame and
    /// then silently overwritten — the number key reads as doing nothing.
    #[test]
    fn a_recall_wins_over_a_transition_in_flight() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        let glow = reg.id("/fx/glow").unwrap();

        // Pad 1 plays the second built-in; fire it and make sure the
        // blend is genuinely in flight before recalling over it.
        e.grid.assign(0, vizz_mod::preset::BUILTINS[1].name);
        reg.set_by_addr("/scene/fire", 1.0);
        e.begin_frame(16.0 / 9.0, dt);
        assert!(e.grid.in_flight().is_some(), "no transition to survive");

        // Recall the first built-in mid-blend, then give the transition
        // more than enough frames to have stamped its target if it were
        // still alive.
        reg.set_by_addr("/preset/recall", 1.0);
        for _ in 0..240 {
            e.begin_frame(16.0 / 9.0, dt);
        }
        let expected = vizz_mod::preset::BUILTINS[0].preset().values["/fx/glow"];
        assert!(
            (reg.target(glow) - expected).abs() < 1e-6,
            "the in-flight transition overwrote the recall: glow {} vs {expected}",
            reg.target(glow)
        );
    }

    /// Pressing the number key for the preset already showing must
    /// re-apply it. Recall is edge-triggered and the key writes the same
    /// slot value, so without `retrigger_preset` the press moves nothing
    /// — and snapping back after hand-tweaking is the main thing the
    /// number keys are pressed for.
    #[test]
    fn a_repeated_number_key_reapplies_the_preset() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        let glow = reg.id("/fx/glow").unwrap();

        reg.set_by_addr("/preset/recall", 1.0);
        e.begin_frame(16.0 / 9.0, dt);
        let recalled = reg.target(glow);

        // Tweak by hand; the parked recall must not fight it...
        reg.set_by_addr("/fx/glow", 0.02);
        e.begin_frame(16.0 / 9.0, dt);
        assert!((reg.target(glow) - 0.02).abs() < 1e-6);

        // ...but the same key pressed again snaps it back.
        e.retrigger_preset();
        reg.set_by_addr("/preset/recall", 1.0);
        e.begin_frame(16.0 / 9.0, dt);
        assert!(
            (reg.target(glow) - recalled).abs() < 1e-6,
            "the repeated press did nothing"
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
#[cfg(test)]
mod vector_pack_tests {
    use super::*;

    /// The lane map, held to code. `begin_frame` writes each parameter
    /// into a specific component of a specific vec4; getting one wrong
    /// does not fail — it makes a knob move the wrong thing, which on
    /// stage reads as "the app is haunted". Distinctive values in, exact
    /// lanes out.
    #[test]
    fn vector_packing_puts_each_parameter_in_its_lane() {
        let params = std::sync::Arc::new(crate::params::AppParams::build());
        let p = &*params;
        let l3 = p.vector_layers[2];
        p.registry.set(l3.kind, 5.0);
        p.registry.set(l3.freq, 23.0);
        p.registry.set(l3.blend, 4.0);
        p.registry.set(l3.opacity, 0.75);
        p.registry.set(p.vector_palette[2][1], 0.33);

        let mut engine = FrameEngine::new(
            std::sync::Arc::clone(&params),
            vizz_audio::AudioEngine::start(Some("\0none")),
        );
        // Two long steps so the smoothed params reach their targets.
        engine.begin_frame(16.0 / 9.0, Some(std::time::Duration::from_secs(5)));
        let inputs = engine.begin_frame(16.0 / 9.0, Some(std::time::Duration::from_secs(5)));

        let l = &inputs.vector.layers[2];
        assert_eq!(l.pat[0], 5.0, "kind lane");
        assert!((l.pat[1] - 23.0).abs() < 0.05, "freq lane: {}", l.pat[1]);
        assert_eq!(l.style[0], 4.0, "blend lane");
        assert!((l.style[1] - 0.75).abs() < 0.02, "opacity lane: {}", l.style[1]);
        assert!(
            (inputs.vector.palette[2][1] - 0.33).abs() < 0.02,
            "palette lane: {}",
            inputs.vector.palette[2][1]
        );
        assert!(inputs.vector_active, "a layer with a kind is an active stack");

        // And the guarantee the render order depends on: everything at
        // defaults means inactive, so the pass is skipped and the frame
        // is byte-identical to the pre-vector app.
        let fresh = std::sync::Arc::new(crate::params::AppParams::build());
        let mut engine = FrameEngine::new(
            std::sync::Arc::clone(&fresh),
            vizz_audio::AudioEngine::start(Some("\0none")),
        );
        let inputs = engine.begin_frame(16.0 / 9.0, Some(std::time::Duration::from_secs(1)));
        assert!(!inputs.vector_active, "defaults must leave the stack off");
    }
}
