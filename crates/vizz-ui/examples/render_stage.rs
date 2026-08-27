//! Render the performance layout offscreen to a PNG.
//!
//! The sibling of `render_panel`, and for the same reason: this screen is
//! judged by eye, in a dark room, at a glance. Contrast, hierarchy and
//! whether a fader reads as full or empty are not things that can be
//! settled by reading the source — they have to be looked at, and looked at
//! repeatedly while changing them.
//!
//!     cargo run -p vizz-ui --example render_stage -- stage.png 1280 800

use vizz_mod::perform::Macros;
use vizz_params::{ParamDef, ParamRegistry};
use vizz_ui::{OutputStatus, performance};

const DEFAULT_W: u32 = 1280;
const DEFAULT_H: u32 = 800;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "stage.png".into());
    let arg = |n: usize, fallback: u32| {
        std::env::args()
            .nth(n)
            .and_then(|s| s.parse().ok())
            .unwrap_or(fallback)
    };
    let (w, h) = (arg(2, DEFAULT_W), arg(3, DEFAULT_H));

    let registry = registry();
    let mut macros = Macros::default();
    // A believable set of assignments: the things actually reached for
    // mid-set, spanning stepped and continuous so both label styles show.
    for (slot, addr) in [
        "/particles/size",
        "/particles/speed",
        "/particles/brightness",
        "/fx/glow",
        "/fx/trail",
        "/shape/morph",
        "/fx/mirror",
        "/color/palette",
    ]
    .iter()
    .enumerate()
    {
        macros.set(slot, Some((*addr).to_string()));
    }
    // Spread the values so the row is not eight identical columns — the
    // whole question is whether you can read a fader's position at a
    // glance, and eight of the same tells you nothing.
    for (addr, v) in [
        // One punch latched, so the engaged state is in the shot.
        ("/punch/strobe", 1.0),
        // Two layers on, so the strip shows in the review shot.
        ("/l1/kind", 1.0),
        ("/l1/blend", 1.0),
        ("/l1/color", 1.0),
        ("/l2/kind", 2.0),
        ("/l2/blend", 4.0),
        ("/l2/color", 2.0),
        ("/pal/1/r", 0.92),
        ("/pal/1/g", 0.10),
        ("/pal/1/b", 0.14),
        ("/pal/2/r", 0.10),
        ("/pal/2/g", 0.30),
        ("/pal/2/b", 0.95),
        ("/particles/size", 0.07),
        ("/particles/speed", 0.42),
        ("/particles/brightness", 0.88),
        ("/fx/glow", 0.31),
        ("/fx/trail", 0.66),
        ("/shape/morph", 0.15),
        ("/fx/mirror", 2.0),
        ("/color/palette", 3.0),
    ] {
        if let Some(id) = registry.id(addr) {
            registry.set(id, v);
        }
    }
    if let Some(id) = registry.id("/master/dim") {
        registry.set(id, 0.82);
    }

    let audio = audio_view();
    let grid = stage_grid();
    // The gravity row, unless the caller asks for it to be absent. The
    // harness used to hardcode `None`, so the layout was never once
    // rendered in the configuration a prepared set actually runs in — and
    // that is exactly where the fader row overflowed the window.
    let gravity = if std::env::args().any(|a| a == "no-gravity") {
        None
    } else {
        Some(gravity_grid())
    };
    // One fader already bound and one mid-learn, so the preview shows both
    // states of the MIDI chip rather than eight identical "learn" links.
    let mut midi = vizz_ui::MidiView {
        available: true,
        ..Default::default()
    };
    midi.map.bind(
        vizz_midi::Source::ControlChange {
            channel: 0,
            controller: 21,
        },
        "/particles/size",
    );
    midi.map.bind(
        vizz_midi::Source::ControlChange {
            channel: 0,
            controller: 22,
        },
        "/fx/glow",
    );
    midi.learn_target = Some(vizz_midi::LearnTarget::param("/fx/trail"));
    // Two parameters pushed away from where their faders sit, so the
    // modulation marks are visible in the preview — that mark is the whole
    // reason this screen can be trusted while an LFO is running.
    let modulated: Vec<f32> = registry
        .iter()
        .map(|(id, def)| {
            let v = registry.target(id);
            match def.addr.as_str() {
                "/particles/speed" => (v + 0.28).min(def.max),
                "/shape/morph" => (v + 0.44).min(def.max),
                _ => v,
            }
        })
        .collect();
    // Sources as well as names, because the row groups and colours by
    // what each look was built on — a screenshot of four unsorted looks
    // would show none of what the page beside it describes.
    let look = |name: &str, source: &str| vizz_ui::PresetEntry {
        name: name.to_string(),
        builtin: false,
        about: None,
        source: Some(source.to_string()),
    };
    let presets = [
        look("Slow bloom", "sphere"),
        look("Butterfly", "Aizawa"),
        look("Warehouse 2", "warehouse.ply"),
        look("Ribbon", "torso-scan.ply"),
    ];
    // A believable set list. The docs describe a row of chips above the
    // pads, so the picture the docs show has to have one — a screenshot
    // that contradicts the page beside it is worse than no screenshot.
    let decks: Vec<performance::DeckChip> = ["opener", "second song", "encore"]
        .iter()
        .map(|name| performance::DeckChip {
            name: (*name).to_string(),
            midi: None,
            learning: false,
            origin: 1,
        })
        .collect();
    let state = performance::PerformanceState {
        project: "Warehouse",
        decks: &decks,
        active_deck: 1,
        follow_columns: Some(true),
        recording: Some(vizz_ui::RecordingView { secs: 72, frames: 4310, dropped: 12 }),
        preset_current: Some(2),
        outputs: &[
            OutputStatus {
                name: "syphon:vizz".into(),
                live: true,
            },
            OutputStatus {
                name: "ndi:vizz".into(),
                live: false,
            },
        ],
        audio: &audio,
        fps: 59.4,
        over_budget: false,
        bpm: 128.0,
        bar_phase: 0.12,
        presets: &presets,
        thumb_revision: 0,
        grid: &grid,
        gravity: gravity.as_ref(),
        midi: &midi,
        values: Some(&modulated),
        output_texture: None,
        output_aspect: 16.0 / 9.0,
        graph: None,
    };

    let (device, queue) = gpu();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stage"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // The performance layout owns the whole window, so the backdrop is
    // what it draws over: near-black, as the app's own window is.
    let mut enc = device.create_command_encoder(&Default::default());
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("bg"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.02,
                    g: 0.021,
                    b: 0.026,
                    a: 1.0,
                }),
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

    let ctx = egui::Context::default();
    ctx.set_visuals(egui::Visuals::dark());
    let mut renderer = vizz_ui::EguiRendererForPreview::new(&device, FORMAT);
    let mut last = None;
    for i in 0..12 {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(w as f32, h as f32),
            )),
            time: Some(i as f64 * 0.05),
            ..Default::default()
        });
        if std::env::args().any(|a| a == "--peek") {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("performance-peek"), true));
        }
        let _ = performance::draw(&ctx, &registry, &state, &mut macros);
        let out = ctx.end_pass();
        renderer.update_textures(&device, &queue, &out.textures_delta);
        last = Some(out);
    }
    let out = last.unwrap();
    let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);

    let mut enc = device.create_command_encoder(&Default::default());
    renderer
        .render(
            &device,
            &queue,
            &mut enc,
            &view,
            &primitives,
            [w, h],
            out.pixels_per_point,
        )
        .expect("stage render failed");
    queue.submit([enc.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    save_png(&device, &queue, &target, &path, w, h);
    println!("wrote {path}");
}

/// Mirrors the app's parameter table closely enough that the assign picker
/// and the fader labels show realistic content.
fn registry() -> ParamRegistry {
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
        "/fx/glow",
        "/fx/mirror",
        "/color/palette",
        "/color/drive",
        "/color/spread",
        "/punch/flash",
        "/punch/strobe",
        "/punch/black",
        "/punch/freeze",
        "/punch/invert",
        "/punch/strobe_div",
        // The status strip draws its REC chip only when this exists, so
        // without it here the harness renders a performance screen with
        // no way to record — which is the state this harness was used to
        // review, and missed.
        "/record/active",
        // The vector layer surface, so the strip can appear in the one
        // screenshot this layout is reviewed in.
        "/l1/kind",
        "/l1/freq",
        "/l1/phase",
        "/l1/duty",
        "/l1/sides",
        "/l1/inset",
        "/l1/fold",
        "/l1/invert",
        "/l1/x",
        "/l1/y",
        "/l1/rot",
        "/l1/scale",
        "/l1/color",
        "/l1/blend",
        "/l1/opacity",
        "/l2/kind",
        "/l2/freq",
        "/l2/phase",
        "/l2/duty",
        "/l2/sides",
        "/l2/inset",
        "/l2/fold",
        "/l2/invert",
        "/l2/x",
        "/l2/y",
        "/l2/rot",
        "/l2/scale",
        "/l2/color",
        "/l2/blend",
        "/l2/opacity",
        "/l3/kind",
        "/l3/freq",
        "/l3/phase",
        "/l3/duty",
        "/l3/sides",
        "/l3/inset",
        "/l3/fold",
        "/l3/invert",
        "/l3/x",
        "/l3/y",
        "/l3/rot",
        "/l3/scale",
        "/l3/color",
        "/l3/blend",
        "/l3/opacity",
        "/l4/kind",
        "/l4/freq",
        "/l4/phase",
        "/l4/duty",
        "/l4/sides",
        "/l4/inset",
        "/l4/fold",
        "/l4/invert",
        "/l4/x",
        "/l4/y",
        "/l4/rot",
        "/l4/scale",
        "/l4/color",
        "/l4/blend",
        "/l4/opacity",
        "/pal/0/r",
        "/pal/0/g",
        "/pal/0/b",
        "/pal/1/r",
        "/pal/1/g",
        "/pal/1/b",
        "/pal/2/r",
        "/pal/2/g",
        "/pal/2/b",
        "/pal/3/r",
        "/pal/3/g",
        "/pal/3/b",
        "/vec/place",
        "/master/dim",
    ] {
        let def = ParamDef::new(addr, 0.0, 1.0, 0.4);
        b.add(match addr {
            "/fx/mirror" => ParamDef::new(addr, 0.0, 3.0, 0.0).labels(&["off", "mirror", "quad", "kaleido"]),
            "/color/palette" => {
                ParamDef::new(addr, 0.0, 4.0, 0.0).labels(&["hsv", "warm", "ember", "ice", "neon"])
            }
            "/particles/size" => ParamDef::new(addr, 0.001, 0.2, 0.015),
            "/punch/strobe_div" => ParamDef::new(addr, 0.25, 4.0, 0.5).transport(),
            "/record/active" => ParamDef::new(addr, 0.0, 1.0, 0.0).transport(),
            a if a.starts_with("/l") && a.ends_with("/kind") => {
                ParamDef::new(addr, 0.0, 7.0, 0.0).labels(&[
                    "off", "rings", "stripes", "checker", "polygon", "star", "rays", "dots",
                ])
            }
            a if a.starts_with("/l") && a.ends_with("/blend") => {
                ParamDef::new(addr, 0.0, 6.0, 0.0).labels(&[
                    "normal", "multiply", "screen", "add", "difference", "exclusion", "subtract",
                ])
            }
            a if a.starts_with("/l") && a.ends_with("/freq") => {
                ParamDef::new(addr, 0.5, 64.0, 8.0)
            }
            a if a.starts_with("/l") && a.ends_with("/color") => {
                ParamDef::new(addr, 0.0, 3.0, 0.0)
            }
            "/vec/place" => ParamDef::new(addr, 0.0, 1.0, 0.0).labels(&["scene", "print"]),
            "/master/dim" => ParamDef::new(addr, 0.0, 1.0, 1.0),
            _ => def,
        });
    }
    b.build()
}

