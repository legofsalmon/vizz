//! The graded path: an exposure meter and a mip-chain bloom.
//!
//! Both work on the post chain's HDR history, after feedback and before
//! the composite, and both are skipped outright while `/fx/grade` is 0 —
//! which is what keeps every look saved before grading existed drawing
//! exactly the picture it always drew, at exactly the cost it always had.
//! The composite reads the meter's result and the bloom's top level; the
//! curve itself (AgX) is in post.wgsl.

use crate::GpuContext;

/// Layout must match `Meter` in meter.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeterUniforms {
    pub adapt: f32,
    pub ev: f32,
    pub max_gain: f32,
    pub key: f32,
    pub percentile: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

/// How many bins the meter's histogram has; must match `BINS`.
const BINS: u64 = 128;

/// Where the metered highlight lands before the curve. AgX puts about 1.5
/// on its shoulder: bright, still graded, not yet white.
pub const KEY: f32 = 1.5;
/// Which highlight is metered: the level 1% of lit pixels are above.
pub const PERCENTILE: f32 = 0.99;

/// Most bloom levels. Six halvings of 1080p is a 30-pixel level, which is
/// already a haze across a fifth of the frame; more only costs passes.
const MAX_LEVELS: usize = 6;
/// Smallest level worth drawing.
const MIN_LEVEL_SIZE: u32 = 8;

pub struct Grade {
    meter_layout: wgpu::BindGroupLayout,
    histogram: wgpu::ComputePipeline,
    adapt: wgpu::ComputePipeline,
    meter_uniforms: wgpu::Buffer,
    hist: wgpu::Buffer,
    /// `[exposure, metered log2 gain, lit samples, valid]`, read by the
    /// composite.
    pub exposure: wgpu::Buffer,

    bloom_layout: wgpu::BindGroupLayout,
    down_first: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// Half size, quarter size, and so on.
    levels: Vec<wgpu::TextureView>,
    /// Level i's downsample source is level i-1; level 0's is the frame,
    /// bound per call. Level i's upsample source is level i+1.
    down_binds: Vec<wgpu::BindGroup>,
    up_binds: Vec<wgpu::BindGroup>,
    /// Whether the last frame was graded. The first graded frame after an
    /// ungraded one lands the exposure at once rather than easing in from
    /// whatever it was the last time grading was on.
    was_on: bool,
}

impl Grade {
    pub fn new(ctx: &GpuContext, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let device = &ctx.device;

        // Meter.
        let meter_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/meter.wgsl").into()),
        });
        let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let meter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meter-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(2),
                storage(3),
            ],
        });
        let meter_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meter-pl"),
            bind_group_layouts: &[Some(&meter_layout)],
            immediate_size: 0,
        });
        let compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&meter_pl),
                module: &meter_shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let histogram = compute("cs_histogram");
        let adapt = compute("cs_adapt");
        let meter_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meter-uniforms"),
            size: std::mem::size_of::<MeterUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hist = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meter-histogram"),
            size: BINS * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let exposure = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meter-exposure"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // Unity until metered, and marked invalid so the first metered
        // frame lands rather than eases.
        ctx.queue.write_buffer(&exposure, 0, bytemuck::cast_slice(&[1.0f32, 0.0, 0.0, 0.0]));

        // Bloom.
        let bloom_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/bloom.wgsl").into()),
        });
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom-bgl"),
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
        let bloom_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom-pl"),
            bind_group_layouts: &[Some(&bloom_layout)],
            immediate_size: 0,
        });
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let build = |entry: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&bloom_pl),
                vertex: wgpu::VertexState {
                    module: &bloom_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &bloom_shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
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
        let down_first = build("fs_down_first", None);
        let down = build("fs_down", None);
        let up = build("fs_up", Some(wgpu::BlendState { color: add, alpha: add }));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let mut levels = Vec::new();
        let (mut w, mut h) = (width / 2, height / 2);
        while levels.len() < MAX_LEVELS && w.min(h) >= MIN_LEVEL_SIZE {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bloom-level"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            levels.push(tex.create_view(&Default::default()));
            w /= 2;
            h /= 2;
        }
        // A target too small for even one level still needs something to
        // bind; one texel of black reads as no bloom.
        if levels.is_empty() {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bloom-level"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            levels.push(tex.create_view(&Default::default()));
        }
        let bind = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom-bg"),
                layout: &bloom_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                ],
            })
        };
        let down_binds = levels.iter().take(levels.len().saturating_sub(1)).map(&bind).collect();
        let up_binds = levels.iter().skip(1).map(&bind).collect();

        Self {
            meter_layout,
            histogram,
            adapt,
            meter_uniforms,
            hist,
            exposure,
            bloom_layout,
            down_first,
            down,
            up,
            sampler,
            levels,
            down_binds,
            up_binds,
            was_on: false,
        }
    }

    /// The bloom's top level, summed over every level below it.
    pub fn bloom_view(&self) -> &wgpu::TextureView {
        &self.levels[0]
    }

    /// How many levels were summed into [`Self::bloom_view`]; the composite
    /// divides by it so the bloom carries one frame's worth of light.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Meter `frame` and build the bloom from it.
    ///
    /// Skipped entirely when `on` is false. The meter's state survives the
    /// gap, but the first frame back lands at once (see `was_on`).
    pub fn run(
        &mut self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::TextureView,
        meter: MeterUniforms,
        on: bool,
    ) {
        let first = on && !self.was_on;
        self.was_on = on;
        if !on {
            return;
        }
        let meter = MeterUniforms { adapt: if first { 1.0 } else { meter.adapt }, ..meter };
        ctx.queue.write_buffer(&self.meter_uniforms, 0, bytemuck::bytes_of(&meter));

        let size = frame.texture().size();
        let meter_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meter-bg"),
            layout: &self.meter_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.meter_uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(frame) },
                wgpu::BindGroupEntry { binding: 2, resource: self.hist.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.exposure.as_entire_binding() },
            ],
        });
        encoder.clear_buffer(&self.hist, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meter"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &meter_bind, &[]);
            pass.set_pipeline(&self.histogram);
            // Every other pixel, 16×16 per workgroup.
            pass.dispatch_workgroups(size.width.div_ceil(32), size.height.div_ceil(32), 1);
            pass.set_pipeline(&self.adapt);
            pass.dispatch_workgroups(1, 1, 1);
        }

        // Down the chain from the frame, then back up adding as it goes.
        let frame_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-frame-bg"),
            layout: &self.bloom_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(frame) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        for i in 0..self.levels.len() {
            let (pipeline, bind) = if i == 0 {
                (&self.down_first, &frame_bind)
            } else {
                (&self.down, &self.down_binds[i - 1])
            };
            pass(encoder, pipeline, &self.levels[i], bind, true);
        }
        for i in (0..self.levels.len().saturating_sub(1)).rev() {
            pass(encoder, &self.up, &self.levels[i], &self.up_binds[i], false);
        }
    }
}

fn pass(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    target: &wgpu::TextureView,
    bind: &wgpu::BindGroup,
    clear: bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("bloom"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load },
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
