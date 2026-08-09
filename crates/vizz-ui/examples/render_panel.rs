//! Render the real control panel offscreen to a PNG.
//!
//! Iterating on GUI layout normally needs a display; this renders the
//! actual panel — same code path the app uses — headlessly, so the design
//! can be reviewed on a machine with no window server (and in CI).
//!
//!     cargo run -p vizz-ui --example render_panel -- panel.png

use std::time::Duration;

use vizz_health::{HealthConfig, HealthMonitor};
use vizz_params::{ParamDef, ParamRegistry};
use vizz_ui::{MidiView, OutputStatus, PanelState, PresetEntry, panel};

const DEFAULT_W: u32 = 460;
/// A tall-ish window on a modest display. The panel has to stay usable at
/// this height — the parameter list only ever grows, and a control you
/// cannot scroll to is a control you do not have.
const DEFAULT_H: u32 = 900;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Window size and whether to open every section, both overridable.
///
/// The tests draw the panel with all its sections open on a screen no real
/// window matches. When one of them says a control is missing, the fastest
/// way to find out whether it is missing or merely below the fold is to
/// render at that exact size and look at it.
///
///     cargo run -p vizz-ui --example render_panel -- short.png 900 700 expand
fn options() -> (u32, u32, bool) {
    let arg = |n: usize, fallback: u32| {
        std::env::args()
            .nth(n)
            .and_then(|s| s.parse().ok())
            .unwrap_or(fallback)
    };
    let expand = std::env::args().any(|a| a == "expand");
    (arg(2, DEFAULT_W), arg(3, DEFAULT_H), expand)
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "panel.png".into());
    let (w, h, expand) = options();

    // Mirrors the app's parameter table. Duplicated because it lives in
    // the binary crate, so keep it in step — the point of this preview is
    // that it shows the panel the app actually draws, and a short list
    // would hide exactly the layout problems a long one causes.
    let mut b = ParamRegistry::builder();
    for addr in [
        "/particles/count",
        "/particles/size",
        "/particles/speed",
        "/particles/spread",
        "/particles/hue",
        "/particles/saturation",
        "/particles/brightness",
        "/shape/mode",
        "/shape/morph",
        "/shape/twist",
        "/fx/trail",
        "/fx/zoom",
        "/fx/spin",
        "/fx/mirror",
        "/fx/glow",
        "/fx/shift",
        "/punch/flash",
        "/punch/black",
        "/punch/invert",
        "/punch/freeze",
        "/punch/strobe",
        "/color/palette",
        "/color/spread",
        "/color/drive",
        "/cloud/a",
        "/cloud/b",
        "/cloud/morph",
        "/camera/distance",
        "/camera/orbit",
        "/camera/elevation",
        "/camera/fov",
        "/camera/focus",
        "/camera/defocus",
        "/camera/pan_x",
        "/camera/pan_y",
        "/room/brightness",
        "/room/depth",
        "/room/fade",
        "/room/converge",
        "/room/vanish_x",
        "/room/vanish_y",
        "/room/anchor",
        "/room/embed",
        "/bg/red",
        "/bg/green",
        "/bg/blue",
        "/bg/alpha",
        // The whole gravity layer. It shipped two releases ago and had
        // never once appeared in a panel render — which is how a layout
        // problem in its group would go unseen until a user hit it.
        "/gravity/amount",
        "/gravity/0/x",
        "/gravity/0/y",
        "/gravity/0/z",
        "/gravity/0/strength",
        "/gravity/0/radius",
        "/gravity/1/x",
        "/gravity/1/y",
        "/gravity/1/z",
        "/gravity/1/strength",
        "/gravity/1/radius",
        "/gravity/2/x",
        "/gravity/2/y",
        "/gravity/2/z",
        "/gravity/2/strength",
        "/gravity/2/radius",
        "/gravity/3/x",
        "/gravity/3/y",
        "/gravity/3/z",
        "/gravity/3/strength",
        "/gravity/3/radius",
        "/master/dim",
        "/scene/fire",
        "/scene/time",
        "/scene/curve",
        "/scene/auto",
        "/scene/bars",
        // Transport, and hidden from the parameter list — but the
        // outputs section draws a record button only when this exists,
        // so without it here the one screenshot anyone reviews the panel
        // in cannot show that button at all. The rest are here because
        // the harness mirrors the registry outright now: the gravity
        // transport row and preset recall are what the grids and the
        // preset list read, and a panel preview missing them is a
        // preview of a different app.
        "/video/depth",
        "/video/relief",
        "/record/active",
        "/punch/strobe_div",
        "/gravity/fire",
        "/gravity/time",
        "/gravity/curve",
        "/gravity/auto",
        "/gravity/bars",
        "/preset/recall",
    ] {
        // Labels where the app has them, so the preview shows names under
        // the stepped controls rather than a number that says nothing.
        let def = ParamDef::new(addr, 0.0, 1.0, 0.4);
        b.add(match addr {
            "/shape/mode" => def.labels(&[
                "sphere", "torus", "knot", "grid", "shell", "Lorenz", "Aizawa", "cloud pair",
            ]),
            "/fx/mirror" => def.labels(&["off", "mirror", "quad", "kaleido"]),
            "/color/drive" => def.labels(&["index", "radius", "depth", "height"]),
            "/color/palette" => def.labels(&["hsv", "warm", "ember", "ice", "neon"]),
            "/video/relief" => {
                def.labels(&["luminance", "hue", "saturation", "chroma"])
            }
            "/scene/curve" | "/gravity/curve" => {
                def.labels(&["linear", "smooth", "ease in", "ease out", "cut"])
            }
            // Alpha rests opaque, as it does in the app: the preview
            // must show the shipped state, not a transparent one.
            "/bg/alpha" => ParamDef::new(addr, 0.0, 1.0, 1.0),
            "/bg/blue" => ParamDef::new(addr, 0.0, 1.0, 0.008),
            "/bg/red" | "/bg/green" => ParamDef::new(addr, 0.0, 1.0, 0.004),
            "/scene/auto" => def.labels(&["off", "on"]),
            _ => def,
        });
    }
    let registry = b.build();

    // Plausible live numbers, including one spike so the sparkline and
    // the over-budget counter show something meaningful.
    let mut health = HealthMonitor::new(HealthConfig::default());
    let mut history = Vec::new();
    for i in 0..240 {
        let ms = 6.0 + 2.0 * ((i as f32) / 9.0).sin().abs() + if i == 170 { 12.0 } else { 0.0 };
        health.on_frame(Duration::from_secs_f32(ms / 1000.0));
        history.push(ms);
    }

    let state = PanelState {
        recording: None,
        preset_current: Some(2),
        update_available: Some("0.2.0".into()),
        health: Some(health.snapshot()),
        outputs: vec![
            OutputStatus { name: "syphon:vizz".into(), live: true },
            OutputStatus { name: "ndi:vizz".into(), live: true },
        ],
        frame_times_ms: history,
        frame_budget_ms: 1000.0 / 60.0,
        midi: midi_view(),
        // A plausible live reading, so the preview shows the meters doing
        // something rather than four empty bars.
        audio: audio_view(),
        audio_bands: vizz_audio::default_bands(),
        audio_auto_bpm: true,
        modulated: Vec::new(),
        output: Default::default(),
        palettes: vec![
            "hsv".into(),
            "warm".into(),
            "ember".into(),
            "ice".into(),
            "neon".into(),
            String::new(),
            "warehouse".into(),
        ],
        clouds: vec![
            "lorenz".into(),
            "aizawa".into(),
            "torso-scan".into(),
            "text:VIZZ".into(),
            "logo.png".into(),
            "(empty)".into(),
            "(empty)".into(),
            "live".into(),
        ],
        bpm: 128.0,
        focus_filter: false,
        grid: preview_grid(),
        gravity_grid: None,
        expand_sections: expand,
        presets: vec![
            PresetEntry { name: "Slow bloom".into(), builtin: true, about: None },
            PresetEntry { name: "Butterfly".into(), builtin: true, about: None },
            PresetEntry { name: "Warehouse 2".into(), builtin: false, about: None },
        ],
        bar_phase: 0.05,
    };

    let (device, queue) = gpu();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("panel"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // Dark backdrop standing in for the preview the panel floats over.
    let mut enc = device.create_command_encoder(&Default::default());
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("bg"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.02, g: 0.02, b: 0.05, a: 1.0 }),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    queue.submit([enc.finish()]);

    let mut modulation = vizz_mod::ModEngine::with_defaults();
    modulation.add_route(vizz_mod::Source::Lfo(0), "/particles/hue", 0.3);
    modulation.add_route(vizz_mod::Source::Audio(0), "/particles/size", 0.4);
    let ctx = egui::Context::default();
    ctx.set_visuals(egui::Visuals::dark());
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(w as f32, h as f32),
        )),
        ..Default::default()
    };

    let mut renderer = vizz_ui::EguiRendererForPreview::new(&device, FORMAT);
    let mut last = None;
    // Several passes with advancing time: egui sizes a fresh window on the
    // first pass, and fades new windows in over ~0.1s. Rendering at t=0
    // captures the panel mid-fade — nearly invisible.
    for i in 0..12 {
        let mut input = input.clone();
        input.time = Some(i as f64 * 0.05);
        ctx.begin_pass(input);
        // A name already in use, typed into the save field. Saving used to
        // replace a preset in silence, and the warning that now says so is
        // only reviewable if the preview renders the state it appears in —
        // an empty field draws the one case where there is nothing to say.
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("preset-save-name"), "Warehouse 2".to_string())
        });
        let _ = panel::draw(&ctx, &registry, &state, &mut modulation, &mut Default::default());
        let out = ctx.end_pass();
        renderer.update_textures(&device, &queue, &out.textures_delta);
        last = Some(out);
    }
    let out = last.unwrap();
    let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);

    let mut enc = device.create_command_encoder(&Default::default());
    renderer
        .render(&device, &queue, &mut enc, &view, &primitives, [w, h], out.pixels_per_point)
        .expect("panel render failed");
    queue.submit([enc.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    save_png(&device, &queue, &target, &path, w, h);
    println!("wrote {path}");
}

fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no GPU adapter");
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("panel-preview"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .expect("no device")
}

fn save_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    path: &str,
    w: u32,
    h: u32,
) {
    const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = (w * 4).div_ceil(ALIGN) * ALIGN;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("panel-readback"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let start = row * padded as usize;
        pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    drop(data);
    buffer.unmap();
    image::RgbaImage::from_raw(w, h, pixels).unwrap().save(path).unwrap();
}

/// A plausible live audio reading.
///
/// The envelopes are *computed* from the raw levels and the shipped gains
/// rather than typed in beside them. Two hand-written arrays drift apart
/// the moment a default changes, and a preview whose meters disagree with
/// its own gain figures is worse than no preview — it is the one thing
/// this is supposed to let you check.
fn audio_view() -> vizz_ui::AudioView {
    // Rough per-band RMS for a track at a healthy input level.
    let raw = [0.10f32, 0.085, 0.055, 0.012];
    let gains = vizz_audio::default_bands();
    vizz_ui::AudioView {
        connected: true,
        device: Some("Scarlett 2i2".into()),
        bands: std::array::from_fn(|i| (raw[i] * gains[i].gain).clamp(0.0, 1.0)),
        raw,
        // Peaks run a little above the running level, as they do live.
        raw_peak: std::array::from_fn(|i| raw[i] * 1.6),
        level: 0.21,
        detected_bpm: 128.0,
        confidence: 0.71,
        dropped: 0,
        clock_midi: false,
        clock_ticking: false,
    }
}

/// A grid part-way through a blend, so the preview shows a filled pad, the
/// arrived-at highlight and the fill of a transition in flight rather than
/// sixteen identical blanks.
fn preview_grid() -> vizz_ui::grid_view::GridView {
    let mut names = vec![None; vizz_ui::grid_view::SLOTS];
    for (slot, name) in [(0, "intro"), (1, "build"), (2, "drop"), (6, "outro")] {
        names[slot] = Some(name.to_string());
    }
    vizz_ui::grid_view::GridView {
        names,
        current: Some(1),
        in_flight: Some((2, 0.62)),
        curve_names: ["linear", "smooth", "ease in", "ease out", "cut"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        ..Default::default()
    }
}

/// A representative MIDI state for the preview: one device connected and
/// a couple of controls already learned.
fn midi_view() -> MidiView {
    let mut map = vizz_midi::MidiMap::default();
    map.bind(vizz_midi::Source::ControlChange { channel: 0, controller: 7 }, "/master/dim");
    map.bind(vizz_midi::Source::ControlChange { channel: 0, controller: 1 }, "/particles/hue");
    MidiView {
        revision: 0,
        available: true,
        connected: vec!["Launch Control XL".into()],
        map,
        learn_target: None,
        last_source: None,
        clock_bpm: None,
        clock_started: false,
    }
}