fn audio_view() -> vizz_ui::AudioView {
    let raw = [0.10f32, 0.085, 0.055, 0.012];
    let gains = vizz_audio::default_bands();
    vizz_ui::AudioView {
        connected: true,
        device: Some("Scarlett 2i2".into()),
        bands: std::array::from_fn(|i| (raw[i] * gains[i].gain).clamp(0.0, 1.0)),
        raw,
        raw_peak: std::array::from_fn(|i| raw[i] * 1.6),
        level: 0.21,
        detected_bpm: 128.0,
        confidence: 0.71,
        dropped: 0,
        // Following MIDI clock with ticks arriving, so the badge shows.
        clock_midi: true,
        clock_ticking: true,
    }
}

/// A second grid for the gravity layer, so the preview shows the shape a
/// prepared set actually has.
fn gravity_grid() -> vizz_ui::grid_view::GridView {
    let mut names = vec![None; vizz_ui::grid_view::SLOTS];
    for (slot, name) in [(0, "still"), (1, "pull in"), (4, "burst")] {
        names[slot] = Some(name.to_string());
    }
    vizz_ui::grid_view::GridView {
        names,
        // The review shot has to show what the app shows, or it is a
        // picture of a layout nobody runs.
        accent: Some(vizz_ui::grid_view::GRAVITY_ACCENT),
        current: Some(1),
        curve_names: ["linear", "smooth", "ease in", "ease out", "cut"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        // The gravity row says "gravity", not "scene". Rendering both rows
        // is what makes a wrong noun visible rather than only wrong.
        noun: "gravity",
        midi: midi_labels(&[(1, "ch1 note52")]),
        midi_available: true,
        ..Default::default()
    }
}

/// A grid mid-blend with the autopilot running, so the preview shows the
/// countdown sweep and the arriving pad rather than sixteen blanks.
fn stage_grid() -> vizz_ui::grid_view::GridView {
    let mut names = vec![None; vizz_ui::grid_view::SLOTS];
    for (slot, name) in [
        (0, "intro"),
        (1, "build"),
        (2, "drop"),
        (6, "outro"),
        (9, "strobe"),
        (12, "calm"),
    ] {
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
        autopilot: true,
        bars: 4.0,
        auto_phase: Some(0.42),
        upcoming: Some(6),
        // Half the row mapped to a controller and one pad mid-learn. A
        // prepared set has most of the grid under a controller's fingers,
        // and drawing it unmapped is drawing a shape nobody plays — the
        // same blind spot that hid the fader overflow when this harness
        // hardcoded an empty gravity layer.
        midi: midi_labels(&[(0, "ch1 note36"), (1, "ch1 note37"), (2, "ch1 note38"), (6, "ch1 note42")]),
        learning: Some(9),
        midi_available: true,
        ..Default::default()
    }
}

/// Binding labels for the slots named, empty elsewhere.
fn midi_labels(bound: &[(usize, &str)]) -> Vec<Option<String>> {
    let mut out = vec![None; vizz_ui::grid_view::SLOTS];
    for (slot, label) in bound {
        out[*slot] = Some((*label).to_string());
    }
    out
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
        label: Some("stage-preview"),
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
        label: Some("stage-readback"),
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
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
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
    image::RgbaImage::from_raw(w, h, pixels)
        .unwrap()
        .save(path)
        .unwrap();
}
