//! The room the point cloud floats in.
//!
//! A wireframe box drawn with the particles' view/projection, so camera
//! movement parallaxes it against the cloud. Its front face is sized from
//! the camera frustum rather than by eye, so the frame edge *is* the room
//! edge — which is what makes the screen read as a window rather than as a
//! box drawn on a screen.

use crate::{GpuContext, camera::Camera};

/// How far the opening reaches past the frame edge.
///
/// Sizing it to land *exactly* on the edge puts the opening's outline at
/// clip x = ±1, where whether a given pixel rasterizes is a coin flip —
/// which shows up as a ragged sliver of line down one border, blurred into
/// a soft bar by the glow pass. Pushing it a hair outside removes the
/// question: the walls run off the edge of the frame, which is what the
/// illusion actually wants. Nobody needs to *see* the opening's outline.
const OVERSCAN: f32 = 1.01;

/// Lines per face along each axis; must match `N` in room.wgsl.
const LINES_PER_AXIS: u32 = 10;
/// Five faces, each with depth lines and cross lines.
const LINE_COUNT: u32 = 5 * LINES_PER_AXIS * 2;
/// Two triangles per line; see `vs_main` in room.wgsl.
const VERTS_PER_LINE: u32 = 6;

/// How wide a room line is, in render pixels, when the scene is drawn
/// `render_h` pixels tall for an output `output_h` tall.
///
/// One output pixel, whatever the render scale. The old hardware lines
/// were one *render* pixel, so drawing at 2× and downscaling halved the
/// room's light — which is what kept 2× from being the default.
pub fn line_px(render_h: u32, output_h: u32) -> f32 {
    (render_h.max(1) as f32 / output_h.max(1) as f32).max(1e-3)
}

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
    /// Back rect size relative to the opening. 1.0 gives parallel walls and
    /// no convergence at all; smaller values pull the far end in, which is
    /// the forced-perspective exaggeration a stage set does with physical
    /// scenery. 0 collapses the back wall to the vanishing point.
    pub converge: f32,
    /// Where the vanishing point sits, in units of the opening's half-size.
    /// 0,0 is frame centre; ±1 puts it on the frame edge.
    pub vanish_x: f32,
    pub vanish_y: f32,
    /// Target size in pixels and line width in those pixels. Set by
    /// [`Room::render`] from the target it draws into; whatever the caller
    /// leaves here is overwritten.
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub line_px: f32,
}

impl RoomUniforms {
    /// Build a room whose front face exactly fills the frame.
    ///
    /// `front` is how far in front of the camera the opening sits; `depth`
    /// how far back the room runs. Both are in world units, so the cloud —
    /// which lives in roughly a unit box — can be placed inside it.
    #[allow(clippy::too_many_arguments)]
    pub fn for_camera(
        cam: &Camera,
        front: f32,
        depth: f32,
        brightness: f32,
        fade: f32,
        converge: f32,
        vanish_x: f32,
        vanish_y: f32,
    ) -> Self {
        let (half_x, half_y) = cam.frustum_half_extents(front);
        let (half_x, half_y) = (half_x * OVERSCAN, half_y * OVERSCAN);
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
            // Clamped: a back rect larger than the opening turns the room
            // inside out, and the walls cross over each other.
            converge: converge.clamp(0.0, 1.0),
            vanish_x,
            vanish_y,
            viewport_w: 0.0,
            viewport_h: 0.0,
            line_px: 1.0,
        }
    }

    /// The same volume, in the form the particle shader needs to put an
    /// object inside it. See [`Placement`].
    pub fn placement(&self, anchor: f32, embed: f32) -> Placement {
        Placement {
            front_z: self.front_z,
            depth: self.depth,
            half_x: self.half_x,
            half_y: self.half_y,
            converge: self.converge,
            vanish_x: self.vanish_x,
            vanish_y: self.vanish_y,
            anchor: anchor.clamp(0.0, 1.0),
            embed: embed.clamp(0.0, 1.0),
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        }
    }
}

