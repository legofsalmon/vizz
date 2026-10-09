//! Gaussian splats: every particle drawn as a soft, oriented ellipse of
//! colour, blended over the frame from the farthest to the nearest.
//!
//! A third way to draw, beside the glow and the surface mode. A cloud
//! loaded from a 3D Gaussian Splatting capture (Kerbl, Kopanas,
//! Leimkühler & Drettakis, SIGGRAPH 2023) keeps each splat's own size,
//! turn, opacity and colour, so a phone capture of a place or a person
//! renders as the photograph it was trained from; anything else is drawn
//! as round splats of the particle size, which reads as soft, overlapping
//! paint rather than light.
//!
//! Three passes a frame. A compute pass evaluates every particle with the
//! dots' own shader functions, carries its ellipsoid to the screen as a 2D
//! Gaussian (EWA splatting, Zwicker et al. 2001) and writes its depth as a
//! sort key. A bitonic sort (Batcher 1968) orders them far to near, one
//! dispatch a step. A draw then lays a quad over each, nearest last, with
//! premultiplied "over" blending, which is why they have to be sorted.
//!
//! Only the colour's zeroth spherical-harmonic band is used, so a capture's
//! colour does not change as the view turns, and the palette tints it as
//! it tints any scan.

use crate::GpuContext;

/// The shader: the particle shader with the splat passes appended.
pub const SOURCE: &str = concat!(
    include_str!("shaders/particles.wgsl"),
    "\n",
    include_str!("shaders/splat.wgsl"),
);

/// Most splats sorted: the particle count's ceiling, as a power of two.
pub const MAX: u32 = 1 << 19;

/// Bytes per projected splat. Must match `Projected` in splat.wgsl.
const PROJECTED_BYTES: u64 = 48;

/// Must match `SplatPass` in splat.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SplatUniforms {
    count: u32,
    padded: u32,
    _pad: [u32; 2],
}

/// Must match `SortStep` in splat.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SortStep {
    k: u32,
    j: u32,
    _pad: [u32; 2],
}

/// Room for every step of a sort of [`MAX`]: 19 · 20 / 2 = 190.
const MAX_STEPS: u64 = 256;
/// One step per this many bytes, for the dynamic offset.
const STEP_STRIDE: u64 = 256;

fn whole(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
}

/// The steps of a bitonic sort of `n` (a power of two), in order.
fn sort_steps(n: u32) -> Vec<SortStep> {
    let mut steps = Vec::new();
    let mut k = 2;
    while k <= n {
        let mut j = k / 2;
        while j > 0 {
            steps.push(SortStep { k, j, _pad: [0; 2] });
            j /= 2;
        }
        k *= 2;
    }
    steps
}

pub(crate) struct SplatPass {
    project: wgpu::ComputePipeline,
    sort: wgpu::ComputePipeline,
    draw: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    compute_bg: wgpu::BindGroup,
    draw_bg: wgpu::BindGroup,
    step_bg: wgpu::BindGroup,
    /// The sort length the step buffer was last filled for.
    steps_for: std::cell::Cell<u32>,
    steps: wgpu::Buffer,
}

