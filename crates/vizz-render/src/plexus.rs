//! Plexus: lines drawn between particles that come near each other.
//!
//! The look of a network or a constellation — nodes and the links between
//! the close ones — over any cloud, in either draw mode. A few thousand of
//! the particles, spread across the whole field, are the nodes: a compute
//! pass places them with the dots' own shader functions, a second finds
//! each node's nearest few within reach by testing every other node, and a
//! draw lays a thin additive line along each link, fading to nothing at
//! the reach so links come and go softly as the field moves.
//!
//! The form is the "plexus" effect of motion graphics, after the
//! Rowbyte *Plexus* plug-in for After Effects (2010), whose name it has
//! become; the geometry is a k-nearest-neighbour graph cut at a radius.

use crate::GpuContext;
use crate::particles::Uniforms;

/// Most nodes drawn. Every node tests every other, so the cost grows with
/// the square of this: 2048 is four million tests a frame.
pub const NODES: u32 = 2048;
/// Links kept per node. Must match `PLEXUS_LINKS` in plexus.wgsl.
pub const LINKS: u32 = 4;

/// The shader: the particle shader with the plexus passes appended.
pub const SOURCE: &str = concat!(
    include_str!("shaders/particles.wgsl"),
    "\n",
    include_str!("shaders/plexus.wgsl"),
);

/// What `/particles/plexus` and `/particles/plexus_reach` ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plexus {
    /// How bright a link is at no distance; 0 is off.
    pub strength: f32,
    /// How near, in world units, two nodes must be to link.
    pub reach: f32,
}

/// Must match `Plexus` in plexus.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PlexusUniforms {
    pub nodes: u32,
    pub stride: u32,
    pub reach: f32,
    pub strength: f32,
    pub width: f32,
    pub _pad: [f32; 3],
}

impl PlexusUniforms {
    /// Which particles are nodes: up to [`NODES`] of the `particles`
    /// drawn, every `stride`-th, so they cover the field evenly.
    pub fn new(plexus: Plexus, particles: u32, target_h: u32) -> Self {
        let nodes = particles.min(NODES);
        let stride = (particles / nodes.max(1)).max(1);
        Self {
            nodes,
            stride,
            reach: plexus.reach.max(1e-4),
            strength: plexus.strength,
            // A pixel and a half on a 1200-line target, the 2× of a
            // 600-line output, and the same share of any other.
            width: (1.5 * target_h as f32 / 1200.0).max(1.0),
            _pad: [0.0; 3],
        }
    }
}

pub(crate) struct PlexusPass {
    nodes_pipe: wgpu::ComputePipeline,
    links_pipe: wgpu::ComputePipeline,
    draw: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    compute_bg: wgpu::BindGroup,
    draw_bg: wgpu::BindGroup,
}

/// Bytes per node. Must match `Node` in plexus.wgsl.
const NODE_BYTES: u64 = 32;

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
}

impl PlexusPass {
    pub fn new(
        ctx: &GpuContext,
        particle_bgl: &wgpu::BindGroupLayout,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("plexus"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plexus-uniforms"),
            size: std::mem::size_of::<PlexusUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let nodes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plexus-nodes"),
            size: NODES as u64 * NODE_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let links = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plexus-links"),
            size: (NODES * LINKS) as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
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
        // Two layouts over the same buffers, as in the surface mode: the
        // compute passes write them and the draw reads them, and a vertex
        // stage may not see a writable storage binding.
        let compute_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("plexus-compute-bgl"),
            entries: &[
                uniform_entry,
                storage(3, false, wgpu::ShaderStages::COMPUTE),
                storage(4, false, wgpu::ShaderStages::COMPUTE),
            ],
        });
        let draw_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("plexus-draw-bgl"),
            entries: &[
                uniform_entry,
                storage(1, true, wgpu::ShaderStages::VERTEX),
                storage(2, true, wgpu::ShaderStages::VERTEX),
            ],
        });
        let compute_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plexus-compute-bg"),
            layout: &compute_bgl,
            entries: &[entry(0, &uniforms), entry(3, &nodes), entry(4, &links)],
        });
        let draw_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plexus-draw-bg"),
            layout: &draw_bgl,
            entries: &[entry(0, &uniforms), entry(1, &nodes), entry(2, &links)],
        });
        let compute_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("plexus-compute-pl"),
            bind_group_layouts: &[Some(particle_bgl), Some(&compute_bgl)],
            immediate_size: 0,
        });
        let draw_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("plexus-draw-pl"),
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
        let nodes_pipe = compute("plexus-nodes", "cs_plexus_nodes");
        let links_pipe = compute("plexus-links", "cs_plexus_links");
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("plexus-draw"),
            layout: Some(&draw_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_plexus"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_plexus"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState { color: additive, alpha: additive }),
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
        Self { nodes_pipe, links_pipe, draw, uniforms, compute_bg, draw_bg }
    }

    /// Find this frame's links and draw them over `target`, on top of
    /// whatever is there.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        particle_bg: &wgpu::BindGroup,
        target: &wgpu::TextureView,
        uniforms: &Uniforms,
        count: u32,
        plexus: Plexus,
    ) {
        let segs = (uniforms.stroke[2] + 0.5).max(1.0) as u32;
        let pu = PlexusUniforms::new(plexus, count / segs, target.texture().height());
        if pu.nodes < 2 {
            return;
        }
        ctx.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&pu));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("plexus-find"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, particle_bg, &[]);
            pass.set_bind_group(1, &self.compute_bg, &[]);
            pass.set_pipeline(&self.nodes_pipe);
            pass.dispatch_workgroups(pu.nodes.div_ceil(64), 1, 1);
            pass.set_pipeline(&self.links_pipe);
            pass.dispatch_workgroups(pu.nodes.div_ceil(64), 1, 1);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("plexus-draw"),
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
        pass.set_pipeline(&self.draw);
        pass.set_bind_group(0, particle_bg, &[]);
        pass.set_bind_group(1, &self.draw_bg, &[]);
        pass.draw(0..pu.nodes * LINKS * 6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_spread_over_the_field_and_never_exceed_the_cap() {
        let p = Plexus { strength: 1.0, reach: 0.1 };
        let small = PlexusUniforms::new(p, 500, 1200);
        assert_eq!((small.nodes, small.stride), (500, 1));
        let big = PlexusUniforms::new(p, 60_000, 1200);
        assert_eq!(big.nodes, NODES);
        assert!(big.stride * big.nodes <= 60_000 && (big.stride + 1) * big.nodes > 60_000);
        assert_eq!(PlexusUniforms::new(p, 0, 1200).nodes, 0);
        assert_eq!(std::mem::size_of::<PlexusUniforms>(), 32);
    }
}
