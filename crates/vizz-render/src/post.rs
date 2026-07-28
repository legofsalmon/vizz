//! Post-processing chain: feedback trails, mirroring, glow.
//!
//! The scene renders into its own texture; this turns that into the master
//! output. Two full-screen passes:
//!
//! 1. **Feedback** — mixes the previous result, zoomed and rotated a
//!    little, back in with the new frame. This is the pass that makes the
//!    output look like VJ material: motion smears into trails and a
//!    sustained zoom builds a tunnel out of whatever is on screen.
//! 2. **Composite** — folds UV space for mirror/kaleidoscope modes, adds a
//!    cheap bloom, and tone-maps into the master texture.
//!
//! Feedback needs last frame's result, so two textures ping-pong. Both are
//! `Rgba16Float`: trails accumulate well past 1.0, and 8-bit would band
//! and clip long before the tone-map gets a chance to roll it off.

use crate::GpuContext;

/// Per-frame post inputs. Layout must match `Post` in post.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PostUniforms {
    pub trail: f32,
    pub zoom: f32,
    pub spin: f32,
    pub mirror: f32,
    pub glow: f32,
    pub aspect: f32,
    /// Radial RGB split (chromatic aberration).
    pub shift: f32,
    pub _pad0: f32,
}

/// HDR so sustained feedback has headroom before the tone-map. Scenes
/// must build their pipelines against this, not the output format.
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const HDR: wgpu::TextureFormat = SCENE_FORMAT;

pub struct PostChain {
    feedback_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// The scene draws here; both passes read it.
    pub scene: wgpu::Texture,
    pub scene_view: wgpu::TextureView,
    history: [wgpu::TextureView; 2],
    history_tex: [wgpu::Texture; 2],
    /// Which history slot holds the previous frame.
    front: usize,
}

