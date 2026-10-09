//! Haze and light beams: a lit medium between the camera and the scene.
//!
//! The space in front of the camera is cut into froxels, a grid that is a
//! pixel grid across and down and slices of distance deep. Each froxel
//! holds how thick the medium is there: a little haze everywhere, and more
//! wherever the particles are, so a cloud becomes smoke. Every froxel is
//! then lit by the lamps and the sun, each light dimmed by the smoke
//! between it and the froxel, which is what cuts beams and shafts; the
//! light is integrated front to back along each column, and the frame is
//! dimmed by what the haze hides and brightened by what it scatters.
//!
//! The volume is the froxel scheme of Wroński, "Volumetric Fog and
//! Lighting", SIGGRAPH 2014 Advances in Real-Time Rendering, with the
//! per-slice step of Hillaire, "Physically Based and Unified Volumetric
//! Rendering in Frostbite", SIGGRAPH 2015; the scattering is single
//! scattering through an absorbing medium as Max sets it out in "Optical
//! Models for Direct Volume Rendering", IEEE TVCG 1995, with the
//! Henyey-Greenstein phase function (Astrophysical Journal, 1941).

use glam::Mat4;

use crate::GpuContext;
use crate::particles::Uniforms;

/// The shader: the particle shader with the haze passes appended.
pub const SOURCE: &str = concat!(
    include_str!("shaders/particles.wgsl"),
    "\n",
    include_str!("shaders/haze.wgsl"),
);

/// Froxels across, down and deep. 160 by 90 is a froxel per 12 pixels of
/// a 1080p frame, which the linear filter blends out; 64 slices spaced by
/// log distance give a near slice a few centimetres deep and a far one
/// half a unit, about what the eye resolves at each.
pub const DIMS: [u32; 3] = [160, 90, 64];

/// Most particles counted into the volume: as many as one dimension of
/// dispatch reaches.
const MAX_COUNTED: u32 = 65_535 * 64;

/// What `/haze/density`, `/haze/smoke` and `/haze/scatter` ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Haze {
    /// 0..1: how thick the medium is; 0 is off.
    pub density: f32,
    /// How much the particles thicken it, 0 for not at all.
    pub smoke: f32,
    /// 0..0.95: how much of the light scatters forwards, so a lamp seen
    /// through the haze glows more than one seen from the side.
    pub scatter: f32,
}

/// Must match `Haze` in haze.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HazeUniforms {
    pub inv_view_proj: [[f32; 4]; 4],
    pub dims: [u32; 4],
    pub near: f32,
    pub far: f32,
    pub density: f32,
    pub smoke: f32,
    pub scatter: f32,
    pub use_depth: f32,
    pub no_depth: f32,
    pub _pad: f32,
}

impl HazeUniforms {
    /// For a frame drawn with `uniforms`, `particles` of them counted in.
    pub fn new(haze: Haze, uniforms: &Uniforms, particles: u32, use_depth: bool) -> Self {
        let inv = Mat4::from_cols_array_2d(&uniforms.view_proj).inverse();
        // Far enough to take in the cloud and a room behind it: the camera
        // looks at the focus, and the scene seldom reaches past two and a
        // half times as far.
        let focus = uniforms.focus.max(0.1);
        let far = (focus * 2.5).max(8.0);
        Self {
            inv_view_proj: inv.to_cols_array_2d(),
            dims: [DIMS[0], DIMS[1], DIMS[2], particles.min(MAX_COUNTED)],
            near: 0.1,
            far,
            density: haze.density.clamp(0.0, 1.0),
            smoke: haze.smoke.max(0.0),
            scatter: haze.scatter.clamp(0.0, 0.95),
            use_depth: if use_depth { 1.0 } else { 0.0 },
            // With no depth to stop at, the frame is taken as lying at the
            // focus: the haze in front of the cloud covers it, and the
            // haze behind it is behind it.
            no_depth: focus,
            _pad: 0.0,
        }
    }
}

pub(crate) struct HazePass {
    clear: wgpu::ComputePipeline,
    count: wgpu::ComputePipeline,
    density: wgpu::ComputePipeline,
    light: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    apply: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    count_bg: wgpu::BindGroup,
    density_bg: wgpu::BindGroup,
    light_bg: wgpu::BindGroup,
    integrate_bg: wgpu::BindGroup,
    apply_bgl: wgpu::BindGroupLayout,
    sum: wgpu::TextureView,
    sampler: wgpu::Sampler,
    /// Stands in for the depth when the frame has none.
    no_depth: wgpu::TextureView,
}

fn buffer(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
}

fn froxels() -> u32 {
    DIMS[0] * DIMS[1] * DIMS[2]
}

