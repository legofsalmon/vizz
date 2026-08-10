//! The app's parameter set: every OSC-controllable value and its range.
//! This table is the single source of truth — the README's OSC reference
//! is generated from what's registered here.

use std::sync::Arc;

use vizz_params::{ParamDef, ParamId, ParamRegistry};

/// Gravity wells in the shader's uniform block. Four is what the block
/// has room for without growing it, and is already more than anyone can
/// place by hand mid-set.
pub const GRAVITY_WELLS: usize = 4;

/// The `/shape/mode` position that shows the cloud pair.
///
/// Named because loading a cloud has to point the shape at it — a cloud
/// that arrives while the shape is still on `sphere` is invisible, and
/// the load reads as having done nothing. A bare `7.0` at that call site
/// says nothing about why, and would not survive the shape list gaining
/// a form before this one; the test below holds it to the label.
pub const SHAPE_CLOUD_PAIR: f32 = 7.0;

/// The parameter ids for one vector layer.
#[derive(Debug, Clone, Copy)]
pub struct VectorLayer {
    pub kind: ParamId,
    pub freq: ParamId,
    pub phase: ParamId,
    pub duty: ParamId,
    pub sides: ParamId,
    pub inset: ParamId,
    pub fold: ParamId,
    pub invert: ParamId,
    pub x: ParamId,
    pub y: ParamId,
    pub rot: ParamId,
    pub scale: ParamId,
    pub color: ParamId,
    pub blend: ParamId,
    pub opacity: ParamId,
}

/// Vector layers the registry exposes. The shader holds
/// [`vizz_render::vector::MAX_LAYERS`] — capacity there is free, but
/// every layer here is fifteen parameters in the panel, the tables and
/// the presets, so the surface starts smaller than the ceiling.
pub const VECTOR_LAYERS: usize = 4;

/// The parameter ids for one well.
#[derive(Debug, Clone, Copy)]
pub struct GravityWell {
    pub x: ParamId,
    pub y: ParamId,
    pub z: ParamId,
    pub strength: ParamId,
    pub radius: ParamId,
}

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
    pub punch_flash: ParamId,
    pub punch_black: ParamId,
    pub punch_invert: ParamId,
    pub punch_freeze: ParamId,
    pub punch_strobe: ParamId,
    pub punch_strobe_div: ParamId,
    pub record_active: ParamId,
    pub palette: ParamId,
    pub color_spread: ParamId,
    pub color_drive: ParamId,
    pub cloud_a: ParamId,
    pub cloud_b: ParamId,
    pub cloud_morph: ParamId,
    pub video_depth: ParamId,
    pub video_relief: ParamId,
    pub vector_layers: Vec<VectorLayer>,
    pub vector_palette: Vec<[ParamId; 3]>,
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
    pub gravity_amount: ParamId,
    pub gravity: Vec<GravityWell>,
    pub gravity_fire: ParamId,
    pub gravity_time: ParamId,
    pub gravity_curve: ParamId,
    pub gravity_auto: ParamId,
    pub gravity_bars: ParamId,
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

