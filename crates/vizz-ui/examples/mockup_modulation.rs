//! Render candidate modulation-UI designs offscreen, for choosing between
//! them before any of them is built.
//!
//! Drawn in real egui at the real panel width with the real fonts, so what
//! this shows is what the widgets would actually measure — a drawing would
//! quietly lie about density, which is the whole thing being decided.
//!
//!     cargo run -p vizz-ui --example mockup_modulation -- out_dir

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, vec2, pos2};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Panel width the app actually uses, so option A is judged at its real size.
const PANEL_W: u32 = 460;

struct Shot {
    name: &'static str,
    w: u32,
    h: u32,
    draw: fn(&egui::Context, f32, f32),
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let (device, queue) = gpu();

    let shots = [
        Shot { name: "mod_a_matrix", w: PANEL_W, h: 520, draw: draw_matrix },
        Shot { name: "mod_b_canvas", w: 900, h: 560, draw: draw_canvas },
        Shot { name: "mod_c_flat", w: PANEL_W, h: 360, draw: draw_flat },
        // The real canvas, not a mockup: same code path the app runs.
        Shot { name: "graph_real", w: 900, h: 620, draw: draw_real_graph },
        Shot { name: "graph_real_zoomed", w: 900, h: 620, draw: draw_real_zoomed },
        Shot { name: "performance", w: 900, h: 460, draw: draw_performance },
        Shot { name: "shortcuts", w: 420, h: 260, draw: draw_shortcuts },
    ];

    for s in shots {
        let path = format!("{dir}/{}.png", s.name);
        render(&device, &queue, &s, &path);
        println!("wrote {path}");
    }
}

// --- Option A: graph engine behind a routing matrix ---------------------

fn draw_matrix(ctx: &egui::Context, _w: f32, _h: f32) {
    egui::Area::new(egui::Id::new("mockup")).fixed_pos([12.0, 10.0]).show(ctx, |ui| {
        ui.heading("A — routing matrix");
        ui.small("Full graph underneath; compact list UI. Fits the existing panel.");
        ui.separator();

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Sources").strong());
            ui.small("· value shown live");
        });
        for (name, detail, v, color) in [
            ("LFO 1", "sine · 4 beats", 0.72, Color32::from_rgb(130, 190, 255)),
            ("LFO 2", "tri · 1 beat", 0.31, Color32::from_rgb(130, 190, 255)),
            ("Band 1", "30–110 Hz", 0.88, Color32::from_rgb(120, 220, 160)),
            ("Band 3", "400–2k Hz", 0.24, Color32::from_rgb(120, 220, 160)),
        ] {
            ui.horizontal(|ui| {
                ui.add_sized([48.0, 16.0], egui::Label::new(name));
                bar(ui, v, color, 54.0);
                ui.small(detail);
            });
        }
        if ui.small_button("+ source").clicked() {}

        ui.add_space(6.0);
        ui.separator();
        ui.label(egui::RichText::new("Routes").strong());

        route_row(ui, "LFO 1", "shape/morph", "0.40", None);
        route_row(ui, "Band 1", "particles/size", "0.25", Some("exp² -> clamp"));
        route_row(ui, "Band 3", "fx/glow", "0.60", Some("smooth 120ms"));
        route_row(ui, "LFO 2", "LFO 1 rate", "0.15", Some("chained"));
        route_row(ui, "Level", "fx/trail", "0.30", None);

        ui.horizontal(|ui| {
            if ui.small_button("+ route").clicked() {}
            if ui.small_button("+ operator").clicked() {}
        });
        ui.add_space(4.0);
        ui.small("A route's target can be another source's parameter —");
        ui.small("that is the chaining a graph gives you, without a canvas.");
    });
}

fn route_row(ui: &mut egui::Ui, src: &str, dst: &str, depth: &str, op: Option<&str>) {
    ui.horizontal(|ui| {
        let mut on = true;
        ui.checkbox(&mut on, "");
        ui.add_sized([46.0, 16.0], egui::Label::new(egui::RichText::new(src).small()));
        ui.small("->");
        ui.add_sized([94.0, 16.0], egui::Label::new(egui::RichText::new(dst).small()));
        ui.small(format!("×{depth}"));
        if let Some(op) = op {
            ui.small(egui::RichText::new(op).color(Color32::from_rgb(200, 170, 110)));
        }
    });
}