impl PostChain {
    pub fn new(
        ctx: &GpuContext,
        width: u32,
        height: u32,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });

        let make = |label: &str, format: wgpu::TextureFormat| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let scene = make("post-scene", HDR);
        let scene_view = scene.create_view(&Default::default());
        let history_tex = [make("post-history-0", HDR), make("post-history-1", HDR)];
        let history =
            [history_tex[0].create_view(&Default::default()), history_tex[1].create_view(&Default::default())];

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post-uniforms"),
            size: std::mem::size_of::<PostUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post-pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let build = |entry: &str, format: wgpu::TextureFormat, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
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
            })
        };

        Self {
            feedback_pipeline: build("fs_feedback", HDR, "post-feedback"),
            composite_pipeline: build("fs_composite", output_format, "post-composite"),
            layout,
            uniforms,
            sampler,
            scene,
            scene_view,
            history,
            history_tex,
            front: 0,
        }
    }

    /// Run both passes, writing the finished frame into `output`.
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        uniforms: &PostUniforms,
    ) {
        ctx.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));

        let back = 1 - self.front;
        // Pass 1: scene + previous history -> the other history slot.
        let bind = self.bind(ctx, &self.scene_view, &self.history[self.front]);
        self.pass(encoder, &self.feedback_pipeline, &self.history[back], &bind, "post-feedback");

        // Pass 2: that result -> master. The history texture is bound as
        // the "scene" slot here; the second texture binding is unused but
        // must still be supplied for the layout.
        let bind = self.bind(ctx, &self.history[back], &self.scene_view);
        self.pass(encoder, &self.composite_pipeline, output, &bind, "post-composite");

        self.front = back;
    }

    fn bind(
        &self,
        ctx: &GpuContext,
        a: &wgpu::TextureView,
        b: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(a) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(b) },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    fn pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        target: &wgpu::TextureView,
        bind: &wgpu::BindGroup,
        label: &str,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Every pixel is written by the fullscreen triangle, so
                    // clearing is wasted bandwidth.
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Drop accumulated trails — used when the look needs to start clean.
    pub fn clear_history(&mut self, ctx: &GpuContext, encoder: &mut wgpu::CommandEncoder) {
        for view in &self.history {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post-clear-history"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
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
        let _ = ctx;
        let _ = &self.history_tex;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::ParticleScene;

    const W: u32 = 128;

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

    /// Run the real chain — particles into the post chain into a master in
    /// the output format — and return the master's pixels.
    fn render_chain(ctx: &GpuContext, background: wgpu::Color, glow: f32, trail: f32) -> Vec<u8> {
        let mut post = PostChain::new(ctx, W, W, crate::output::OUTPUT_FORMAT);
        let scene = ParticleScene::new(ctx, SCENE_FORMAT);
        let master = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post-test-master"),
            size: wgpu::Extent3d { width: W, height: W, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::output::OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = master.create_view(&Default::default());
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post-test-readback"),
            size: (W * W * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let cam = crate::camera::Camera { aspect: 1.0, ..Default::default() }.uniforms();
        let uniforms = crate::particles::Uniforms {
            view_proj: cam.view_proj,
            cam_right: cam.right,
            focus: 3.5,
            cam_up: cam.up,
            defocus: 0.0,
            cam_position: cam.position,
            _pad_cam: 0.0,
            time: 0.0,
            aspect: 1.0,
            size: 0.02,
            spread: 1.0,
            hue: 0.5,
            saturation: 0.8,
            brightness: 1.0,
            shape: 0.0,
            morph: 0.0,
            twist: 0.0,
            palette: 0.0,
            color_spread: 0.12,
            color_drive: 0.0,
            cloud_a: 0.0,
            cloud_b: 1.0,
            cloud_morph: 0.0,
            room: Default::default(),
        };
        let post_u = PostUniforms {
            trail,
            zoom: 0.0,
            spin: 0.0,
            mirror: 0.0,
            glow,
            aspect: 1.0,
            shift: 0.0,
            _pad0: 0.0,
        };

        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        scene.render(ctx, &mut enc, &post.scene_view, &uniforms, 20_000, true, background);
        post.render(ctx, &mut enc, &view, &post_u);
        enc.copy_texture_to_buffer(
            master.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * 4),
                    rows_per_image: None,
                },
            },
            master.size(),
        );
        ctx.queue.submit([enc.finish()]);

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

    /// Transparency must survive the *whole* chain, not just the clear.
    ///
    /// This is the one that matters for using vizz as a layer: the output
    /// a Syphon or NDI receiver sees is the master, and it is two full
    /// shader passes downstream of the background. Both of those passes
    /// used to write a hardcoded alpha of 1.0, so the clear colour could
    /// be perfectly transparent and the receiver would still get an opaque
    /// rectangle — which looks exactly like the feature not existing.
    #[test]
    fn transparency_survives_the_post_chain() {
        let Some(ctx) = gpu() else { return };
        let clear = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
        let pixels = render_chain(&ctx, clear, 0.0, 0.0);

        let alphas: Vec<u8> = pixels.chunks_exact(4).map(|p| p[3]).collect();
        let clear_px = alphas.iter().filter(|a| **a < 8).count();
        let covered = alphas.iter().filter(|a| **a > 8).count();
        assert!(
            clear_px > alphas.len() / 4,
            "the master came out opaque: only {clear_px} of {} pixels were transparent",
            alphas.len()
        );
        assert!(covered > 0, "the field did not carry alpha, so the output is empty");
    }

    /// Glow has to widen the alpha halo as well as the colour one.
    ///
    /// A bloom that brightens pixels without covering them looks correct
    /// on a black background and vanishes the instant the output is
    /// composited over anything else — the failure mode is invisible in
    /// the preview window and obvious on the projector.
    #[test]
    fn glow_widens_the_alpha_halo_not_just_the_colour() {
        let Some(ctx) = gpu() else { return };
        let clear = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
        let covered = |glow: f32| {
            render_chain(&ctx, clear, glow, 0.0)
                .chunks_exact(4)
                .filter(|p| p[3] > 8)
                .count()
        };
        let plain = covered(0.0);
        let bloomed = covered(1.0);
        assert!(
            bloomed > plain,
            "glow did not extend coverage: {plain} covered without, {bloomed} with"
        );
    }

    /// The default background must still be completely opaque, since that
    /// is what every existing receiver and the preview window expect.
    #[test]
    fn the_opaque_default_survives_the_post_chain() {
        let Some(ctx) = gpu() else { return };
        let pixels = render_chain(&ctx, crate::particles::SCENE_CLEAR, 0.4, 0.0);
        let holes = pixels.chunks_exact(4).filter(|p| p[3] < 250).count();
        assert_eq!(holes, 0, "{holes} pixels were transparent with an opaque background");
    }
}
