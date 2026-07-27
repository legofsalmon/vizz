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
    snapshot: ParamSnapshot,
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
}

pub struct FrameInputs {
    pub uniforms: Uniforms,
    pub post: PostUniforms,
    pub room: RoomUniforms,
    /// Skip the room pass entirely when it is dark — it is off by default,
    /// and drawing invisible lines every frame is wasted work.
    pub room_visible: bool,
    pub count: u32,
}

impl FrameEngine {
    pub fn new(params: Arc<AppParams>, audio: AudioEngine) -> Self {
        Self {
            snapshot: ParamSnapshot::new(&params.registry),
            params,
            health: HealthMonitor::new(HealthConfig::default()),
            modulation: ModEngine::with_defaults(),
            audio,
            bands: [0.0; BAND_COUNT],
            vis_time: 0.0,
            last_frame: None,
            last_log: Instant::now(),
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
        let p = &self.params;
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
        let levels = AudioLevels { bands: &self.bands, level: self.audio.state.level() };
        let offsets = self.modulation.tick(dt_s, &p.registry, levels);
        self.snapshot.advance_modulated(&p.registry, dt_s, offsets);
        self.vis_time += dt_s * self.snapshot.get(p.speed);

        let brightness = self.snapshot.get(p.brightness) * self.snapshot.get(p.dim);

        let camera = Camera {
            distance: self.snapshot.get(p.cam_dist),
            orbit: self.snapshot.get(p.cam_orbit),
            elevation: self.snapshot.get(p.cam_elev),
            fov: self.snapshot.get(p.cam_fov),
            aspect,
            focus: self.snapshot.get(p.cam_focus),
            defocus: self.snapshot.get(p.cam_defocus),
        };
        let cam = camera.uniforms();
        let room_brightness = self.snapshot.get(p.room);

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
                // Slot choice is stepped; the morph between them is not.
                cloud_a: self.snapshot.get(p.cloud_a).round(),
                cloud_b: self.snapshot.get(p.cloud_b).round(),
                cloud_morph: self.snapshot.get(p.cloud_morph),
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
            // The opening sits a little in front of the origin so the cloud
            // is inside the room rather than pressed against its face.
            room: RoomUniforms::for_camera(
                &camera,
                (camera.distance - 1.6).max(0.3),
                self.snapshot.get(p.room_depth),
                room_brightness,
                self.snapshot.get(p.room_fade),
            ),
            room_visible: room_brightness > 0.002,
            count: self.snapshot.get(p.count).max(0.0) as u32,
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