/// Longest blend either grid will accept.
///
/// Taken from the grid rather than typed here: the two disagreed —
/// registered at 30 while `Grid` clamped at 60 — so a grid file saved with
/// a longer blend was silently shortened on load, with nothing to say so.
pub const MAX_BLEND: f32 = vizz_mod::scene::MAX_DURATION;

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
        // Names taken from what `fold()` in post.wgsl actually does. They
        // used to read ["off","x","y","quad"], which was shifted one step:
        // there is no y-only mirror in the shader, so reaching for "quad"
        // gave a kaleidoscope. This is a default macro, so the wrong label
        // was on the performance screen during every set.
        let mirror = b.add(
            ParamDef::new("/fx/mirror", 0.0, 3.0, 0.0)
                .labels(&["off", "mirror", "quad", "kaleido"]),
        );
        let glow = b.add(ParamDef::new("/fx/glow", 0.0, 1.0, 0.25).smooth(0.2));
        // Chromatic aberration. Subtle at the low end, prismatic at the top.
        let shift = b.add(ParamDef::new("/fx/shift", 0.0, 1.0, 0.0).smooth(0.2));
        // Punch effects: the things you do on the drop. All gestures —
        // resting at zero, never smoothed (a flash that fades in is not a
        // flash), excluded from presets (recalling a look must not replay
        // a blackout) — and all still modulatable, because a strobe under
        // an audio band is a legitimate patch. Resting at min matters
        // twice over: it is also what makes a MIDI value binding behave
        // as a momentary out of the box — press sends the value, release
        // sends the bottom of the range, and the bottom is "off".
        let punch_flash = b.add(ParamDef::new("/punch/flash", 0.0, 1.0, 0.0).gesture());
        let punch_black = b.add(ParamDef::new("/punch/black", 0.0, 1.0, 0.0).gesture());
        let punch_invert = b.add(ParamDef::new("/punch/invert", 0.0, 1.0, 0.0).gesture());
        let punch_freeze = b.add(
            ParamDef::new("/punch/freeze", 0.0, 1.0, 0.0)
                .labels(&["off", "hold"])
                .gesture(),
        );
        let punch_strobe = b.add(ParamDef::new("/punch/strobe", 0.0, 1.0, 0.0).gesture());
        // Beats per strobe cycle. Transport, like the scene blend time:
        // it says *how* the gesture behaves, not what anything looks
        // like — a preset must not retune the strobe someone is holding,
        // and modulation wobbling the division would make the strobe
        // untrustworthy. Set from its own control beside the button.
        let punch_strobe_div =
            b.add(ParamDef::new("/punch/strobe_div", 0.25, 4.0, 0.5).transport());
        // Recording is transport all the way: it says when, modulation
        // must never toggle disk writes, and a preset recalling with a
        // recording embedded would start one behind your back.
        let record_active = b.add(
            ParamDef::new("/record/active", 0.0, 1.0, 0.0)
                .labels(&["off", "rec"])
                .transport(),
        );
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
        // The ceiling is the bank's, derived rather than retyped, so
        // growing the bank cannot leave the top slots unreachable.
        let cloud_max = (vizz_render::attractor::SLOTS - 1) as f32;
        let cloud_a = b.add(ParamDef::new("/cloud/a", 0.0, cloud_max, 0.0));
        let cloud_b = b.add(ParamDef::new("/cloud/b", 0.0, cloud_max, 1.0));
        let cloud_morph = b.add(ParamDef::new("/cloud/morph", 0.0, 1.0, 0.0).smooth(0.5));
        // Live video, which arrives as a cloud in its own slot. Depth is
        // signed so the relief can be pushed either way from the plane —
        // a picture standing proud of it or sunk into it are different
        // looks, and zero is the flat picture, which is a look too.
        let video_depth = b.add(ParamDef::new("/video/depth", -2.0, 2.0, 0.6).smooth(0.3));
        // What the picture pushes with. Stepped: these are four different
        // readings of a frame, not points on one scale, so sweeping
        // between them would spend the move showing neither.
        let video_relief = b.add(
            ParamDef::new("/video/relief", 0.0, 3.0, 0.0)
                .labels(&["luminance", "hue", "saturation", "chroma"]),
        );
        // Vector layers: the hard-edged print-look counterpart to the
        // particle field, drawn behind it. Kind 0 is "off", which is
        // both the default and the way a controller removes a layer —
        // so a fresh launch renders exactly what it did before these
        // existed. Kind and blend are switches (unsmoothed + labelled),
        // which is also what makes scene transitions flip them at the
        // midpoint instead of sweeping through modes neither look asked
        // for. Sides is a sweep on purpose: the polygon SDF is
        // continuous in it, and a triangle melting into a hexagon is a
        // move worth having.
        let vector_layers: Vec<VectorLayer> = (1..=VECTOR_LAYERS)
            .map(|i| {
                let a = |name: &str| format!("/l{i}/{name}");
                VectorLayer {
                    kind: b.add(
                        ParamDef::new(a("kind"), 0.0, 7.0, 0.0)
                            .labels(vizz_render::vector::KIND_LABELS),
                    ),
                    freq: b.add(ParamDef::new(a("freq"), 0.5, 64.0, 8.0).smooth(0.15)),
                    // Unsmoothed: phase is the strobe/step control, and
                    // a phase that glides is a pattern that drifts when
                    // it was told to jump.
                    phase: b.add(ParamDef::new(a("phase"), 0.0, 1.0, 0.0)),
                    duty: b.add(ParamDef::new(a("duty"), 0.05, 0.95, 0.5).smooth(0.1)),
                    sides: b.add(ParamDef::new(a("sides"), 2.0, 16.0, 4.0).smooth(0.2)),
                    inset: b.add(ParamDef::new(a("inset"), 0.0, 1.0, 0.5).smooth(0.15)),
                    fold: b.add(ParamDef::new(a("fold"), 0.0, 12.0, 0.0)),
                    invert: b.add(
                        ParamDef::new(a("invert"), 0.0, 1.0, 0.0).labels(&["fill", "invert"]),
                    ),
                    x: b.add(ParamDef::new(a("x"), -2.0, 2.0, 0.0).smooth(0.2)),
                    y: b.add(ParamDef::new(a("y"), -2.0, 2.0, 0.0).smooth(0.2)),
                    rot: b.add(ParamDef::new(a("rot"), -2.0, 2.0, 0.0).smooth(0.2)),
                    scale: b.add(ParamDef::new(a("scale"), 0.05, 8.0, 1.0).smooth(0.2)),
                    color: b.add(
                        ParamDef::new(a("color"), 0.0, 3.0, 0.0)
                            .labels(&["ink 1", "ink 2", "ink 3", "ink 4"]),
                    ),
                    blend: b.add(
                        ParamDef::new(a("blend"), 0.0, 6.0, 0.0)
                            .labels(vizz_render::vector::BLEND_LABELS),
                    ),
                    opacity: b.add(ParamDef::new(a("opacity"), 0.0, 1.0, 1.0).smooth(0.1)),
                }
            })
            .collect();
        // The four inks, shared by every layer. A small fixed palette is
        // the discipline this look is built on: per-layer free colour
        // invites mud, four inks invite a print. Defaults match
        // `StackU::default()` so the two ways of reaching the renderer
        // agree about what "untouched" looks like.
        let ink_defaults: [[f32; 3]; 4] = [
            [0.05, 0.05, 0.05],
            [0.92, 0.10, 0.14],
            [0.10, 0.30, 0.95],
            [0.98, 0.80, 0.05],
        ];
        let vector_palette: Vec<[ParamId; 3]> = (0..vizz_render::vector::PALETTE_SLOTS)
            .map(|i| {
                let mut add = |j: usize, ch: &str| {
                    b.add(
                        ParamDef::new(format!("/pal/{i}/{ch}"), 0.0, 1.0, ink_defaults[i][j])
                            .smooth(0.1),
                    )
                };
                [add(0, "r"), add(1, "g"), add(2, "b")]
            })
            .collect();
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
        // Gravity: four wells that bend the cloud around them.
        //
        // A layer over the shape rather than part of it. The shape decides
        // what the field *is*; a well decides what happens to it on the
        // way to the screen, which is why these are their own group and
        // why they blend independently of the geometry.
        //
        // `amount` is the way in and out. Every well can be dialled in
        // advance and the whole layer brought up on one fader, which is
        // the control you actually want mid-set — reaching for four
        // strengths at once is not playable.
        let gravity_amount = b.add(ParamDef::new("/gravity/amount", 0.0, 1.0, 0.0).smooth(0.4));
        let mut gravity = Vec::with_capacity(GRAVITY_WELLS);
        for i in 0..GRAVITY_WELLS {
            // Positions span rather more than the field's own extent, so a
            // well can sit outside the cloud and pull it sideways — which
            // is a different and more useful move than one buried in the
            // middle of it.
            let x = b.add(ParamDef::new(format!("/gravity/{i}/x"), -3.0, 3.0, 0.0).smooth(0.4));
            let y = b.add(ParamDef::new(format!("/gravity/{i}/y"), -3.0, 3.0, 0.0).smooth(0.4));
            let z = b.add(ParamDef::new(format!("/gravity/{i}/z"), -3.0, 3.0, 0.0).smooth(0.4));
            // Signed, so one control is both an attractor and a deflector.
            // Two separate controls would mean a well can be both at once,
            // which is not a thing.
            let strength = b.add(
                ParamDef::new(format!("/gravity/{i}/strength"), -2.0, 2.0, 0.0).smooth(0.4),
            );
            let radius =
                b.add(ParamDef::new(format!("/gravity/{i}/radius"), 0.05, 4.0, 1.0).smooth(0.4));
            gravity.push(GravityWell {
                x,
                y,
                z,
                strength,
                radius,
            });
        }

        // The gravity grid's transport, mirroring the scene grid's. Its
        // own rather than shared, because the whole point of a second
        // layer is that it moves on its own clock — a well arriving over
        // eight bars under a look that cut is a normal thing to want.
        let gravity_fire = b.add(ParamDef::new("/gravity/fire", 0.0, SCENE_SLOTS, 0.0).transport());
        let gravity_time = b.add(ParamDef::new("/gravity/time", 0.0, MAX_BLEND, 2.0).transport());
        let gravity_curve = b.add(
            ParamDef::new("/gravity/curve", 0.0, 4.0, 1.0)
                .transport()
                .labels(&["linear", "smooth", "ease in", "ease out", "cut"]),
        );
        let gravity_auto = b.add(
            ParamDef::new("/gravity/auto", 0.0, 1.0, 0.0)
                .transport()
                .labels(&["off", "on"]),
        );
        let gravity_bars = b.add(ParamDef::new("/gravity/bars", 0.25, 16.0, 4.0).transport());

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
        let preset_recall = b.add(ParamDef::new("/preset/recall", 0.0, MAX_PRESET_SLOT, 0.0).transport());
        // The scene grid. Parameters rather than plain settings for the
        // same reason recall is one: a pad controller addresses them for
        // free, and there is one path to firing a scene rather than a UI
        // path and a control path that can drift apart.
        //
        // Unsmoothed, all of them: a glided fire sweeps through every slot
        // between here and there, firing each on the way.
        let scene_fire = b.add(ParamDef::new("/scene/fire", 0.0, SCENE_SLOTS, 0.0).transport());
        // Transition length. Zero is a cut, which is why the range starts
        // there rather than at some minimum that would put cuts out of a
        // fader's reach.
        let scene_time = b.add(ParamDef::new("/scene/time", 0.0, MAX_BLEND, 2.0).transport());
        let scene_curve = b.add(
            ParamDef::new("/scene/curve", 0.0, 4.0, 1.0)
                .transport()
                .labels(&["linear", "smooth", "ease in", "ease out", "cut"]),
        );
        let scene_auto = b.add(
            ParamDef::new("/scene/auto", 0.0, 1.0, 0.0)
                .transport()
                .labels(&["off", "on"]),
        );
        // Bars between autopilot steps. Down to a quarter bar, because a
        // scene change on every beat is a legitimate effect and a minimum
        // of one bar would rule it out.
        let scene_bars = b.add(ParamDef::new("/scene/bars", 0.25, 16.0, 4.0).transport());
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
            punch_flash,
            punch_black,
            punch_invert,
            punch_freeze,
            punch_strobe,
            punch_strobe_div,
            record_active,
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
            video_depth,
            video_relief,
            vector_layers,
            vector_palette,
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
            gravity_amount,
            gravity,
            gravity_fire,
            gravity_time,
            gravity_curve,
            gravity_auto,
            gravity_bars,
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
                // The gravity grid's transport, for the same reasons: a
                // gravity preset holding its own fire control would fire
                // itself on arrival, forever.
                "/gravity/fire",
                "/gravity/time",
                "/gravity/curve",
                "/gravity/auto",
                "/gravity/bars",
                // Punch gestures: performed, not part of a look.
                "/punch/flash",
                "/punch/black",
                "/punch/invert",
                "/punch/freeze",
                "/punch/strobe",
                "/punch/strobe_div",
                "/record/active",
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

    /// Every transport parameter must be excluded from presets, and every
    /// excluded parameter except the panic fader must be transport.
    ///
    /// This is the drift guard. The set existed twice — `preset::EXCLUDED`
    /// and the panel's `is_transport` — and adding the gravity layer
    /// updated one and not the other, so dragging `/gravity/fire` in the
    /// parameter list fired every scene it glided over. Both now derive
    /// from `ParamDef::transport`; this asserts the third list agrees.
    #[test]
    fn transport_and_the_preset_exclusion_list_agree() {
        let p = AppParams::build();
        for (_, def) in p.registry.iter() {
            if def.transport {
                assert!(
                    vizz_mod::preset::EXCLUDED.contains(&def.addr.as_str()),
                    "{} is transport but a preset would capture it",
                    def.addr
                );
                assert!(
                    !vizz_mod::preset::Kind::Look.owns_def(def)
                        && !vizz_mod::preset::Kind::Gravity.owns_def(def),
                    "{} is transport but a layer claims it",
                    def.addr
                );
            }
        }
        for addr in vizz_mod::preset::EXCLUDED {
            // The master dim is excluded for a different reason — it is
            // the panic fader, not transport — and gestures are excluded
            // for their own reason: performed, not scheduled, but still
            // never part of a look. Everything else excluded must be
            // transport, or the exclusion is a list drifting on its own.
            if *addr == "/master/dim" {
                continue;
            }
            let id = p.registry.id(addr).expect("EXCLUDED names a real parameter");
            let def = &p.registry.defs()[id.index()];
            assert!(
                def.transport || def.gesture,
                "{addr} is excluded from presets but neither transport nor gesture"
            );
        }
    }

    /// A flash that fades in is not a flash, and a punch that a preset
    /// can recall is a booby trap. Every gesture rests at zero, snaps
    /// (no smoothing), and sits in the exclusion list.
    #[test]
    fn punch_params_rest_at_zero_and_never_glide() {
        let p = AppParams::build();
        let mut seen = 0;
        for (_, def) in p.registry.iter() {
            if !def.gesture {
                continue;
            }
            seen += 1;
            assert_eq!(def.min, 0.0, "{}: a gesture's off state must be its floor", def.addr);
            assert_eq!(def.smooth, 0.0, "{}: gestures snap", def.addr);
            assert!(
                vizz_mod::preset::EXCLUDED.contains(&def.addr.as_str()),
                "{}: gesture missing from preset::EXCLUDED",
                def.addr
            );
            // Resting at the floor is also what makes a MIDI value
            // binding momentary: release drives to the bottom of the
            // range, and the bottom must mean "off".
            assert_eq!(
                def.default, def.min,
                "{}: a gesture must rest at its floor",
                def.addr
            );
        }
        assert!(seen >= 5, "expected the punch group, found {seen} gestures");
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

#[cfg(test)]
mod reference_tests {
    /// Expand a compacted table row into the addresses it stands for.
    /// `/gravity/N/x` covers wells 0–3; `/lN/kind` covers layers 1–4.
    /// Everything else stands for itself. Shared by the README and docs
    /// table tests so the two cannot drift in what they accept.
    fn expand_compacted(addr: &str) -> Vec<String> {
        if addr.contains("/N/") {
            return (0..4).map(|i| addr.replace("/N/", &format!("/{i}/"))).collect();
        }
        if let Some(rest) = addr.strip_prefix("/lN/") {
            return (1..=super::VECTOR_LAYERS)
                .map(|i| format!("/l{i}/{rest}"))
                .collect();
        }
        vec![addr.to_string()]
    }

    /// The README's OSC reference used to claim completeness while
    /// documenting 21 of 76 addresses, with ranges two releases stale.
    /// This parses the table back out of the README and holds it against
    /// the registry, so adding or retuning a parameter without updating
    /// the reference fails here instead of shipping.
    #[test]
    fn the_readme_osc_reference_matches_the_registry() {
        let readme = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../README.md"
        ))
        .expect("README.md beside the workspace");

        // Rows look like: | `/addr` | min – max | default | meaning |
        // Gravity wells are compacted as `/gravity/N/x` (N = 0–3).
        let mut rows = std::collections::BTreeMap::new();
        for line in readme.lines() {
            let Some(rest) = line.strip_prefix("| `/") else { continue };
            let Some((addr, rest)) = rest.split_once('`') else { continue };
            let addr = format!("/{addr}");
            let cols: Vec<&str> = rest.split('|').map(str::trim).collect();
            if cols.len() < 4 {
                continue;
            }
            let Some((min, max)) = cols[1].split_once('–').map(|(a, b)| {
                (a.trim().parse::<f32>(), b.trim().parse::<f32>())
            }) else {
                continue;
            };
            let (Ok(min), Ok(max), Ok(default)) = (min, max, cols[2].parse::<f32>()) else {
                continue;
            };
            for a in expand_compacted(&addr) {
                rows.insert(a, (min, max, default));
            }
        }
        assert!(rows.len() > 50, "parsed only {} rows — table format changed?", rows.len());

        let p = super::AppParams::build();
        for (_, d) in p.registry.iter() {
            let (min, max, default) = rows
                .get(&d.addr)
                .unwrap_or_else(|| panic!("README OSC reference is missing {}", d.addr));
            assert_eq!((d.min, d.max), (*min, *max), "{}: range drifted", d.addr);
            assert_eq!(d.default, *default, "{}: default drifted", d.addr);
        }
        for addr in rows.keys() {
            assert!(
                p.registry.id(addr).is_some(),
                "README documents {addr}, which no longer exists"
            );
        }
    }

    /// `SHAPE_CLOUD_PAIR` is a bare number pointed at a position in a
    /// list that has grown twice. Held to the label, so inserting a form
    /// ahead of the cloud pair fails here rather than silently making
    /// every cloud load select the wrong shape.
    #[test]
    fn the_cloud_pair_constant_still_names_the_cloud_pair() {
        let p = super::AppParams::build();
        let (_, def) = p.registry.iter().find(|(id, _)| *id == p.shape).expect("/shape/mode");
        assert_eq!(
            def.label_for(super::SHAPE_CLOUD_PAIR),
            Some("cloud pair"),
            "SHAPE_CLOUD_PAIR points at {:?}, not the cloud pair",
            def.label_for(super::SHAPE_CLOUD_PAIR)
        );
        assert!(
            super::SHAPE_CLOUD_PAIR >= def.min && super::SHAPE_CLOUD_PAIR <= def.max,
            "SHAPE_CLOUD_PAIR is outside /shape/mode's range"
        );
    }

    /// The docs site carries the same OSC reference as the README, and a
    /// second copy is a second thing that can go stale — the exact
    /// failure the README test exists for. Held against the registry the
    /// same way, so publishing a docs page that documents a parameter
    /// that no longer exists fails here.
    #[test]
    fn the_docs_site_osc_reference_matches_the_registry() {
        let docs = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../site/docs/index.html"
        ))
        .expect("site/docs/index.html");

        // Rows look like:
        // <tr><td><code>/addr</code></td><td>min – max</td><td>default</td>…
        let mut rows = std::collections::BTreeMap::new();
        for row in docs.split("<tr>") {
            let Some(rest) = row.strip_prefix("<td><code>/") else { continue };
            // The address cell may carry a suffix after the code span —
            // gravity wells are written `/gravity/N/x` (N = 0–3), exactly
            // as the README compacts them.
            let Some((addr, rest)) = rest.split_once("</code>") else { continue };
            let addr = format!("/{addr}");
            let cells: Vec<&str> = rest
                .split("<td>")
                .skip(1)
                .filter_map(|c| c.split("</td>").next())
                .collect();
            if cells.len() < 3 {
                continue;
            }
            let Some((min, max)) = cells[0].split_once('–') else { continue };
            let (Ok(min), Ok(max), Ok(default)) = (
                min.trim().parse::<f32>(),
                max.trim().parse::<f32>(),
                cells[1].trim().parse::<f32>(),
            ) else {
                continue;
            };
            // Compacted rows expand the same way the README's do.
            for a in expand_compacted(&addr) {
                rows.insert(a, (min, max, default));
            }
        }
        assert!(rows.len() > 50, "parsed only {} rows — docs table format changed?", rows.len());

        let p = super::AppParams::build();
        for (_, d) in p.registry.iter() {
            let (min, max, default) = rows
                .get(&d.addr)
                .unwrap_or_else(|| panic!("the docs site's OSC reference is missing {}", d.addr));
            assert_eq!((d.min, d.max), (*min, *max), "{}: docs range drifted", d.addr);
            assert_eq!(d.default, *default, "{}: docs default drifted", d.addr);
        }
        for addr in rows.keys() {
            assert!(
                p.registry.id(addr).is_some(),
                "the docs site documents {addr}, which no longer exists"
            );
        }
    }

    /// The render_panel harness is the only place panel layout is ever
    /// looked at. It once drew 46 of 76 parameters — the whole gravity
    /// layer had never appeared in a single screenshot, which is how a
    /// layout fault in its group would ship unseen.
    #[test]
    fn the_panel_harness_mirrors_the_registry() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vizz-ui/examples/render_panel.rs"
        ))
        .expect("the render_panel harness");
        let p = super::AppParams::build();
        for (_, d) in p.registry.iter() {
            // Transport params used to be exempted here, on the grounds
            // that the panel's parameter list never lists them. That was
            // true of the list and false of the panel: /record/active is
            // transport and has a button of its own in the outputs
            // section, which the exemption then kept out of every
            // screenshot the panel is ever reviewed in. The harness now
            // mirrors the registry outright — a transport param costs a
            // line here and nothing on screen, and the next bespoke
            // control gets reviewed instead of shipping unseen.
            assert!(
                src.contains(&format!("\"{}\"", d.addr)),
                "render_panel harness is missing {} — the panel preview cannot show it",
                d.addr
            );
        }
    }
}
