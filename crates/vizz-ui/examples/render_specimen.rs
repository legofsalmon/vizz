//! The design-system specimen sheet: every token, rendered, on one page.
//!
//!     cargo run -p vizz-ui --example render_specimen -- specimen.png
//!
//! The system is judged the way the vector renderer was: by eye, from an
//! image, not by reading constants. Colour relationships — whether WARN
//! and ARMED are tellable apart at a glance, whether the ink ramp steps
//! evenly, whether a state reads against every surface it sits on — only
//! exist rendered. This sheet is the review surface, and the pattern for
//! a sister app checking that it still speaks the same language.

use vizz_design::{accent, feedback, ink, motion, radius, space, state, surface, text, widgets};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const W: u32 = 960;
const H: u32 = 860;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "specimen.png".into());

    let (device, queue) = gpu();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("specimen"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // The page is the BASE surface itself: tokens are judged on the
    // ground they ship on, not on a neutral grey.
    let mut enc = device.create_command_encoder(&Default::default());
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("bg"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.008, g: 0.009, b: 0.013, a: 1.0 }),
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
        let time = i as f64 * 0.05;
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(W as f32, H as f32),
            )),
            time: Some(time),
            ..Default::default()
        });
        // One armed-button demo held in the armed state: re-seeded every
        // pass so the sheet shows both halves of the idiom side by side.
        ctx.memory_mut(|m| {
            m.data
                .insert_temp(egui::Id::new("specimen-armed-live"), (0u64, time))
        });
        page(&ctx);
        let out = ctx.end_pass();
        renderer.update_textures(&device, &queue, &out.textures_delta);
        last = Some(out);
    }
    let out = last.unwrap();
    let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
    let mut enc = device.create_command_encoder(&Default::default());
    renderer
        .render(&device, &queue, &mut enc, &view, &primitives, [W, H], out.pixels_per_point)
        .expect("specimen render failed");
    queue.submit([enc.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    save_png(&device, &queue, &target, &path);
    println!("wrote {path}");
}

fn page(ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("specimen"))
        .fixed_pos(egui::pos2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_min_size(egui::vec2(W as f32, H as f32));
            egui::Frame::NONE
                .inner_margin(egui::Margin::same(18))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("vizz design language — specimen")
                            .size(text::CONTROL)
                            .strong()
                            .color(ink::PRIMARY),
                    );
                    ui.add_space(space::SECTION);
                    ui.columns(2, |cols| {
                        left(&mut cols[0]);
                        right(&mut cols[1]);
                    });
                });
        });
}

fn left(ui: &mut egui::Ui) {
    section(ui, "STATE — one meaning, one colour");
    for (name, c) in [
        ("state::LEARN", state::LEARN),
        ("state::LIVE", state::LIVE),
        ("state::WARN", state::WARN),
        ("state::ARMED", state::ARMED),
        ("state::CURRENT", state::CURRENT),
    ] {
        swatch(ui, name, c);
    }

    section(ui, "INK — emphasis, not taste");
    for (name, c) in [
        ("ink::PRIMARY", ink::PRIMARY),
        ("ink::SECONDARY", ink::SECONDARY),
        ("ink::TERTIARY", ink::TERTIARY),
        ("ink::FAINT", ink::FAINT),
    ] {
        ui.label(
            egui::RichText::new(format!("{name}  —  the quick brown fox"))
                .size(text::BODY)
                .color(c),
        );
    }
    ui.add_space(space::GAP);

    section(ui, "SURFACE — levels of the dark ground");
    for (name, c) in [
        ("surface::BASE", surface::BASE),
        ("surface::WELL", surface::WELL),
        ("surface::RAISED", surface::RAISED),
        ("surface::SLOT_EMPTY", surface::SLOT_EMPTY),
        ("surface::SLOT", surface::SLOT),
        ("surface::ENGAGED", surface::ENGAGED),
        ("surface::HANDLE", surface::HANDLE),
        ("surface::HAIRLINE", surface::HAIRLINE),
        ("surface::EDGE", surface::EDGE),
        ("surface::TICK", surface::TICK),
        ("surface::FOCUS", surface::FOCUS),
    ] {
        swatch(ui, name, c);
    }

    section(ui, "FEEDBACK — what verdicts sit on");
    swatch(ui, "feedback::OK_TEXT", feedback::OK_TEXT);
    swatch(ui, "feedback::ERR_TEXT", feedback::ERR_TEXT);
    bed(ui, "feedback::OK_BED + ON_OK", feedback::OK_BED, feedback::ON_OK, "saved “warehouse 2am”");
    bed(
        ui,
        "feedback::DANGER_BED + ON_DANGER",
        feedback::DANGER_BED,
        feedback::ON_DANGER,
        "could not save the preset",
    );
    bed(
        ui,
        "feedback::LEARN_BED + ON_LEARN_BED",
        feedback::LEARN_BED,
        feedback::ON_LEARN_BED,
        "MIDI learn armed — click to cancel",
    );

    section(ui, "MOTION — feedback has a clock");
    for line in [
        format!("armed window      {}s", motion::ARM_WINDOW),
        format!(
            "status fade       {}s, errors {}s",
            motion::STATUS_TTL,
            motion::STATUS_ERROR_TTL
        ),
        format!(
            "notices           {}s, errors {}s",
            motion::NOTICE_TTL.as_secs(),
            motion::NOTICE_ERROR_TTL.as_secs()
        ),
    ] {
        ui.label(egui::RichText::new(line).size(text::LABEL).monospace().color(ink::SECONDARY));
    }
}