fn bar(ui: &mut egui::Ui, v: f32, color: Color32, w: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, 10.0), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, Color32::from_black_alpha(140));
    ui.painter().rect_filled(
        Rect::from_min_size(rect.left_top(), vec2(rect.width() * v.clamp(0.0, 1.0), rect.height())),
        2.0,
        color,
    );
}

// --- Option B: TouchDesigner-style canvas -------------------------------

fn draw_canvas(ctx: &egui::Context, w: f32, h: f32) {
    egui::Area::new(egui::Id::new("mockup")).fixed_pos([12.0, 10.0]).show(ctx, |ui| {
        ui.heading("B — node canvas");
        ui.small("Draggable nodes and wires, pan/zoom, saved layout. Needs its own window.");
        ui.separator();

        let (rect, _) = ui.allocate_exact_size(vec2(w - 24.0, h - 76.0), Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 4.0, Color32::from_rgb(24, 26, 30));

        // Dot grid, the usual cue that a surface pans.
        let step = 26.0;
        let mut y = rect.top() + 12.0;
        while y < rect.bottom() {
            let mut x = rect.left() + 12.0;
            while x < rect.right() {
                p.circle_filled(pos2(x, y), 1.0, Color32::from_rgb(44, 48, 54));
                x += step;
            }
            y += step;
        }

        let src = Color32::from_rgb(70, 120, 175);
        let opc = Color32::from_rgb(150, 120, 60);
        let dst = Color32::from_rgb(70, 140, 100);

        // Two chains, one of them branching, so the wiring is visible.
        let lfo1 = node(&p, rect.left_top() + vec2(30.0, 40.0), "LFO 1", &["sine", "4 beats"], src, 0.72);
        let band1 = node(&p, rect.left_top() + vec2(30.0, 170.0), "Band 1", &["30–110 Hz", "×6.0"], src, 0.88);
        let lvl = node(&p, rect.left_top() + vec2(30.0, 300.0), "Level", &["broadband"], src, 0.34);

        let curve = node(&p, rect.left_top() + vec2(250.0, 170.0), "Curve", &["exp²", "clamp 0..1"], opc, 0.0);
        let mul = node(&p, rect.left_top() + vec2(250.0, 300.0), "Multiply", &["a × b"], opc, 0.0);

        let morph = node(&p, rect.left_top() + vec2(500.0, 40.0), "/shape/morph", &["× 0.40"], dst, 0.55);
        let size = node(&p, rect.left_top() + vec2(500.0, 170.0), "/particles/size", &["× 0.25"], dst, 0.41);
        let trail = node(&p, rect.left_top() + vec2(500.0, 300.0), "/fx/trail", &["× 0.30"], dst, 0.18);

        for (a, b) in [
            (lfo1, morph),
            (band1, curve),
            (curve, size),
            (lvl, mul),
            (band1, mul),
            (mul, trail),
        ] {
            wire(&p, a, b);
        }

        p.text(
            rect.left_bottom() + vec2(12.0, -18.0),
            Align2::LEFT_BOTTOM,
            "drag to move · scroll to zoom · drag a port to wire",
            FontId::proportional(11.0),
            Color32::from_rgb(110, 116, 124),
        );
    });
}

