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
    pub cloud_a: ParamId,
    pub cloud_b: ParamId,
    pub cloud_morph: ParamId,
    pub cam_dist: ParamId,
    pub cam_orbit: ParamId,
    pub cam_elev: ParamId,
    pub cam_fov: ParamId,
    pub cam_focus: ParamId,
    pub cam_defocus: ParamId,
    pub room: ParamId,
    pub room_depth: ParamId,
    pub room_fade: ParamId,
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
        // Geometry: sphere, torus, knot, grid, shell, Lorenz, Aizawa,
        // cloud pair.
        // Fractional values sit between two forms, so this is a sweep, not
        // a switch — and it wraps, so the top of the range morphs the
        // Aizawa attractor back into the sphere.
        let shape = b.add(ParamDef::new("/shape/mode", 0.0, 8.0, 0.0).smooth(0.4));
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
        // Point-cloud pair. Slot choice is stepped — half a slot is not a
        // cloud — while the morph between them is the swept, modulatable
        // control, which is what makes it worth having separately from the
        // shape sweep (that one only reaches *adjacent* modes).
        let cloud_a = b.add(ParamDef::new("/cloud/a", 0.0, 3.0, 0.0));
        let cloud_b = b.add(ParamDef::new("/cloud/b", 0.0, 3.0, 1.0));
        let cloud_morph = b.add(ParamDef::new("/cloud/morph", 0.0, 1.0, 0.0).smooth(0.5));
        // Camera. Distance and field of view are two different kinds of
        // zoom — moving closer changes the perspective, narrowing the lens
        // does not — so both are exposed rather than conflated.
        let cam_dist = b.add(ParamDef::new("/camera/distance", 0.4, 12.0, 3.5).smooth(0.4));
        let cam_orbit = b.add(ParamDef::new("/camera/orbit", -3.15, 3.15, 0.0).smooth(0.4));
        let cam_elev = b.add(ParamDef::new("/camera/elevation", -1.4, 1.4, 0.34).smooth(0.4));
        let cam_fov = b.add(ParamDef::new("/camera/fov", 0.2, 2.0, 0.9).smooth(0.4));
        // Focus is a distance, so its useful range tracks the camera's.
        let cam_focus = b.add(ParamDef::new("/camera/focus", 0.0, 12.0, 3.5).smooth(0.4));
        let cam_defocus = b.add(ParamDef::new("/camera/defocus", 0.0, 1.0, 0.0).smooth(0.3));
        // Room. Off by default: it is a strong look, not a neutral one.
        let room = b.add(ParamDef::new("/room/brightness", 0.0, 1.0, 0.0).smooth(0.3));
        let room_depth = b.add(ParamDef::new("/room/depth", 1.0, 20.0, 7.0).smooth(0.4));
        let room_fade = b.add(ParamDef::new("/room/fade", 0.0, 1.0, 0.75).smooth(0.3));
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
            cloud_a,
            cloud_b,
            cloud_morph,
            cam_dist,
            cam_orbit,
            cam_elev,
            cam_fov,
            cam_focus,
            cam_defocus,
            room,
            room_depth,
            room_fade,
        }
    }
}
