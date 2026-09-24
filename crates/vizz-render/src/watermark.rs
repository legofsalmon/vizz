//! The licence mark, drawn into the master output.
//!
//! "VIZZ · UNLICENSED" in a dark capsule across the lower part of the
//! frame. Into the master rather than over the preview, because the
//! master is the show: it is what Syphon and NDI carry, what a recording
//! writes and what the preview and the panel's thumbnail are drawn from.
//! A mark anywhere else would be a mark on the one screen that does not
//! matter.
//!
//! What it costs a frame is one small draw: a pass that loads the master
//! and shades only the capsule's rectangle. The lettering is rasterised on
//! the CPU once, when the mark is made — at exactly the size it is drawn,
//! so one texel is one output pixel and nothing is resampled — and a
//! master of a new size makes a new mark. Nothing here reads back, waits
//! or allocates per frame.

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};

/// The capsule's height as a share of the output's. Big enough that it
/// cannot be missed at the back of a room, small enough to leave the
/// picture readable round it.
const HEIGHT_SHARE: f32 = 0.075;
/// Where its centre sits, from the top.
const CENTRE_FROM_TOP: f32 = 0.82;
/// Widest it may be, as a share of the output — a tall narrow canvas gets
/// a smaller mark rather than one that runs off the sides.
const MAX_WIDTH_SHARE: f32 = 0.9;
/// Never smaller than this, however small the output.
const MIN_HEIGHT: u32 = 18;

/// The lettering's coverage and where it goes. Pure CPU, so it can be
/// checked without a GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    /// Top-left corner in the output, in pixels.
    pub x: u32,
    pub y: u32,
    /// One byte of coverage per pixel, row-major.
    pub pixels: Vec<u8>,
}

impl Mask {
    /// `[x, y, width, height]` in output pixels.
    pub fn rect(&self) -> [u32; 4] {
        [self.x, self.y, self.width, self.height]
    }
}

/// Lay out and rasterise `text` for an output of `out_w × out_h`.
pub fn rasterise(text: &str, out_w: u32, out_h: u32) -> Mask {
    // The panel's own face; shipped bytes, so failing to parse them is a
    // build defect rather than a runtime condition.
    let font =
        FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("the shipped Ubuntu-Light parses");

    let wanted = ((out_h as f32 * HEIGHT_SHARE).round() as u32).max(MIN_HEIGHT);
    let (w, h) = measure(&font, text, wanted as f32);
    // Too wide for the canvas: shrink the whole mark, keeping its shape.
    let limit = out_w as f32 * MAX_WIDTH_SHARE;
    let height =
        if w > limit { ((h * limit / w).floor() as u32).max(MIN_HEIGHT.min(out_h.max(1))) } else { h as u32 };
    let (width, height) = {
        let (w, h) = measure(&font, text, height as f32);
        ((w.ceil() as u32).clamp(1, out_w.max(1)), (h as u32).clamp(1, out_h.max(1)))
    };

    let mut pixels = vec![0u8; (width * height) as usize];
    draw_text(&font, text, height as f32, width, height, &mut pixels);

    let x = out_w.saturating_sub(width) / 2;
    let centre = (out_h as f32 * CENTRE_FROM_TOP).round() as u32;
    let y = centre.saturating_sub(height / 2).min(out_h.saturating_sub(height));
    Mask { width, height, x, y, pixels }
}

/// Glyph size, letter spacing and side padding for a capsule this tall.
fn metrics(height: f32) -> (f32, f32, f32) {
    let scale = height * 0.58;
    let tracking = scale * 0.16;
    let pad = height * 0.62;
    (scale, tracking, pad)
}

/// The capsule's size for a given height: the lettering plus padding.
fn measure(font: &FontRef<'_>, text: &str, height: f32) -> (f32, f32) {
    let (scale, tracking, pad) = metrics(height);
    let scaled = font.as_scaled(PxScale::from(scale));
    let mut pen = 0.0;
    let mut n = 0;
    for c in text.chars() {
        pen += scaled.h_advance(scaled.glyph_id(c));
        n += 1;
    }
    let text_w = pen + tracking * (n.max(1) - 1) as f32;
    (text_w + 2.0 * pad, height)
}