impl HazePass {
    pub fn new(
        ctx: &GpuContext,
        particle_bgl: &wgpu::BindGroupLayout,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("haze"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("haze-uniforms"),
            size: std::mem::size_of::<HazeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("haze-counts"),
            size: froxels() as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let volume = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: DIMS[0],
                        height: DIMS[1],
                        depth_or_array_layers: DIMS[2],
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let thickness = volume("haze-density");
        let light = volume("haze-light");
        let sum = volume("haze-sum");
        let no_depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("haze-no-depth"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("haze-linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let uniform = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let counts_entry = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let write = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let read = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        let linear = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let compute = wgpu::ShaderStages::COMPUTE;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        // A layout per pass: each writes what the next reads, and a
        // texture may not be written and read in the same dispatch.
        let bgl = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let count_bgl = bgl("haze-count-bgl", &[uniform, counts_entry(1, false)]);
        let density_bgl = bgl("haze-density-bgl", &[uniform, counts_entry(10, true), write(2)]);
        let light_bgl = bgl("haze-light-bgl", &[uniform, read(3, compute), linear(compute), write(5)]);
        let integrate_bgl = bgl("haze-integrate-bgl", &[uniform, read(6, compute), write(7)]);
        let apply_bgl = bgl(
            "haze-apply-bgl",
            &[
                uniform,
                read(8, fragment),
                linear(fragment),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        );

        let view = |binding, v| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(v),
        };
        let filter = wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::Sampler(&sampler),
        };
        let bg = |label, layout, entries: &[wgpu::BindGroupEntry]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries,
            })
        };
        let count_bg = bg("haze-count-bg", &count_bgl, &[buffer(0, &uniforms), buffer(1, &counts)]);
        let density_bg = bg(
            "haze-density-bg",
            &density_bgl,
            &[buffer(0, &uniforms), buffer(10, &counts), view(2, &thickness)],
        );
        let light_bg = bg(
            "haze-light-bg",
            &light_bgl,
            &[buffer(0, &uniforms), view(3, &thickness), filter.clone(), view(5, &light)],
        );
        let integrate_bg = bg(
            "haze-integrate-bg",
            &integrate_bgl,
            &[buffer(0, &uniforms), view(6, &light), view(7, &sum)],
        );

        let pipe = |label, layout: &wgpu::BindGroupLayout, entry| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(particle_bgl), Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let clear = pipe("haze-clear", &count_bgl, "cs_haze_clear");
        let count = pipe("haze-count", &count_bgl, "cs_haze_count");
        let density = pipe("haze-density", &density_bgl, "cs_haze_density");
        let light_pipe = pipe("haze-light", &light_bgl, "cs_haze_light");
        let integrate = pipe("haze-integrate", &integrate_bgl, "cs_haze_integrate");

        let apply_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("haze-apply-pl"),
            bind_group_layouts: &[Some(particle_bgl), Some(&apply_bgl)],
            immediate_size: 0,
        });
        let apply = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("haze-apply"),
            layout: Some(&apply_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_haze"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_haze"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    // out = scattered + scene * transmittance. The alpha
                    // is the scene's: haze is not coverage.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::SrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
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
            clear,
            count,
            density,
            light: light_pipe,
            integrate,
            apply,
            uniforms,
            count_bg,
            density_bg,
            light_bg,
            integrate_bg,
            apply_bgl,
            sum,
            sampler,
            no_depth,
        }
    }

    /// Fill the volume for this frame and lay it over `target`. `depth` is
    /// the surface mode's depth for the frame, when it has one, so the
    /// haze stops at each surface; without it the frame is taken to lie
    /// at the focus.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        particle_bg: &wgpu::BindGroup,
        target: &wgpu::TextureView,
        uniforms: &Uniforms,
        count: u32,
        haze: Haze,
        depth: Option<&wgpu::TextureView>,
    ) {
        if haze.density <= 0.0 {
            return;
        }
        // A stroke is several quads of one particle; count the particle.
        let segs = (uniforms.stroke[2] + 0.5).max(1.0) as u32;
        let hu = HazeUniforms::new(haze, uniforms, count / segs, depth.is_some());
        ctx.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&hu));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("haze-volume"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, particle_bg, &[]);
            pass.set_bind_group(1, &self.count_bg, &[]);
            pass.set_pipeline(&self.clear);
            pass.dispatch_workgroups(froxels().div_ceil(64), 1, 1);
            if hu.dims[3] > 0 && hu.smoke > 0.0 {
                pass.set_pipeline(&self.count);
                pass.dispatch_workgroups(hu.dims[3].div_ceil(64), 1, 1);
            }
            let cells = [DIMS[0].div_ceil(4), DIMS[1].div_ceil(4), DIMS[2].div_ceil(4)];
            pass.set_bind_group(1, &self.density_bg, &[]);
            pass.set_pipeline(&self.density);
            pass.dispatch_workgroups(cells[0], cells[1], cells[2]);
            pass.set_bind_group(1, &self.light_bg, &[]);
            pass.set_pipeline(&self.light);
            pass.dispatch_workgroups(cells[0], cells[1], cells[2]);
            pass.set_bind_group(1, &self.integrate_bg, &[]);
            pass.set_pipeline(&self.integrate);
            pass.dispatch_workgroups(DIMS[0].div_ceil(8), DIMS[1].div_ceil(8), 1);
        }
        // Made per frame: the depth it reads is whichever the frame had.
        let apply_bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("haze-apply-bg"),
            layout: &self.apply_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniforms.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&self.sum),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(depth.unwrap_or(&self.no_depth)),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("haze-apply"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.apply);
        pass.set_bind_group(0, particle_bg, &[]);
        pass.set_bind_group(1, &apply_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniforms_match_the_shader_and_clamp() {
        assert_eq!(std::mem::size_of::<HazeUniforms>(), 112);
        let mut u: Uniforms = bytemuck::Zeroable::zeroed();
        u.view_proj = Mat4::IDENTITY.to_cols_array_2d();
        u.focus = 3.5;
        let h = HazeUniforms::new(Haze { density: 3.0, smoke: -1.0, scatter: 1.0 }, &u, 10, false);
        assert_eq!((h.density, h.smoke, h.scatter), (1.0, 0.0, 0.95));
        assert!(h.far >= 8.0 && h.near < h.far);
        assert_eq!(h.dims, [DIMS[0], DIMS[1], DIMS[2], 10]);
    }
}
