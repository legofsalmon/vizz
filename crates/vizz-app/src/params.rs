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
    pub trail: ParamId,
    pub zoom: ParamId,
    pub spin: ParamId,
    pub mirror: ParamId,
    pub glow: ParamId,
    pub shift: ParamId,
    pub palette: ParamId,
    pub color_spread: ParamId,
    pub color_drive: ParamId,
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
        // Geometry: sphere, torus, knot, grid, shell, Lorenz, Aizawa.
        // Fractional values sit between two forms, so this is a sweep, not
        // a switch — and it wraps, so the top of the range morphs the
        // Aizawa attractor back into the sphere.
        let shape = b.add(ParamDef::new("/shape/mode", 0.0, 7.0, 0.0).smooth(0.4));
        let morph = b.add(ParamDef::new("/shape/morph", 0.0, 1.0, 0.0).smooth(0.3));
        let twist = b.add(ParamDef::new("/shape/twist", 0.0, 2.0, 0.0).smooth(0.25));
        // Feedback: the effect that turns a particle field into VJ
        // material. Capped below 1.0 because at 1.0 nothing ever decays
        // and the frame saturates to white within seconds.
        let trail = b.add(ParamDef::new("/fx/trail", 0.0, 0.98, 0.0).smooth(0.2));
        // Per-frame zoom of the history. Around 1.0 is still; away from it
        // in either direction builds a tunnel.
        let zoom = b.add(ParamDef::new("/fx/zoom", 0.9, 1.1, 1.0).smooth(0.3));
        let spin = b.add(ParamDef::new("/fx/spin", -0.1, 0.1, 0.0).smooth(0.3));
        // Stepped, not swept: half a mirror is not a look.
        let mirror = b.add(ParamDef::new("/fx/mirror", 0.0, 3.0, 0.0));
        let glow = b.add(ParamDef::new("/fx/glow", 0.0, 1.0, 0.25).smooth(0.2));
        // Chromatic aberration. Subtle at the low end, prismatic at the top.
        let shift = b.add(ParamDef::new("/fx/shift", 0.0, 1.0, 0.0).smooth(0.2));
        // Colour. Palette 0 is the original HSV behaviour, so the defaults
        // below leave the look exactly as it was.
        let palette = b.add(ParamDef::new("/color/palette", 0.0, 4.0, 0.0).smooth(0.4));
        let color_spread = b.add(ParamDef::new("/color/spread", 0.0, 1.0, 0.12).smooth(0.3));
        // Stepped: these are four different ideas, not a sweep.
        let color_drive = b.add(ParamDef::new("/color/drive", 0.0, 3.0, 0.0));
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
            trail,
            zoom,
            spin,
            mirror,
            glow,
            shift,
            palette,
            color_spread,
            color_drive,
        }
    }
}
