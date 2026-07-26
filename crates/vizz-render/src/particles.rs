//! The first generator: a fully procedural particle field.
//!
//! All per-particle state is derived in the vertex shader from the vertex
//! index, so per-frame CPU cost is one 32-byte uniform upload and one draw
//! call regardless of particle count.

use crate::{GpuContext, attractor::Attractors};

/// Per-frame shader inputs. Layout must match `Uniforms` in particles.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub time: f32,
    pub aspect: f32,
    pub size: f32,
    pub spread: f32,
    pub hue: f32,
    pub saturation: f32,
    pub brightness: f32,
    /// Geometry mode; fractional part morphs into the next one.
    pub shape: f32,
    pub morph: f32,
    pub twist: f32,
    /// 0 = classic HSV; above that, crossfades through cosine gradients.
    pub palette: f32,
    pub color_spread: f32,
    /// What drives palette position: index, radius, depth or height.
    pub color_drive: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

pub struct ParticleScene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Kept alive for the bind group's texture view.
    _attractors: Attractors,
}

impl ParticleScene {
    pub fn new(ctx: &GpuContext, target_format: wgpu::TextureFormat) -> Self {
        let device = &ctx.device;
        let attractors = Attractors::new(ctx);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/particles.wgsl").into()),
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Precomputed attractor trajectories. Vertex-stage texture
                // reads are universally supported; a storage buffer here
                // would not be, since vertex-stage storage is zero under
                // WebGPU's default limits.
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&attractors.view),
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        // Additive blending: particles accumulate into light.
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle-pipeline"),
            layout: Some(&layout),
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
                    format: target_format,
                    blend: Some(additive),
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
            _attractors: attractors,
        }
    }

    /// Encode one frame into `target`. `count` is the number of particles.
    pub fn render(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        uniforms: &Uniforms,
        count: u32,
    ) {
        ctx.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.004,
                        g: 0.004,
                        b: 0.008,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..count * 6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 128;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// See readback.rs for why a missing adapter is a hard failure under
    /// `VIZZ_REQUIRE_GPU`: a silently skipped GPU test looks exactly like
    /// a passing one.
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

    /// Render one frame at a given `shape` and return the pixels.
    fn render_shape(ctx: &GpuContext, scene: &ParticleScene, shape: f32) -> Vec<u8> {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle-test-target"),
            size: wgpu::Extent3d { width: W, height: W, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle-test-readback"),
            size: (W * W * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let uniforms = Uniforms {
            time: 0.0,
            aspect: 1.0,
            size: 0.02,
            spread: 1.0,
            hue: 0.5,
            saturation: 0.8,
            brightness: 1.0,
            shape,
            morph: 0.0,
            twist: 0.0,
            palette: 0.0,
            color_spread: 0.12,
            color_drive: 0.0,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        };

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        scene.render(ctx, &mut encoder, &view, &uniforms, 20_000);
        // W is 128, so the row stride is already 512 bytes — a multiple of
        // COPY_BYTES_PER_ROW_ALIGNMENT, no padding needed.
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * 4),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        ctx.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let pixels = slice.get_mapped_range().unwrap().to_vec();
        drop(buffer);
        pixels
    }

    fn lit_pixels(px: &[u8]) -> usize {
        px.chunks_exact(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 24).count()
    }

    /// The attractor modes read their geometry from a texture the CPU
    /// filled, through a binding the parametric modes never touch. If that
    /// binding, the row indexing or the trajectory upload were wrong, the
    /// mode would render as empty or as something indistinguishable from
    /// the sphere — so assert it is both non-empty and materially
    /// different.
    #[test]
    fn attractor_modes_render_distinct_geometry() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);

        let sphere = render_shape(&ctx, &scene, 0.0);
        let lorenz = render_shape(&ctx, &scene, 5.0);
        let aizawa = render_shape(&ctx, &scene, 6.0);

        for (name, px) in [("lorenz", &lorenz), ("aizawa", &aizawa)] {
            let lit = lit_pixels(px);
            assert!(lit > 200, "{name} rendered almost nothing: {lit} lit pixels");
        }

        // Distinct from the sphere, and from each other.
        for (a, b, pair) in [
            (&sphere, &lorenz, "sphere/lorenz"),
            (&sphere, &aizawa, "sphere/aizawa"),
            (&lorenz, &aizawa, "lorenz/aizawa"),
        ] {
            let diff = a
                .iter()
                .zip(b.iter())
                .filter(|(x, y)| x.abs_diff(**y) > 16)
                .count();
            assert!(diff > 1_000, "{pair} render near-identically: {diff} differing bytes");
        }
    }
}
