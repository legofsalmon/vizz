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
    pub room_converge: ParamId,
    pub room_vanish_x: ParamId,
    pub room_vanish_y: ParamId,
    pub room_anchor: ParamId,
    pub room_embed: ParamId,
    pub cam_pan_x: ParamId,
    pub cam_pan_y: ParamId,
    pub bg_r: ParamId,
    pub bg_g: ParamId,
    pub bg_b: ParamId,
    pub bg_a: ParamId,
    pub preset_recall: ParamId,
    pub scene_fire: ParamId,
    pub scene_time: ParamId,
    pub scene_curve: ParamId,
    pub scene_auto: ParamId,
    pub scene_bars: ParamId,
}

pub const MAX_PARTICLES: f32 = 500_000.0;

/// Highest slot `/preset/recall` will address. Slot 0 is "none" and
/// presets run from 1, so this is one more than the number of presets
/// reachable. Fixed rather than sized from the preset list, because the
/// parameter set is built once at startup and saving a preset must not
/// reshape the registry underneath a running show.
pub const MAX_PRESET_SLOT: f32 = 64.0;

/// Highest slot `/scene/fire` will address. Slot 0 is "none" and the grid
/// runs from 1, exactly as preset recall does, so a control resting at
/// zero cannot fire a scene on the first frame.
pub const SCENE_SLOTS: f32 = vizz_mod::scene::SLOTS as f32;