/// Returns (output port, input port) positions for wiring.
fn node(
    p: &egui::Painter,
    tl: Pos2,
    title: &str,
    lines: &[&str],
    accent: Color32,
    value: f32,
) -> (Pos2, Pos2) {
    let w = 132.0;
    let h = 30.0 + lines.len() as f32 * 15.0 + if value > 0.0 { 12.0 } else { 0.0 };
    let rect = Rect::from_min_size(tl, vec2(w, h));
    p.rect_filled(rect, 5.0, Color32::from_rgb(38, 41, 47));
    p.rect_stroke(rect, 5.0, Stroke::new(1.0, accent), egui::StrokeKind::Outside);
    // Title bar.
    p.rect_filled(
        Rect::from_min_size(tl, vec2(w, 20.0)),
        5.0,
        accent.gamma_multiply(0.55),
    );
    p.text(tl + vec2(8.0, 10.0), Align2::LEFT_CENTER, title, FontId::proportional(12.0), Color32::from_rgb(232, 236, 240));

    let mut y = 28.0;
    for line in lines {
        p.text(
            tl + vec2(8.0, y),
            Align2::LEFT_TOP,
            *line,
            FontId::proportional(10.5),
            Color32::from_rgb(155, 162, 170),
        );
        y += 15.0;
    }
    if value > 0.0 {
        let bar = Rect::from_min_size(tl + vec2(8.0, y + 1.0), vec2(w - 16.0, 5.0));
        p.rect_filled(bar, 2.0, Color32::from_black_alpha(150));
        p.rect_filled(
            Rect::from_min_size(bar.left_top(), vec2(bar.width() * value, bar.height())),
            2.0,
            accent.gamma_multiply(1.6),
        );
    }

    let out = pos2(rect.right(), rect.top() + 10.0);
    let inp = pos2(rect.left(), rect.top() + 10.0);
    p.circle_filled(out, 3.5, Color32::from_rgb(200, 206, 214));
    p.circle_filled(inp, 3.5, Color32::from_rgb(120, 126, 134));
    (out, inp)
}

fn wire(p: &egui::Painter, from: (Pos2, Pos2), to: (Pos2, Pos2)) {
    let (a, b) = (from.0, to.1);
    // Horizontal-tangent bezier: the shape everyone expects from a patcher.
    let dx = ((b.x - a.x).abs() * 0.5).max(30.0);
    p.add(egui::Shape::CubicBezier(egui::epaint::CubicBezierShape::from_points_stroke(
        [a, pos2(a.x + dx, a.y), pos2(b.x - dx, b.y), b],
        false,
        Color32::TRANSPARENT,
        Stroke::new(1.6, Color32::from_rgb(96, 122, 150)),
    )));
}

// --- Option C: current list plus per-route operators --------------------

fn draw_flat(ctx: &egui::Context, _w: f32, _h: f32) {
    egui::Area::new(egui::Id::new("mockup")).fixed_pos([12.0, 10.0]).show(ctx, |ui| {
        ui.heading("C — current list + curves");
        ui.small("Smallest change. Per-route shaping, but no chaining.");
        ui.separator();
        ui.label(egui::RichText::new("Routes").strong());

        for (src, dst, depth, curve) in [
            ("LFO 1", "shape/morph", "0.40", "linear"),
            ("Band 1", "particles/size", "0.25", "exp²"),
            ("Band 3", "fx/glow", "0.60", "S-curve"),
            ("LFO 2", "fx/spin", "0.20", "quantise ¼"),
        ] {
            ui.horizontal(|ui| {
                let mut on = true;
                ui.checkbox(&mut on, "");
                ui.add_sized([46.0, 16.0], egui::Label::new(egui::RichText::new(src).small()));
                ui.small("->");
                ui.add_sized([92.0, 16.0], egui::Label::new(egui::RichText::new(dst).small()));
                ui.small(format!("×{depth}"));
                egui::ComboBox::from_id_salt(src)
                    .selected_text(curve)
                    .width(78.0)
                    .show_ui(ui, |_| {});
            });
        }
        if ui.small_button("+ route").clicked() {}
        ui.add_space(6.0);
        ui.small("A source still cannot feed another source —");
        ui.small("no LFO modulating an LFO's rate, no summing two bands.");
    });
}

// --- the real canvas -----------------------------------------------------