fn draw_text(font: &FontRef<'_>, text: &str, height: f32, w: u32, h: u32, out: &mut [u8]) {
    let (scale, tracking, _) = metrics(height);
    let scaled = font.as_scaled(PxScale::from(scale));

    // Lay the line out on a baseline at zero, and take the union of the
    // ink's bounds, so the lettering can be centred on what is actually
    // drawn rather than on the font's ascent — capitals and a middle dot
    // sit well below the ascent line.
    let mut glyphs = Vec::new();
    let (mut pen, mut prev) = (0.0f32, None);
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(p) = prev {
            pen += scaled.kern(p, id);
        }
        glyphs.push(id.with_scale_and_position(scale, ab_glyph::point(pen, 0.0)));
        pen += scaled.h_advance(id) + tracking;
        prev = Some(id);
    }
    let outlined: Vec<_> = glyphs.into_iter().filter_map(|g| font.outline_glyph(g)).collect();
    let Some(bounds) = outlined.iter().map(|g| g.px_bounds()).reduce(|a, b| ab_glyph::Rect {
        min: ab_glyph::point(a.min.x.min(b.min.x), a.min.y.min(b.min.y)),
        max: ab_glyph::point(a.max.x.max(b.max.x), a.max.y.max(b.max.y)),
    }) else {
        return;
    };
    let dx = ((w as f32 - bounds.width()) * 0.5 - bounds.min.x).round();
    let dy = ((h as f32 - bounds.height()) * 0.5 - bounds.min.y).round();

    for g in &outlined {
        let b = g.px_bounds();
        g.draw(|gx, gy, coverage| {
            let x = (b.min.x + dx) as i64 + gx as i64;
            let y = (b.min.y + dy) as i64 + gy as i64;
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                return;
            }
            let i = (y as u32 * w + x as u32) as usize;
            let v = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i] = out[i].max(v);
        });
    }
    // The shipped face is a light weight, which reads as a whisper at the
    // back of a room. A small dilation embolden it without a second font.
    let radius = (scale * 0.028).round().max(1.0) as usize;
    dilate(out, w as usize, h as usize, radius);
}

/// Max filter, separable, in place.
fn dilate(px: &mut [u8], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0u8; px.len()];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            tmp[y * w + x] = px[y * w + a..=y * w + b].iter().copied().max().unwrap_or(0);
        }
    }
    for y in 0..h {
        let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            px[y * w + x] = (a..=b).map(|yy| tmp[yy * w + x]).max().unwrap_or(0);
        }
    }
}

/// The mark on the GPU: its lettering, and the pipeline that composites it.
pub struct Watermark {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    rect: [u32; 4],
    // Held so the view in `bind` stays valid.
    _mask: wgpu::Texture,
}