impl SplatPass {
    pub fn new(
        ctx: &GpuContext,
        particle_bgl: &wgpu::BindGroupLayout,
        attractors: &crate::attractor::Attractors,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("splat"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
        };
        let uniforms = buffer(
            "splat-uniforms",
            std::mem::size_of::<SplatUniforms>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let projected = buffer("splat-projected", MAX as u64 * PROJECTED_BYTES, wgpu::BufferUsages::STORAGE);
        let keys = buffer("splat-keys", MAX as u64 * 4, wgpu::BufferUsages::STORAGE);
        let order = buffer("splat-order", MAX as u64 * 4, wgpu::BufferUsages::STORAGE);
        let steps = buffer(
            "splat-sort-steps",
            MAX_STEPS * STEP_STRIDE,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );

        let uniform_entry = |binding, visibility, dynamic| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        };
        let storage = |binding, read_only, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bank = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let all = wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE;
        // Two layouts over the same buffers, as for the plexus: the
        // compute passes write them and the draw reads them, and a vertex
        // stage may not see a writable storage binding.
        let compute_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splat-compute-bgl"),
            entries: &[
                uniform_entry(0, all, false),
                bank(1),
                bank(2),
                storage(3, false, wgpu::ShaderStages::COMPUTE),
                storage(4, false, wgpu::ShaderStages::COMPUTE),
                storage(5, false, wgpu::ShaderStages::COMPUTE),
            ],
        });
        let draw_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splat-draw-bgl"),
            entries: &[
                uniform_entry(0, all, false),
                storage(6, true, wgpu::ShaderStages::VERTEX),
                storage(7, true, wgpu::ShaderStages::VERTEX),
            ],
        });
        let step_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splat-step-bgl"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::COMPUTE, true)],
        });
        let compute_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("splat-compute-bg"),
            layout: &compute_bgl,
            entries: &[
                whole(0, &uniforms),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&attractors.splat_shape_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&attractors.splat_turn_view),
                },
                whole(3, &projected),
                whole(4, &keys),
                whole(5, &order),
            ],
        });
        let draw_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("splat-draw-bg"),
            layout: &draw_bgl,
            entries: &[whole(0, &uniforms), whole(6, &projected), whole(7, &order)],
        });
        let step_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("splat-step-bg"),
            layout: &step_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &steps,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<SortStep>() as u64),
                }),
            }],
        });
        let compute_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("splat-compute-pl"),
            bind_group_layouts: &[Some(particle_bgl), Some(&compute_bgl), Some(&step_bgl)],
            immediate_size: 0,
        });
        let draw_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("splat-draw-pl"),
            bind_group_layouts: &[Some(particle_bgl), Some(&draw_bgl)],
            immediate_size: 0,
        });
        let compute = |label, entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&compute_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let project = compute("splat-project", "cs_splat_project");
        let sort = compute("splat-sort", "cs_splat_sort");
        // Premultiplied "over": each splat covers what is behind it by
        // its own opacity.
        let over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("splat-draw"),
            layout: Some(&draw_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_splat"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_splat"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState { color: over, alpha: over }),
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
            project,
            sort,
            draw,
            uniforms,
            compute_bg,
            draw_bg,
            step_bg,
            steps_for: std::cell::Cell::new(0),
            steps,
        }
    }

    /// Project, sort and draw this frame's splats into `target`, clearing
    /// it to `background` first if `clear`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        particle_bg: &wgpu::BindGroup,
        target: &wgpu::TextureView,
        count: u32,
        clear: bool,
        background: wgpu::Color,
    ) {
        let count = count.min(MAX);
        let padded = count.next_power_of_two().max(2);
        ctx.queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&SplatUniforms { count, padded, _pad: [0; 2] }),
        );
        let steps = sort_steps(padded);
        if self.steps_for.get() != padded {
            let mut bytes = vec![0u8; steps.len() * STEP_STRIDE as usize];
            for (i, s) in steps.iter().enumerate() {
                let at = i * STEP_STRIDE as usize;
                bytes[at..at + std::mem::size_of::<SortStep>()].copy_from_slice(bytemuck::bytes_of(s));
            }
            ctx.queue.write_buffer(&self.steps, 0, &bytes);
            self.steps_for.set(padded);
        }
        if count > 0 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("splat-project-sort"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, particle_bg, &[]);
            pass.set_bind_group(1, &self.compute_bg, &[]);
            pass.set_bind_group(2, &self.step_bg, &[0]);
            let groups = padded.div_ceil(64);
            pass.set_pipeline(&self.project);
            pass.dispatch_workgroups(groups, 1, 1);
            // Each step reads what the last wrote; dispatches in one pass
            // are ordered and their storage writes visible to the next.
            pass.set_pipeline(&self.sort);
            for i in 0..steps.len() {
                pass.set_bind_group(2, &self.step_bg, &[(i as u64 * STEP_STRIDE) as u32]);
                pass.dispatch_workgroups(groups, 1, 1);
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("splat-draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear { wgpu::LoadOp::Clear(background) } else { wgpu::LoadOp::Load },
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if count > 0 {
            pass.set_pipeline(&self.draw);
            pass.set_bind_group(0, particle_bg, &[]);
            pass.set_bind_group(1, &self.draw_bg, &[]);
            pass.draw(0..count * 6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The network is the whole sort: every step, in order, and none
    /// past the length. Checked by running it on the CPU.
    #[test]
    fn the_sort_steps_sort() {
        for n in [2u32, 8, 64, 1024] {
            let mut v: Vec<u32> = (0..n).map(|i| (i * 7919 + 13) % 1009).collect();
            for s in sort_steps(n) {
                for i in 0..n {
                    let l = i ^ s.j;
                    if l > i {
                        let up = i & s.k == 0;
                        let (a, b) = (v[i as usize], v[l as usize]);
                        if (a > b) == up && a != b {
                            v.swap(i as usize, l as usize);
                        }
                    }
                }
            }
            assert!(v.windows(2).all(|w| w[0] <= w[1]), "{n}: {v:?}");
        }
        assert_eq!(sort_steps(MAX).len(), 190);
        assert!(sort_steps(MAX).len() as u64 <= MAX_STEPS);
    }
}