fn demo_graph() -> vizz_mod::graph::NodeGraph {
    use vizz_mod::graph::{CurveShape, MathOp, NodeKind as K};
    let mut g = vizz_mod::graph::NodeGraph::default();
    let lfo = g.add(K::Lfo(vizz_mod::Lfo::default()), [20.0, 20.0]);
    let band = g.add(K::Band(0), [20.0, 150.0]);
    let level = g.add(K::Level, [20.0, 280.0]);
    let curve = g.add(K::Curve { shape: CurveShape::Exp2, amount: 1.0 }, [240.0, 150.0]);
    let math = g.add(K::Math { op: MathOp::Multiply }, [240.0, 280.0]);
    let morph = g.add(K::Param { addr: "/shape/morph".into(), depth: 0.4 }, [470.0, 20.0]);
    let size = g.add(K::Param { addr: "/particles/size".into(), depth: 0.25 }, [470.0, 150.0]);
    let trail = g.add(K::Param { addr: "/fx/trail".into(), depth: 0.3 }, [470.0, 280.0]);
    g.connect(lfo, morph, 0);
    g.connect(band, curve, 0);
    g.connect(curve, size, 0);
    g.connect(level, math, 0);
    g.connect(band, math, 1);
    g.connect(math, trail, 0);
    g
}

fn registry() -> vizz_params::ParamRegistry {
    use vizz_params::ParamDef;
    let mut b = vizz_params::ParamRegistry::builder();
    for (a, lo, hi, d) in [
        ("/shape/morph", 0.0, 1.0, 0.0),
        ("/particles/size", 0.001, 0.2, 0.015),
        ("/fx/trail", 0.0, 0.98, 0.0),
    ] {
        b.add(ParamDef::new(a, lo, hi, d));
    }
    b.build()
}

/// Ticked once so the live per-node readouts show real numbers rather
/// than zeros — a canvas of +0.00 would not show whether they work.
fn ticked() -> (vizz_mod::graph::NodeGraph, vizz_params::ParamRegistry) {
    let (mut g, reg) = (demo_graph(), registry());
    let mut offsets = Vec::new();
    for _ in 0..8 {
        g.tick(1.0 / 60.0, 0.02, 1.7, vizz_mod::AudioLevels { bands: &[0.8, 0.3, 0.5, 0.2], level: 0.44 }, &reg, &mut offsets);
    }
    (g, reg)
}

fn draw_real_graph(ctx: &egui::Context, w: f32, h: f32) {
    let (mut g, reg) = ticked();
    let mut view = vizz_ui::GraphView::default();
    view.selected = Some(vizz_mod::graph::NodeId(3));
    egui::Area::new(egui::Id::new("real")).fixed_pos([0.0, 0.0]).show(ctx, |ui| {
        ui.set_max_size(vec2(w, h));
        view.show(ui, &mut g, &reg);
    });
}

/// Zoomed out: the case where a node has to stay legible or the canvas is
/// useless exactly when a patch is big enough to need it.
fn draw_real_zoomed(ctx: &egui::Context, w: f32, h: f32) {
    let (mut g, reg) = ticked();
    let mut view = vizz_ui::GraphView::with_zoom(0.55);
    egui::Area::new(egui::Id::new("realz")).fixed_pos([0.0, 0.0]).show(ctx, |ui| {
        ui.set_max_size(vec2(w, h));
        view.show(ui, &mut g, &reg);
    });
}

/// The `?` overlay, drawn through the real code path.
fn draw_shortcuts(ctx: &egui::Context, _w: f32, _h: f32) {
    let mut open = true;
    vizz_ui::draw_shortcuts_for_preview(ctx, &mut open);
}

