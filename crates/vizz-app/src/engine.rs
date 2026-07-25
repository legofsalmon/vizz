//! Frame engine: everything one frame needs that isn't the GPU itself.
//! Shared verbatim between windowed and headless modes so a benchmark run
//! measures the same code a live set executes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use vizz_health::{HealthConfig, HealthMonitor, HealthSnapshot};
use vizz_params::ParamSnapshot;
use vizz_render::particles::Uniforms;

use crate::params::AppParams;

pub struct FrameEngine {
    params: Arc<AppParams>,
    snapshot: ParamSnapshot,
    pub health: HealthMonitor,
    /// Visual time, pre-integrated so `/particles/speed` changes modulate
    /// the rate without jumping the phase.
    vis_time: f32,
    last_frame: Option<Instant>,
    last_log: Instant,
}

pub struct FrameInputs {
    pub uniforms: Uniforms,
    pub count: u32,
}

impl FrameEngine {
    pub fn new(params: Arc<AppParams>) -> Self {
        Self {
            snapshot: ParamSnapshot::new(&params.registry),
            params,
            health: HealthMonitor::new(HealthConfig::default()),
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
        self.snapshot.advance(&p.registry, dt_s);
        self.vis_time += dt_s * self.snapshot.get(p.speed);

        let brightness = self.snapshot.get(p.brightness) * self.snapshot.get(p.dim);
        FrameInputs {
            uniforms: Uniforms {
                time: self.vis_time,
                aspect,
                size: self.snapshot.get(p.size),
                spread: self.snapshot.get(p.spread),
                hue: self.snapshot.get(p.hue),
                saturation: self.snapshot.get(p.saturation),
                brightness,
                _pad: 0.0,
            },
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
