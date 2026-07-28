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
    midi.learn_target = Some("/fx/trail".into());
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
    let presets = [
        "Slow bloom".to_string(),
        "Butterfly".to_string(),
        "Warehouse 2".to_string(),
        "Ribbon".to_string(),
    ];
    let state = performance::PerformanceState {
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
        grid: &grid,
        gravity: None,
        midi: &midi,
        values: Some(&modulated),
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
        "/master/dim",
    ] {
        let def = ParamDef::new(addr, 0.0, 1.0, 0.4);
        b.add(match addr {
            "/fx/mirror" => ParamDef::new(addr, 0.0, 3.0, 0.0).labels(&["off", "x", "y", "quad"]),
            "/color/palette" => {
                ParamDef::new(addr, 0.0, 4.0, 0.0).labels(&["hsv", "warm", "ember", "ice", "neon"])
            }
            "/particles/size" => ParamDef::new(addr, 0.001, 0.2, 0.015),
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
        ..Default::default()
    }
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