fn draw_performance(ctx: &egui::Context, _w: f32, _h: f32) {
    use vizz_params::ParamDef;
    let mut b = vizz_params::ParamRegistry::builder();
    // Labels included where the app has them, or the preview would show
    // bare numbers under `mode` and `mirror` and hide the thing this
    // layout most needs to get right.
    const SHAPES: &[&str] = &[
        "sphere", "torus", "knot", "grid", "shell", "Lorenz", "Aizawa", "cloud pair",
    ];
    const MIRRORS: &[&str] = &["off", "x", "y", "quad"];
    for (a, lo, hi, d, labels) in [
        ("/particles/size", 0.001, 0.2, 0.06, None),
        ("/particles/speed", 0.0, 4.0, 1.4, None),
        ("/shape/mode", 0.0, 7.0, 5.0, Some(SHAPES)),
        ("/shape/morph", 0.0, 1.0, 0.3, None),
        ("/fx/trail", 0.0, 0.98, 0.72, None),
        ("/fx/glow", 0.0, 1.0, 0.55, None),
        ("/fx/mirror", 0.0, 3.0, 2.0, Some(MIRRORS)),
        ("/particles/hue", 0.0, 1.0, 0.58, None),
        ("/master/dim", 0.0, 1.0, 0.85, None),
    ] {
        let def = ParamDef::new(a, lo, hi, d);
        b.add(match labels {
            Some(l) => def.labels(l),
            None => def,
        });
    }
    let reg = b.build();
    let mut macros = vizz_mod::perform::Macros::default();
    let audio = vizz_ui::AudioView {
        connected: true,
        device: Some("Scarlett 2i2".into()),
        bands: [0.85, 0.42, 0.3, 0.12],
        raw: [0.14, 0.1, 0.06, 0.01],
        level: 0.22,
        detected_bpm: 128.0,
        confidence: 0.72,
        dropped: 0,
    };
    // A grid part-way through a blend, so the preview shows the pad fill
    // and the two highlights doing something rather than sixteen blanks.
    let grid = {
        let mut names = vec![None; vizz_ui::grid_view::SLOTS];
        for (slot, name) in [(0, "intro"), (1, "build"), (2, "drop"), (3, "break"), (8, "outro")] {
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
    };
    let state = vizz_ui::PerformanceState {
        grid: &grid,
        outputs: &[
            vizz_ui::OutputStatus { name: "syphon:vizz".into(), live: true },
            vizz_ui::OutputStatus { name: "ndi:vizz".into(), live: true },
        ],
        audio: &audio,
        fps: 60.0,
        over_budget: false,
        bpm: 128.0,
        bar_phase: 0.05,
        presets: &[
            "Slow bloom".into(),
            "Butterfly".into(),
            "Tunnel".into(),
            "Stage".into(),
            "Confetti".into(),
            "Ribbon".into(),
        ],
    };
    vizz_ui::performance::draw(ctx, &reg, &state, &mut macros);
}

// --- offscreen plumbing -------------------------------------------------

fn render(device: &wgpu::Device, queue: &wgpu::Queue, shot: &Shot, path: &str) {
    let (w, h) = (shot.w, shot.h);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mockup"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());

    let ctx = egui::Context::default();
    ctx.set_visuals(egui::Visuals::dark());
    let mut renderer = vizz_ui::EguiRendererForPreview::new(device, FORMAT);
    let mut last = None;
    // egui fades new surfaces in; advance its clock or the capture lands
    // mid-fade and reads as nearly transparent.
    for i in 0..12 {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32))),
            time: Some(i as f64 * 0.05),
            ..Default::default()
        });
        (shot.draw)(&ctx, w as f32, h as f32);
        let out = ctx.end_pass();
        renderer.update_textures(device, queue, &out.textures_delta);
        last = Some(out);
    }
    let out = last.unwrap();
    let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);

    let mut enc = device.create_command_encoder(&Default::default());
    {
        let _ = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.06, g: 0.065, b: 0.075, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    renderer
        .render(device, queue, &mut enc, &view, &primitives, [w, h], out.pixels_per_point)
        .expect("mockup render failed");
    queue.submit([enc.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    save_png(device, queue, &target, path, w, h);
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
        label: Some("mockup"),
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
        label: Some("mockup-readback"),
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
        tex.size(),
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
    for row in 0..h {
        let s = (row * padded) as usize;
        pixels.extend_from_slice(&data[s..s + (w * 4) as usize]);
    }
    drop(data);
    buffer.unmap();
    image::RgbaImage::from_raw(w, h, pixels)
        .expect("size mismatch")
        .save(path)
        .expect("png write failed");
}