impl AppParams {
    pub fn build() -> Self {
        let mut b = ParamRegistry::builder();
        let count =
            b.add(ParamDef::new("/particles/count", 0.0, MAX_PARTICLES, 60_000.0).smooth(0.2));
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
        let shape = b.add(
            ParamDef::new("/shape/mode", 0.0, 8.0, 0.0)
                .smooth(0.4)
                // The sweep wraps, so 8 is the sphere again coming round.
                .labels(&[
                    "sphere",
                    "torus",
                    "knot",
                    "grid",
                    "shell",
                    "Lorenz",
                    "Aizawa",
                    "cloud pair",
                    "sphere",
                ]),
        );
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
        let mirror =
            b.add(ParamDef::new("/fx/mirror", 0.0, 3.0, 0.0).labels(&["off", "x", "y", "quad"]));
        let glow = b.add(ParamDef::new("/fx/glow", 0.0, 1.0, 0.25).smooth(0.2));
        // Chromatic aberration. Subtle at the low end, prismatic at the top.
        let shift = b.add(ParamDef::new("/fx/shift", 0.0, 1.0, 0.0).smooth(0.2));
        // Colour. Palette 0 is the original HSV behaviour, so the defaults
        // below leave the look exactly as it was.
        // The range covers the loaded palettes as well as the shipped
        // ones. Indices 0..=4 are fixed forever — a preset saved with
        // palette 3 must still be "ice" in every future build, and a saved
        // patch is the one thing that cannot be migrated after the fact —
        // so anything loaded lands above them.
        //
        // Labelled only as far as the built-ins go; past that the label
        // would have to be the name of whatever happens to be loaded,
        // which is not something a static table can know.
        let palette = b.add(
            ParamDef::new(
                "/color/palette",
                0.0,
                (vizz_render::palette::PALETTES - 1) as f32,
                0.0,
            )
            .smooth(0.4)
            .labels(&["hsv", "warm", "ember", "ice", "neon"]),
        );
        let color_spread = b.add(ParamDef::new("/color/spread", 0.0, 1.0, 0.12).smooth(0.3));
        // Stepped: these are four different ideas, not a sweep.
        let color_drive = b.add(
            ParamDef::new("/color/drive", 0.0, 3.0, 0.0)
                .labels(&["index", "radius", "depth", "height"]),
        );
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
        // Pan, in the camera's own screen plane. The range is roughly the
        // width of the field at the default distance: enough to push the
        // subject fully out of frame, which is a legitimate move, without
        // a fader whose useful travel is the middle two percent.
        let cam_pan_x = b.add(ParamDef::new("/camera/pan_x", -4.0, 4.0, 0.0).smooth(0.4));
        let cam_pan_y = b.add(ParamDef::new("/camera/pan_y", -4.0, 4.0, 0.0).smooth(0.4));
        // Room. Off by default: it is a strong look, not a neutral one.
        let room = b.add(ParamDef::new("/room/brightness", 0.0, 1.0, 0.0).smooth(0.3));
        let room_depth = b.add(ParamDef::new("/room/depth", 1.0, 20.0, 7.0).smooth(0.4));
        let room_fade = b.add(ParamDef::new("/room/fade", 0.0, 1.0, 0.75).smooth(0.3));
        // The room's own angle of view, separate from the lens. 1.0 is a
        // parallel-walled box, 0 collapses the far end to a point; a stage
        // set built out of physical scenery lives somewhere in between.
        // Having it apart from /camera/fov is the whole trick — the lens
        // decides what the frame contains, this decides how deep it feels.
        let room_converge = b.add(ParamDef::new("/room/converge", 0.0, 1.0, 0.35).smooth(0.4));
        // Where the far end sits, in units of the opening's half-size. The
        // opening never moves, so pushing these off centre skews the room
        // without ever unsticking it from the frame edge.
        let room_vanish_x = b.add(ParamDef::new("/room/vanish_x", -1.0, 1.0, 0.0).smooth(0.4));
        let room_vanish_y = b.add(ParamDef::new("/room/vanish_y", -1.0, 1.0, 0.0).smooth(0.4));
        // Where the field sits along the room's depth, and how much it
        // belongs to the room. Embed is 0 by default so switching the room
        // on never moves the cloud — see room.rs.
        let room_anchor = b.add(ParamDef::new("/room/anchor", 0.0, 1.0, 0.35).smooth(0.4));
        let room_embed = b.add(ParamDef::new("/room/embed", 0.0, 1.0, 0.0).smooth(0.4));
        // The background. Defaults match the near-black the renderer has
        // always cleared to, so this is invisible until someone reaches
        // for it.
        //
        // Alpha is the interesting one. At 0 the field is delivered on a
        // transparent background, which is what lets vizz be a layer in
        // Resolume or VDMX rather than a whole picture. It is a parameter
        // like everything else, so it blends across a scene change and can
        // be pulled on a fader — fading the background out from under a
        // look is a transition in its own right.
        let bg_r = b.add(ParamDef::new("/bg/red", 0.0, 1.0, 0.004).smooth(0.3));
        let bg_g = b.add(ParamDef::new("/bg/green", 0.0, 1.0, 0.004).smooth(0.3));
        let bg_b = b.add(ParamDef::new("/bg/blue", 0.0, 1.0, 0.008).smooth(0.3));
        let bg_a = b.add(ParamDef::new("/bg/alpha", 0.0, 1.0, 1.0).smooth(0.3));
        // Preset recall by slot: 0 selects nothing, 1 is the first preset.
        // Unsmoothed on
        // purpose: a smoothed value glides through every index between
        // where it was and where it is going, firing each preset on the
        // way. Being an ordinary parameter is what gets it MIDI learn and
        // OSC for free.
        let preset_recall = b.add(ParamDef::new("/preset/recall", 0.0, MAX_PRESET_SLOT, 0.0));
        // The scene grid. Parameters rather than plain settings for the
        // same reason recall is one: a pad controller addresses them for
        // free, and there is one path to firing a scene rather than a UI
        // path and a control path that can drift apart.
        //
        // Unsmoothed, all of them: a glided fire sweeps through every slot
        // between here and there, firing each on the way.
        let scene_fire = b.add(ParamDef::new("/scene/fire", 0.0, SCENE_SLOTS, 0.0));
        // Transition length. Zero is a cut, which is why the range starts
        // there rather than at some minimum that would put cuts out of a
        // fader's reach.
        let scene_time = b.add(ParamDef::new("/scene/time", 0.0, 30.0, 2.0));
        let scene_curve = b.add(
            ParamDef::new("/scene/curve", 0.0, 4.0, 1.0)
                .labels(&["linear", "smooth", "ease in", "ease out", "cut"]),
        );
        let scene_auto = b.add(ParamDef::new("/scene/auto", 0.0, 1.0, 0.0).labels(&["off", "on"]));
        // Bars between autopilot steps. Down to a quarter bar, because a
        // scene change on every beat is a legitimate effect and a minimum
        // of one bar would rule it out.
        let scene_bars = b.add(ParamDef::new("/scene/bars", 0.25, 16.0, 4.0));
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
            room_converge,
            room_vanish_x,
            room_vanish_y,
            room_anchor,
            room_embed,
            cam_pan_x,
            cam_pan_y,
            bg_r,
            bg_g,
            bg_b,
            bg_a,
            preset_recall,
            scene_fire,
            scene_time,
            scene_curve,
            scene_auto,
            scene_bars,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in presets are hand-written address tables in another
    /// crate, so nothing but a test connects them to the real parameter
    /// set. A typo'd address is not a compile error — it is a preset that
    /// silently does less than it claims, which you would only notice by
    /// recalling it and squinting.
    #[test]
    fn every_builtin_preset_names_real_parameters_in_range() {
        let p = AppParams::build();
        for b in vizz_mod::preset::BUILTINS {
            for (addr, value) in b.values {
                let id = p
                    .registry
                    .id(addr)
                    .unwrap_or_else(|| panic!("preset {:?}: no such parameter {addr}", b.name));
                let def = &p.registry.defs()[id.index()];
                assert!(
                    (def.min..=def.max).contains(value),
                    "preset {:?}: {addr} = {value} outside {}..{}",
                    b.name,
                    def.min,
                    def.max
                );
            }
        }
    }

    /// A preset must be able to address everything worth recalling. If a
    /// parameter is added and deliberately kept out of presets, it belongs
    /// in `preset::EXCLUDED` with a reason, not left to chance.
    #[test]
    fn only_the_documented_parameters_are_excluded_from_presets() {
        let p = AppParams::build();
        for addr in vizz_mod::preset::EXCLUDED {
            assert!(
                p.registry.id(addr).is_some(),
                "EXCLUDED names {addr}, which is not a parameter"
            );
        }
        assert_eq!(
            vizz_mod::preset::EXCLUDED,
            &[
                "/master/dim",
                "/preset/recall",
                "/scene/fire",
                "/scene/time",
                "/scene/curve",
                "/scene/auto",
                "/scene/bars",
            ]
        );
    }

    /// A scene cell is a captured preset, so anything that fires or shapes
    /// a scene has to be excluded or the grid feeds itself. This is the
    /// test that catches a `/scene/*` parameter added later and not
    /// excluded — the failure mode there is a cell that re-fires on
    /// arrival, which is a hung show rather than a wrong colour.
    #[test]
    fn nothing_that_drives_the_grid_is_stored_in_a_scene() {
        let p = AppParams::build();
        for (_, def) in p.registry.iter() {
            if def.addr.starts_with("/scene/") {
                assert!(
                    vizz_mod::preset::EXCLUDED.contains(&def.addr.as_str()),
                    "{} would be captured into a scene cell",
                    def.addr
                );
            }
        }
    }

    /// Firing must rest at "nothing selected" and must not glide, for the
    /// same two reasons recall must: a fresh start cannot fire slot 0 over
    /// your defaults, and a smoothed fire sweeps through every slot in
    /// between, firing each on the way.
    #[test]
    fn scene_fire_rests_at_nothing_and_does_not_glide() {
        let p = AppParams::build();
        let def = &p.registry.defs()[p.scene_fire.index()];
        assert_eq!(def.default, 0.0);
        assert_eq!(def.min, 0.0);
        assert_eq!(def.smooth, 0.0);
        assert_eq!(
            def.max,
            vizz_mod::scene::SLOTS as f32,
            "not every pad is reachable"
        );
    }

    /// Recall must reach every preset the app can list. The range is
    /// fixed at startup while the list grows on disk, so the two can drift
    /// apart — and a preset you cannot address is invisible to MIDI.
    #[test]
    fn recall_range_covers_the_builtins_with_room_to_spare() {
        // Slot 0 is "none", so N built-ins need slots up to N.
        let builtins = vizz_mod::preset::BUILTINS.len() as f32;
        assert!(
            MAX_PRESET_SLOT >= builtins,
            "recall tops out at {MAX_PRESET_SLOT} but there are {builtins} built-ins"
        );
        assert!(
            MAX_PRESET_SLOT >= builtins + 16.0,
            "no headroom for user presets"
        );
    }

    /// Every default fader assignment must name a real parameter.
    ///
    /// The shipped macro list lives in another crate, which cannot see this
    /// registry, so a typo or a renamed address there produces a fader that
    /// draws as an empty slot on a fresh install and is silent about why.
    /// This is the only place both halves are visible at once.
    #[test]
    fn every_default_macro_names_a_real_parameter() {
        let p = AppParams::build();
        let macros = vizz_mod::perform::Macros::default();
        for (slot, addr) in macros.slots.iter().enumerate() {
            let Some(addr) = addr else { continue };
            assert!(
                p.registry.id(addr).is_some(),
                "default fader {slot} points at {addr}, which is not a parameter"
            );
        }
    }

    /// Slot 0 must select nothing. It is the resting value, so anything
    /// else means a fresh start recalls a preset over the defaults.
    #[test]
    fn slot_zero_is_reserved_for_nothing_selected() {
        let p = AppParams::build();
        let def = &p.registry.defs()[p.preset_recall.index()];
        assert_eq!(def.default, 0.0, "recall must rest at the empty slot");
        assert_eq!(def.min, 0.0);
        assert_eq!(
            def.smooth, 0.0,
            "a smoothed recall glides through every slot on the way"
        );
    }
}
