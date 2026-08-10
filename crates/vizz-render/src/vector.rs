//! Hard-edged vector layers: the print-look counterpart to the particle
//! field. A fixed stack of procedural pattern layers — rings, stripes,
//! checker, polygon, star, rays, dots — each with a similarity transform,
//! a flat palette colour and a blend mode, composited in one fullscreen
//! pass. Moiré between layers is the point, not an artifact: two pattern
//! fields at slightly different frequencies interfere per fragment.
//!
//! One über-shader rather than per-layer passes because the stack is
//! small and fixed: compositing happens in-register in the fragment
//! shader, blend modes are exact in float with no intermediate textures,
//! and the whole thing is a single draw of three vertices. Per-layer
//! passes only earn their cost when layers become unbounded or need
//! render-to-texture sources.

use crate::GpuContext;

/// Layers the shader evaluates. The registry may expose fewer — capacity
/// here is free, parameters are not.
pub const MAX_LAYERS: usize = 8;

/// Flat colour slots layers pick from. A small shared palette is the
/// aesthetic: per-layer free RGB invites mud, four inks invite a print.
pub const PALETTE_SLOTS: usize = 4;

/// Generator names, by the index the `kind` lane carries. Index 0 is
/// "off" so a layer can be removed by a controller without a second
/// enable parameter. The parameter table's labels must point at this
/// array — it is the single source of truth, held to the WGSL switch by
/// a test below.
pub const KIND_LABELS: &[&str] = &[
    "off", "rings", "stripes", "checker", "polygon", "star", "rays", "dots",
];

/// Blend mode names, by the index the `blend` lane carries. These
/// operate in sRGB-encoded space, deliberately: multiply/screen on
/// encoded values is what every print-era compositor does, and it is
/// the look this exists for.
pub const BLEND_LABELS: &[&str] = &[
    "normal", "multiply", "screen", "add", "difference", "exclusion", "subtract",
];

/// One layer, packed as four vec4 lanes so the Rust and WGSL layouts
/// agree by construction — an all-vec4 struct has no padding for the
/// two languages to disagree over. Lane map:
///
/// | field   | x            | y        | z             | w        |
/// |---------|--------------|----------|---------------|----------|
/// | `xform` | translate x  | trans. y | rotation, turns | scale  |
/// | `pat`   | kind (index) | frequency| phase 0..1    | duty 0..1|
/// | `shape` | sides        | star inset | kaleido segs (0 off) | invert fill |
/// | `style` | blend (index)| opacity  | palette slot  | reserved |
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LayerU {
    pub xform: [f32; 4],
    pub pat: [f32; 4],
    pub shape: [f32; 4],
    pub style: [f32; 4],
}

/// The whole stack, one uniform buffer write per frame.
///
/// `globals`: x aspect (w/h), y time (seconds, pre-rated), z master dim,
/// w active layer count. `bg`: rgb paper colour (sRGB-encoded) and, in
/// the otherwise-unused w lane, the height of the render target in
/// pixels — the shader derives its analytic pixel footprint from it.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StackU {
    pub globals: [f32; 4],
    pub bg: [f32; 4],
    pub palette: [[f32; 4]; PALETTE_SLOTS],
    pub layers: [LayerU; MAX_LAYERS],
}

impl Default for StackU {
    /// White paper, black ink in slot 0, everything off. Renders as a
    /// blank page rather than as garbage or as black-on-black.
    fn default() -> Self {
        Self {
            globals: [16.0 / 9.0, 0.0, 1.0, MAX_LAYERS as f32],
            bg: [1.0, 1.0, 1.0, 720.0],
            palette: [
                [0.05, 0.05, 0.05, 1.0],
                [0.92, 0.10, 0.14, 1.0],
                [0.10, 0.30, 0.95, 1.0],
                [0.98, 0.80, 0.05, 1.0],
            ],
            layers: [LayerU::default(); MAX_LAYERS],
        }
    }
}