impl Watermark {
    /// Make the mark for a master of this size and format. Built once, and
    /// again whenever the master is rebuilt; never per frame.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        text: &str,
        out_w: u32,
        out_h: u32,
    ) -> Self {
        let mask = rasterise(text, out_w, out_h);
        let size = wgpu::Extent3d { width: mask.width, height: mask.height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("watermark-mask"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &mask.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mask.width),
                rows_per_image: Some(mask.height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("watermark"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/watermark.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("watermark-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // Nearest: the mask is drawn one texel to one pixel.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("watermark-sampler"),
            ..Default::default()
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("watermark-bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("watermark-pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("watermark-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, bind, rect: mask.rect(), _mask: texture }
    }

    /// Where the mark lands, `[x, y, width, height]` in output pixels.
    pub fn rect(&self) -> [u32; 4] {
        self.rect
    }

    /// Composite the mark over whatever `master` already holds.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, master: &wgpu::TextureView) {
        let [x, y, w, h] = self.rect;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("watermark"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: master,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.draw(0..3, 0..1);
    }
}

/// Clear the master to black: what a locked session publishes, and what
/// its preview shows behind the licence panel.
pub fn blackout(encoder: &mut wgpu::CommandEncoder, master: &wgpu::TextureView) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("licence-blackout"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: master,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GpuContext;

    const TEXT: &str = "VIZZ · UNLICENSED";

    fn lit(mask: &Mask) -> usize {
        mask.pixels.iter().filter(|&&p| p > 128).count()
    }

    /// Every output shape the app allows, from the smallest to the widest:
    /// the mark is inside the frame, centred, in the lower part, and has
    /// lettering in it.
    #[test]
    fn the_mark_fits_every_output_the_app_allows() {
        for (w, h) in [
            (160, 160),
            (320, 240),
            (1280, 720),
            (1920, 1080),
            (3840, 2160),
            (8192, 4320),
            (8192, 1080),
            (1080, 1920),
            (160, 8192),
        ] {
            let m = rasterise(TEXT, w, h);
            assert!(m.x + m.width <= w && m.y + m.height <= h, "{w}x{h}: {:?} leaves the frame", m.rect());
            assert!(m.width <= w, "{w}x{h}");
            let centre_x = m.x + m.width / 2;
            assert!(centre_x.abs_diff(w / 2) <= 1, "{w}x{h}: not centred");
            assert!(m.y >= h / 2, "{w}x{h}: in the top half");
            assert!(lit(&m) > 50, "{w}x{h}: no lettering drawn");
            assert_eq!(m.pixels.len(), (m.width * m.height) as usize);
        }
    }

    #[test]
    fn at_1080p_it_is_big_enough_to_read_and_leaves_the_picture_alone() {
        let m = rasterise(TEXT, 1920, 1080);
        assert!(m.height >= 70, "{} px tall is easy to miss", m.height);
        assert!(m.width > m.height * 5, "not a single line");
        let share = (m.width * m.height) as f32 / (1920.0 * 1080.0);
        assert!(share < 0.06, "covers {:.0}% of the frame", share * 100.0);
    }

    /// The lettering is inside the capsule, clear of its rounded ends.
    #[test]
    fn the_lettering_stays_clear_of_the_ends() {
        let m = rasterise(TEXT, 1920, 1080);
        let edge = (m.height / 2) as usize;
        for y in 0..m.height as usize {
            let row = &m.pixels[y * m.width as usize..(y + 1) * m.width as usize];
            assert!(row[..edge].iter().all(|&p| p == 0), "ink in the left end, row {y}");
            assert!(row[row.len() - edge..].iter().all(|&p| p == 0), "ink in the right end, row {y}");
        }
    }

    fn gpu() -> Option<GpuContext> {
        match pollster::block_on(GpuContext::new(None)) {
            Ok(ctx) => Some(ctx),
            Err(_) if std::env::var_os("VIZZ_REQUIRE_GPU").is_some() => {
                panic!("VIZZ_REQUIRE_GPU is set but no GPU adapter was found")
            }
            Err(_) => {
                eprintln!("no GPU adapter available; skipping GPU test");
                None
            }
        }
    }

    /// Draw the mark over a flat master of `fill` and read the master back.
    fn render(ctx: &GpuContext, fill: wgpu::Color, w: u32, h: u32) -> (Vec<u8>, [u32; 4]) {
        let format = crate::output::OUTPUT_FORMAT;
        let master = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("watermark-test-master"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = master.create_view(&Default::default());
        let mark = Watermark::new(&ctx.device, &ctx.queue, format, TEXT, w, h);
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(fill), store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        mark.draw(&mut encoder, &view);
        let padded =
            (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            master.as_image_copy(),
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
        ctx.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = slice.get_mapped_range().unwrap();
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let start = (row * padded) as usize;
            px.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        (px, mark.rect())
    }

    fn inside(rect: [u32; 4], x: u32, y: u32) -> bool {
        x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3]
    }

    /// On black, the letters show; on white, the bed does. Outside the
    /// capsule's rectangle not one pixel changes.
    #[test]
    fn the_mark_shows_on_black_and_on_white_and_touches_nothing_else() {
        let Some(ctx) = gpu() else { return };
        let (w, h) = (640, 360);

        let (black, rect) = render(&ctx, wgpu::Color::BLACK, w, h);
        let mut bright_inside = 0;
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let px = &black[i..i + 3];
                if inside(rect, x, y) {
                    if px.iter().all(|&c| c > 200) {
                        bright_inside += 1;
                    }
                } else {
                    assert_eq!(px, [0, 0, 0], "the mark touched ({x},{y}), outside {rect:?}");
                }
            }
        }
        assert!(bright_inside > 200, "only {bright_inside} lettering pixels on black");

        let (white, rect) = render(&ctx, wgpu::Color::WHITE, w, h);
        let mut dark_inside = 0;
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let px = &white[i..i + 3];
                if inside(rect, x, y) {
                    if px.iter().all(|&c| c < 200) {
                        dark_inside += 1;
                    }
                } else {
                    assert_eq!(px, [255, 255, 255], "the mark touched ({x},{y}), outside {rect:?}");
                }
            }
        }
        assert!(dark_inside > 1000, "only {dark_inside} bed pixels on white — unreadable there");
    }
}
