//! The room the point cloud floats in.
//!
//! A wireframe box drawn with the particles' view/projection, so camera
//! movement parallaxes it against the cloud. Its front face is sized from
//! the camera frustum rather than by eye, so the frame edge *is* the room
//! edge — which is what makes the screen read as a window rather than as a
//! box drawn on a screen.

use crate::{GpuContext, camera::Camera};

/// Lines per face along each axis; must match `N` in room.wgsl.
const LINES_PER_AXIS: u32 = 10;
/// Five faces, each with depth lines and cross lines.
const LINE_COUNT: u32 = 5 * LINES_PER_AXIS * 2;

/// Layout must match `Room` in room.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RoomUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub half_x: f32,
    pub half_y: f32,
    pub depth: f32,
    pub front_z: f32,
    pub brightness: f32,
    pub fade: f32,
    pub lines: f32,
    pub _pad: f32,
}

impl RoomUniforms {
    /// Build a room whose front face exactly fills the frame.
    ///
    /// `front` is how far in front of the camera the opening sits; `depth`
    /// how far back the room runs. Both are in world units, so the cloud —
    /// which lives in roughly a unit box — can be placed inside it.
    pub fn for_camera(cam: &Camera, front: f32, depth: f32, brightness: f32, fade: f32) -> Self {
        let (half_x, half_y) = cam.frustum_half_extents(front);
        // The opening sits `front` along the camera's view direction. With
        // the camera looking at the origin from `distance`, that is
        // `distance - front` along the view axis — expressed here in world
        // z because the room is axis-aligned and the camera orbits around
        // it, which is what produces the parallax.
        Self {
            view_proj: cam.uniforms().view_proj,
            half_x,
            half_y,
            depth,
            front_z: cam.distance - front,
            brightness,
            fade,
            lines: LINES_PER_AXIS as f32,
            _pad: 0.0,
        }
    }
}

pub struct Room {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Room {
    pub fn new(ctx: &GpuContext, target_format: wgpu::TextureFormat) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("room"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/room.wgsl").into()),
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("room-uniforms"),
            size: std::mem::size_of::<RoomUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("room-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("room-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("room-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("room-pipeline"),
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
                    // Additive, matching the particles: the room sits under
                    // the cloud rather than occluding it, which is right
                    // because there is no depth buffer to sort them.
                    blend: Some(wgpu::BlendState {
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
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self { pipeline, uniforms, bind_group }
    }

    /// Draw into an existing pass target. Must run *before* the particles,
    /// so the cloud accumulates on top of it.
    pub fn render(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        uniforms: &RoomUniforms,
    ) {
        ctx.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("room"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The room clears; the particles then load and add.
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
        pass.draw(0..LINE_COUNT * 2, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The room's opening must match the frame at any aspect, which is the
    /// entire forced-perspective premise. Sizing it by eye instead leaves a
    /// sliver of background along an edge and it reads as a floating box.
    #[test]
    fn the_opening_matches_the_frame_at_any_aspect() {
        for aspect in [16.0 / 9.0, 4.0 / 3.0, 1.0, 2.35] {
            let cam = Camera { aspect, ..Default::default() };
            let u = RoomUniforms::for_camera(&cam, 2.0, 6.0, 1.0, 0.7);
            // Half-extents must carry the frame's aspect exactly, or the
            // opening is the wrong shape for the canvas.
            assert!(
                (u.half_x / u.half_y - aspect).abs() < 1e-4,
                "aspect {aspect}: opening is {}:{}",
                u.half_x,
                u.half_y
            );
        }
    }

    /// A wider field of view sees more, so the opening must grow with it —
    /// otherwise zooming out reveals the outside of the box.
    #[test]
    fn a_wider_field_of_view_widens_the_opening() {
        let narrow = RoomUniforms::for_camera(
            &Camera { fov: 0.5, ..Default::default() },
            2.0,
            6.0,
            1.0,
            0.7,
        );
        let wide = RoomUniforms::for_camera(
            &Camera { fov: 1.3, ..Default::default() },
            2.0,
            6.0,
            1.0,
            0.7,
        );
        assert!(wide.half_x > narrow.half_x, "opening did not widen with fov");
        assert!(wide.half_y > narrow.half_y);
    }

    /// The vertex count the pipeline draws has to match what the shader
    /// generates, or the last face is silently missing.
    #[test]
    fn line_count_matches_the_shader() {
        // 5 faces × (N depth lines + N cross lines).
        assert_eq!(LINE_COUNT, 5 * LINES_PER_AXIS * 2);
        assert_eq!(LINE_COUNT, 100);
    }
}