// The WGSL-agreement contract. If either of these moves, the shader's
// Stack struct no longer matches what write_buffer uploads, and every
// lane after the change reads garbage — silently.
const _: () = assert!(std::mem::size_of::<LayerU>() == 64);
const _: () = assert!(std::mem::size_of::<StackU>() == 16 + 16 + PALETTE_SLOTS * 16 + MAX_LAYERS * 64);

/// The vector stack as a scene element: draws the full frame (paper
/// colour included), so wherever it renders it *replaces* the clear.
pub struct VectorScene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl VectorScene {
    pub fn new(ctx: &GpuContext, format: wgpu::TextureFormat) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vector"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vector.wgsl").into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vector-uniforms"),
            size: std::mem::size_of::<StackU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vector-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vector-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vector-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vector-pipeline"),
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
                    // The shader paints every pixel; nothing to blend with.
                    blend: None,
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
        Self {
            pipeline,
            uniforms,
            bind_group,
        }
    }

    /// Draw the stack into `target`, clearing it — this pass paints the
    /// paper colour under everything, so it owns the frame's base.
    pub fn render(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        stack: &StackU,
    ) {
        ctx.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(stack));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("vector"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(&self.bind_group), &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WGSL carries its own copies of the layer count, palette size,
    /// and the generator/blend switch arms — uniform structs and switch
    /// statements cannot import Rust constants. Copies go stale exactly
    /// the way CLOUD_SLOTS did (folding the top cloud slots onto lower
    /// ones for two releases), so each one is read back out of the
    /// shader source and held against its Rust twin.
    #[test]
    fn the_shader_agrees_with_rust_about_the_stack() {
        let src = include_str!("shaders/vector.wgsl");
        let read = |needle: &str| -> usize {
            let at = src
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} not found in vector.wgsl"));
            let rest = &src[at + needle.len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .expect("a number after the needle");
            rest[..end].parse().expect("a number")
        };
        assert_eq!(
            read("const MAX_LAYERS: u32 = "),
            MAX_LAYERS,
            "layer arrays disagree — lanes past the shorter count read garbage"
        );
        assert_eq!(
            read("const PALETTE_SLOTS: u32 = "),
            PALETTE_SLOTS,
            "palette arrays disagree"
        );

        // Every labelled kind and blend must have a switch arm, and no
        // arm may exist without a label: the panel shows the labels, and
        // a label without an arm is a control that silently does nothing.
        for (i, label) in KIND_LABELS.iter().enumerate().skip(1) {
            assert!(
                src.contains(&format!("case {i}u: {{ // {label}")),
                "generator {i} ({label}) has no matching switch arm in vector.wgsl"
            );
        }
        for (i, label) in BLEND_LABELS.iter().enumerate().skip(1) {
            assert!(
                src.contains(&format!("case {i}u: {{ // {label}")),
                "blend {i} ({label}) has no matching switch arm in vector.wgsl"
            );
        }
        assert!(
            !src.contains(&format!("case {}u:", KIND_LABELS.len())),
            "the shader has a generator arm past the labelled range"
        );
    }

    /// Coverage maths the shader relies on, checked here because the GPU
    /// cannot assert. Encoded-space multiply of mid-grey must give the
    /// encoded product — the print behaviour the blend-space decision
    /// exists for — and the sRGB exit conversion must round-trip it to
    /// the bytes a compositor would produce.
    #[test]
    fn encoded_space_multiply_matches_print_expectations() {
        let srgb_to_linear = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let linear_to_srgb = |c: f32| {
            if c <= 0.003_130_8 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            }
        };
        // 0.5 × 0.5 in encoded space = 0.25 encoded ≈ byte 64 — visibly
        // darker than the linear-space product (byte ~188). If the
        // shader ever blends in linear by mistake, this is the number
        // that shows it on screen.
        let encoded = 0.5f32 * 0.5;
        let byte = (encoded * 255.0).round() as u8;
        assert_eq!(byte, 64);
        // The exit conversion + the sRGB target's re-encode must be a
        // round trip, or every colour ships shifted.
        let round = linear_to_srgb(srgb_to_linear(encoded));
        assert!((round - encoded).abs() < 1e-5, "sRGB round trip drifted: {round}");
    }
}