fn right(ui: &mut egui::Ui) {
    section(ui, "ACCENT — fixed jobs");
    for (name, c) in [
        ("accent::MOD", accent::MOD),
        ("accent::GLOBAL", accent::GLOBAL),
        ("accent::FILL", accent::FILL),
        ("accent::FILL_BRIGHT", accent::FILL_BRIGHT),
        ("accent::METER", accent::METER),
        ("accent::METER_DIM", accent::METER_DIM),
        ("accent::MASTER", accent::MASTER),
        ("accent::MASTER_INK", accent::MASTER_INK),
        ("accent::ARRIVING", accent::ARRIVING),
        ("accent::AUTO", accent::AUTO),
        ("accent::BINDING", accent::BINDING),
        ("accent::REC", accent::REC),
        ("accent::NODE_SOURCE", accent::NODE_SOURCE),
        ("accent::NODE_OPERATOR", accent::NODE_OPERATOR),
        ("accent::NODE_SINK", accent::NODE_SINK),
    ] {
        swatch(ui, name, c);
    }

    section(ui, "TYPE — sizes by role");
    for (name, size) in [
        ("text::MICRO", text::MICRO),
        ("text::INDEX", text::INDEX),
        ("text::SECTION", text::SECTION),
        ("text::CAPTION", text::CAPTION),
        ("text::LABEL", text::LABEL),
        ("text::BODY", text::BODY),
        ("text::CONTROL", text::CONTROL),
        ("text::BANNER", text::BANNER),
    ] {
        ui.label(
            egui::RichText::new(format!("{name}  {size}pt"))
                .size(size)
                .color(ink::PRIMARY),
        );
    }
    ui.add_space(space::GAP);

    section(ui, "SPACE & RADIUS");
    for (name, v) in [
        ("space::CHIP", space::CHIP),
        ("space::GAP", space::GAP),
        ("space::INSET", space::INSET),
        ("space::SECTION", space::SECTION),
        ("space::PAD", space::PAD),
    ] {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(v * 8.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.0, accent::FILL);
            ui.label(egui::RichText::new(format!("{name}  {v}")).size(text::LABEL).monospace().color(ink::SECONDARY));
        });
    }
    ui.add_space(space::GAP);
    ui.horizontal(|ui| {
        for (name, r) in [
            ("PIP", radius::PIP),
            ("CHIP", radius::CHIP),
            ("CONTROL", radius::CONTROL),
            ("TRACK", radius::TRACK),
            ("SHEET", radius::SHEET),
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(52.0, 26.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, r, surface::RAISED);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                name,
                egui::FontId::monospace(text::INDEX),
                ink::TERTIARY,
            );
        }
    });
    ui.add_space(space::GAP);

    section(ui, "WIDGETS — idioms as code");
    ui.horizontal(|ui| {
        widgets::status_dot(ui, true, state::LIVE);
        ui.label(egui::RichText::new("live").size(text::LABEL).color(ink::SECONDARY));
        widgets::status_dot(ui, false, state::WARN);
        ui.label(egui::RichText::new("not sending").size(text::LABEL).color(ink::SECONDARY));
    });
    ui.add_space(space::GAP);
    ui.horizontal(|ui| {
        // At rest, and held armed by the harness: both halves of the
        // armed-click idiom on one sheet.
        widgets::armed_button(
            ui,
            egui::Id::new("specimen-armed-idle"),
            0,
            widgets::Armed {
                idle_label: "reset",
                armed_label: "reset?",
                idle_hover: "",
                armed_hover: "",
                small: false,
            },
        );
        widgets::armed_button(
            ui,
            egui::Id::new("specimen-armed-live"),
            0,
            widgets::Armed {
                idle_label: "reset",
                armed_label: "reset?",
                idle_hover: "",
                armed_hover: "",
                small: false,
            },
        );
        ui.label(
            egui::RichText::new("the armed click: first press asks, in place")
                .size(text::LABEL)
                .color(ink::SECONDARY),
        );
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(space::SECTION);
    ui.label(
        egui::RichText::new(title)
            .size(text::SECTION)
            .strong()
            .color(ink::TERTIARY)
            .monospace(),
    );
    ui.add_space(space::GAP);
}

fn swatch(ui: &mut egui::Ui, name: &str, c: egui::Color32) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 14.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, radius::CHIP, c);
        ui.painter().rect_stroke(
            rect,
            radius::CHIP,
            (1.0, surface::EDGE),
            egui::StrokeKind::Inside,
        );
        ui.label(
            egui::RichText::new(format!("{name}  {} {} {}", c.r(), c.g(), c.b()))
                .size(text::LABEL)
                .monospace()
                .color(ink::SECONDARY),
        );
    });
}

fn bed(ui: &mut egui::Ui, name: &str, bed: egui::Color32, on: egui::Color32, sample: &str) {
    egui::Frame::NONE
        .fill(bed)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .corner_radius(radius::TRACK)
        .show(ui, |ui| {
            ui.label(egui::RichText::new(sample).size(text::LABEL).color(on));
        });
    ui.label(egui::RichText::new(name).size(text::INDEX).monospace().color(ink::TERTIARY));
    ui.add_space(space::GAP);
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
        label: Some("specimen"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .expect("no device")
}

fn save_png(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture, path: &str) {
    const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = (W * 4).div_ceil(ALIGN) * ALIGN;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("specimen-readback"),
        size: (padded * H) as u64,
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
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
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
    let mut img = image::RgbaImage::new(W, H);
    for y in 0..H {
        let row = &data[(y * padded) as usize..(y * padded + W * 4) as usize];
        for x in 0..W {
            let o = (x * 4) as usize;
            img.put_pixel(x, y, image::Rgba([row[o], row[o + 1], row[o + 2], 255]));
        }
    }
    img.save(path).expect("png write failed");
}