/// How a foreground object sits inside the room.
///
/// Drawing the room and drawing the cloud with an ordinary camera gives two
/// objects in the same frame, not an object *in* a space: the cloud keeps
/// its own proportions while the set around it is compressed, and the eye
/// reads it as a sprite pasted over a backdrop. Passing the room's volume
/// to the particle shader lets the cloud take the same forced perspective
/// as the walls, so it belongs to the set.
///
/// Layout must match `RoomPlace` in particles.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Placement {
    pub front_z: f32,
    pub depth: f32,
    pub half_x: f32,
    pub half_y: f32,
    pub converge: f32,
    pub vanish_x: f32,
    pub vanish_y: f32,
    /// Where along the room's depth the object's centre sits. 0 is the
    /// opening, 1 the back wall.
    pub anchor: f32,
    /// How much the object belongs to the room. 0 leaves it exactly where
    /// it would be with no room at all, which is why it is the default:
    /// turning the room on must not move the cloud.
    pub embed: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

impl Placement {
    /// Where a point ends up, and the scale it picked up on the way.
    ///
    /// Mirrors `room_place` in particles.wgsl. Duplicated rather than
    /// shared because one side is WGSL — so the tests below exist to keep
    /// the two honest about the properties that matter.
    pub fn place(&self, p: [f32; 3]) -> ([f32; 3], f32) {
        if self.embed <= 0.001 {
            return (p, 1.0);
        }
        let depth = self.depth.max(1e-4);
        // Slide the object to its anchor depth first, then read the
        // cross-section at each point's *own* depth: that is what makes the
        // near side of the object larger than its far side, the same way
        // the room's walls converge.
        let centre_z = self.front_z - self.anchor * depth;
        let z = centre_z + p[2];
        let t = ((self.front_z - z) / depth).clamp(0.0, 1.0);
        let s = 1.0 + (self.converge - 1.0) * t;
        let cx = self.vanish_x * self.half_x * t;
        let cy = self.vanish_y * self.half_y * t;
        let placed = [cx + p[0] * s, cy + p[1] * s, z];
        let mix = |a: f32, b: f32| a + (b - a) * self.embed;
        (
            [mix(p[0], placed[0]), mix(p[1], placed[1]), mix(p[2], placed[2])],
            mix(1.0, s),
        )
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
                topology: wgpu::PrimitiveTopology::TriangleList,
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
    ///
    /// `output_h` is the height of the master the target is eventually
    /// downscaled into; lines are one pixel of *that* wide, so the room's
    /// brightness does not change with the render scale.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        uniforms: &RoomUniforms,
        output_h: u32,
        background: wgpu::Color,
        clear: bool,
    ) {
        let size = target.texture().size();
        let uniforms = RoomUniforms {
            viewport_w: size.width as f32,
            viewport_h: size.height as f32,
            line_px: line_px(size.height, output_h),
            ..*uniforms
        };
        ctx.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("room"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The room clears; the particles then load and add.
                    // Takes the same background as the particle pass, so
                    // turning the room on does not change what the
                    // background is — the one thing that would make the
                    // setting look broken.
                    // First pass of the frame clears; when the vector
                    // stack has already painted the page, load it.
                    load: if clear {
                        wgpu::LoadOp::Clear(background)
                    } else {
                        wgpu::LoadOp::Load
                    },
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
        pass.draw(0..LINE_COUNT * VERTS_PER_LINE, 0..1);
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
            let u = RoomUniforms::for_camera(&cam, 2.0, 6.0, 1.0, 0.7, 0.35, 0.0, 0.0);
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

    /// The opening must cover the frame and then some. Landing it exactly
    /// on the edge leaves its outline at clip ±1, which rasterizes as a
    /// ragged half-drawn line down the border; too much overscan and the
    /// walls visibly start outside the frame, which loses the illusion.
    #[test]
    fn the_opening_clears_the_frame_without_overshooting_it() {
        let cam = Camera::default();
        let front = 2.0;
        let (fx, fy) = cam.frustum_half_extents(front);
        let u = RoomUniforms::for_camera(&cam, front, 6.0, 1.0, 0.7, 0.35, 0.0, 0.0);
        for (room, frame, axis) in [(u.half_x, fx, "x"), (u.half_y, fy, "y")] {
            let ratio = room / frame;
            assert!(ratio > 1.0, "{axis}: opening sits inside the frame at {ratio}");
            assert!(ratio < 1.03, "{axis}: overscan {ratio} is visible as a gap");
        }
    }

    /// A wider field of view sees more, so the opening must grow with it —
    /// otherwise zooming out reveals the outside of the box.
    #[test]
    fn a_wider_field_of_view_widens_the_opening() {
        let cfg = |fov| {
            RoomUniforms::for_camera(&Camera { fov, ..Default::default() }, 2.0, 6.0, 1.0, 0.7, 0.35, 0.0, 0.0)
        };
        let narrow = cfg(0.5);
        let wide = cfg(1.3);
        assert!(wide.half_x > narrow.half_x, "opening did not widen with fov");
        assert!(wide.half_y > narrow.half_y);
    }

    /// The opening is pinned to the frame whatever the perspective controls
    /// do — that is the "side edges stay bound" property. Only the far end
    /// may move, or the illusion breaks at the frame border.
    #[test]
    fn perspective_controls_never_move_the_opening() {
        let cam = Camera::default();
        let base = RoomUniforms::for_camera(&cam, 2.0, 6.0, 1.0, 0.7, 1.0, 0.0, 0.0);
        for (converge, vx, vy) in [
            (0.0, 0.0, 0.0),
            (0.2, 0.9, -0.6),
            (1.0, -1.0, 1.0),
            // Out of range on purpose: it must clamp, not invert the room.
            (5.0, 0.0, 0.0),
            (-3.0, 0.0, 0.0),
        ] {
            let u = RoomUniforms::for_camera(&cam, 2.0, 6.0, 1.0, 0.7, converge, vx, vy);
            assert_eq!(u.half_x, base.half_x, "converge {converge} moved the opening");
            assert_eq!(u.half_y, base.half_y);
            assert!(
                (0.0..=1.0).contains(&u.converge),
                "converge {converge} escaped its range as {}",
                u.converge
            );
        }
    }

    fn room() -> RoomUniforms {
        RoomUniforms::for_camera(&Camera::default(), 2.0, 6.0, 1.0, 0.7, 0.4, 0.0, 0.0)
    }

    /// The room is off by default and must stay a pure addition: switching
    /// it on cannot teleport the cloud. Anything else means a control that
    /// is unusable live, because reaching for the room mid-set would move
    /// the thing the audience is looking at.
    #[test]
    fn nothing_moves_until_you_ask_it_to() {
        let p = room().placement(0.35, 0.0);
        for point in [[0.0, 0.0, 0.0], [0.7, -0.3, 0.5], [-1.0, 1.0, -0.9]] {
            let (out, scale) = p.place(point);
            assert_eq!(out, point, "embed 0 moved {point:?}");
            assert_eq!(scale, 1.0);
        }
    }

    /// Embedded, the object has to pick up the room's compression — and
    /// its near side must stay bigger than its far side. A single uniform
    /// scale would shrink the object without making it *belong* to the
    /// space, which is the whole point of associating it.
    #[test]
    fn an_embedded_object_takes_the_rooms_perspective() {
        let p = room().placement(0.5, 1.0);
        let (_, near) = p.place([0.0, 0.0, 0.5]);
        let (_, far) = p.place([0.0, 0.0, -0.5]);
        assert!(near > far, "object did not compress with depth: {near} vs {far}");
        // And both are smaller than life size, since the anchor is halfway
        // into a room that converges.
        assert!(far < 1.0, "far side was not compressed at all: {far}");

        // Wider point, further from the axis, must move inward more.
        let (edge, _) = p.place([1.0, 0.0, -0.5]);
        assert!(edge[0] < 1.0, "edge did not pull in: {}", edge[0]);
    }

    /// Anchor is a depth control: pushing it back must put the object
    /// further from the camera and make it smaller.
    #[test]
    fn anchoring_deeper_pushes_the_object_away() {
        let r = room();
        let front = r.placement(0.0, 1.0).place([0.0, 0.0, 0.0]);
        let back = r.placement(1.0, 1.0).place([0.0, 0.0, 0.0]);
        // The camera looks down -z, so deeper is a smaller world z.
        assert!(back.0[2] < front.0[2], "anchor did not move the object back");
        assert!(back.1 < front.1, "the deeper object was not smaller");
        // At the opening the object is exactly life size — the room has
        // not compressed it yet.
        assert!((front.1 - 1.0).abs() < 1e-5, "front anchor scaled: {}", front.1);
    }

    /// The vanishing point drags the object with it. If the room's far end
    /// slides one way and the object stays put, the two stop reading as
    /// parts of the same space.
    #[test]
    fn the_object_follows_the_vanishing_point() {
        let cam = Camera::default();
        let with = |vx| {
            RoomUniforms::for_camera(&cam, 2.0, 6.0, 1.0, 0.7, 0.4, vx, 0.0)
                .placement(1.0, 1.0)
                .place([0.0, 0.0, 0.0])
                .0[0]
        };
        assert!(with(0.8) > with(0.0), "object ignored a rightward vanishing point");
        assert!(with(-0.8) < with(0.0), "object ignored a leftward vanishing point");
    }

    /// Out-of-range anchor and embed must clamp. Both are modulatable, and
    /// an LFO swinging past the ends should park at them rather than turn
    /// the object inside out.
    #[test]
    fn placement_inputs_clamp() {
        let p = room().placement(9.0, -4.0);
        assert_eq!(p.anchor, 1.0);
        assert_eq!(p.embed, 0.0);
    }

    /// The vertex count the pipeline draws has to match what the shader
    /// generates, or the last face is silently missing.
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

    fn f16_to_f32(h: u16) -> f32 {
        let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = ((h >> 10) & 0x1f) as i32;
        let frac = (h & 0x3ff) as f32;
        match exp {
            0 => sign * frac * 2f32.powi(-24),
            31 => f32::NAN,
            _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
        }
    }

    /// Draw the room `scale` times larger than a `size`-pixel output, box
    /// it back down to the output, and return the output's pixels (the
    /// green channel, linear).
    fn render_room(ctx: &GpuContext, room: &Room, size: [u32; 2], scale: u32) -> Vec<f32> {
        let [w, h] = [size[0] * scale, size[1] * scale];
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("room-test-target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::post::SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let row = (w * 8).div_ceil(256) * 256;
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("room-test-readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let cam = Camera { aspect: w as f32 / h as f32, ..Default::default() };
        let u = RoomUniforms::for_camera(&cam, cam.distance - 1.6, 6.0, 1.0, 0.7, 0.6, 0.1, -0.05);
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        room.render(ctx, &mut enc, &view, &u, size[1], wgpu::Color::BLACK, true);
        enc.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            texture.size(),
        );
        ctx.queue.submit([enc.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = slice.get_mapped_range().unwrap().to_vec();
        let texel = |x: u32, y: u32| {
            let o = (y * row + x * 8 + 2) as usize;
            f16_to_f32(u16::from_le_bytes([bytes[o], bytes[o + 1]]))
        };
        let mut out = vec![0.0; (size[0] * size[1]) as usize];
        for y in 0..size[1] {
            for x in 0..size[0] {
                let mut sum = 0.0;
                for dy in 0..scale {
                    for dx in 0..scale {
                        sum += texel(x * scale + dx, y * scale + dy);
                    }
                }
                out[(y * size[0] + x) as usize] = sum / (scale * scale) as f32;
            }
        }
        out
    }

    /// The room is as bright at every render scale.
    ///
    /// The old hardware lines were one render pixel wide, so 2× kept half
    /// the room's light and 4× a quarter — measured at 9.7, 4.9 and 2.3
    /// (×1e-3 mean linear) in the 2026-09-23 previz review, which is what
    /// kept 2× from being the default. Lines one *output* pixel wide with
    /// exact coverage put the same light into the frame however finely
    /// they are drawn.
    #[test]
    fn the_room_is_as_bright_at_every_render_scale() {
        let Some(ctx) = gpu() else { return };
        let room = Room::new(&ctx, crate::post::SCENE_FORMAT);
        let size = [160, 90];
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let one = render_room(&ctx, &room, size, 1);
        let two = render_room(&ctx, &room, size, 2);
        let four = render_room(&ctx, &room, size, 4);
        let (m1, m2, m4) = (mean(&one), mean(&two), mean(&four));
        assert!(m1 > 1e-3, "the room drew nothing: {m1}");
        for (label, m) in [("2x", m2), ("4x", m4)] {
            let ratio = m / m1;
            assert!(
                (0.93..1.07).contains(&ratio),
                "{label} keeps {ratio:.2} of the 1x room's light ({m} vs {m1})"
            );
        }
        // And the finer renders converge on one picture rather than just
        // matching in total: 2x sits closer to 4x than 1x does.
        let err = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>();
        assert!(err(&two, &four) < err(&one, &four));
    }

    #[test]
    fn line_width_is_one_output_pixel() {
        assert_eq!(line_px(1080, 1080), 1.0);
        assert_eq!(line_px(2160, 1080), 2.0);
        assert_eq!(line_px(540, 1080), 0.5);
        // Never zero, even for a nonsense size.
        assert!(line_px(0, 0) > 0.0);
    }

    #[test]
    fn line_count_matches_the_shader() {
        // 5 faces × (N depth lines + N cross lines).
        assert_eq!(LINE_COUNT, 5 * LINES_PER_AXIS * 2);
        assert_eq!(LINE_COUNT, 100);
    }
}
