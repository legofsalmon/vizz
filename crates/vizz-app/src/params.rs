//! The app's parameter set: every OSC-controllable value and its range.
//! This table is the single source of truth — the README's OSC reference
//! is generated from what's registered here.

use std::sync::Arc;

use vizz_params::{ParamDef, ParamId, ParamRegistry};

pub struct AppParams {
    /// Shared with control threads (OSC now, MIDI/UI later).
    pub registry: Arc<ParamRegistry>,
    pub count: ParamId,
    pub size: ParamId,
    pub speed: ParamId,
    pub spread: ParamId,
    pub hue: ParamId,
    pub saturation: ParamId,
    pub brightness: ParamId,
    pub dim: ParamId,
    pub shape: ParamId,
    pub morph: ParamId,
    pub twist: ParamId,
}

pub const MAX_PARTICLES: f32 = 500_000.0;

impl AppParams {
    pub fn build() -> Self {
        let mut b = ParamRegistry::builder();
        let count = b.add(ParamDef::new("/particles/count", 0.0, MAX_PARTICLES, 60_000.0).smooth(0.2));
        let size = b.add(ParamDef::new("/particles/size", 0.001, 0.2, 0.015).smooth(0.1));
        let speed = b.add(ParamDef::new("/particles/speed", 0.0, 4.0, 0.6).smooth(0.25));
        let spread = b.add(ParamDef::new("/particles/spread", 0.05, 3.0, 1.2).smooth(0.3));
        let hue = b.add(ParamDef::new("/particles/hue", 0.0, 1.0, 0.58).smooth(0.15));
        let saturation = b.add(ParamDef::new("/particles/saturation", 0.0, 1.0, 0.8).smooth(0.15));
        let brightness = b.add(ParamDef::new("/particles/brightness", 0.0, 2.0, 1.0).smooth(0.1));
        // Geometry: sphere, torus, knot, grid, shell. Fractional values
        // sit between two forms, so this is a sweep, not a switch.
        let shape = b.add(ParamDef::new("/shape/mode", 0.0, 5.0, 0.0).smooth(0.4));
        let morph = b.add(ParamDef::new("/shape/morph", 0.0, 1.0, 0.0).smooth(0.3));
        let twist = b.add(ParamDef::new("/shape/twist", 0.0, 2.0, 0.0).smooth(0.25));
        // Master dim is the "oh no" fader: fast but still click-free.
        let dim = b.add(ParamDef::new("/master/dim", 0.0, 1.0, 1.0).smooth(0.05));
        Self {
            registry: Arc::new(b.build()),
            count,
            size,
            speed,
            spread,
            hue,
            saturation,
            brightness,
            dim,
            shape,
            morph,
            twist,
        }
    }
}
