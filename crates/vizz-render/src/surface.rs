//! The surface draw mode: the cloud as opaque, lit, shadowed surfels.
//!
//! The additive pass treats every particle as light, which is the right
//! model for a haze and the wrong one for an object: nothing occludes
//! anything, a lamp cannot light one side of a form, and nothing casts a
//! shadow. This mode draws the same particles — placed, coloured and
//! oriented by the same shader functions — as depth-tested discs lit per
//! pixel by the lamps, the sun and a sky, with the sun's shadow from a
//! depth map. When the room is up, its walls, floor and ceiling are drawn
//! as surfaces too, so the lamps and the cloud's shadow land on them.
//!
//! It is opt-in with `/particles/surface`. The additive pass is untouched:
//! its shader shares the per-particle functions with this one, and a frame
//! drawn in the additive mode is byte-for-byte what it was before this mode
//! existed.
//!
//! Sources: surfels are Pfister, Zwicker, van Baar & Gross, "Surfels:
//! Surface Elements as Rendering Primitives", SIGGRAPH 2000; shadow maps
//! are Williams, "Casting curved shadows on curved surfaces", SIGGRAPH
//! 1978; the filtered lookup is Reeves, Salesin & Cook, "Rendering
//! antialiased shadows with depth maps", SIGGRAPH 1987.

use crate::GpuContext;
use crate::particles::Uniforms;
use glam::{Mat4, Vec3};

/// The shader: the particle shader with the surface passes appended, so
/// both modes place a particle with literally the same code.
pub const SOURCE: &str = concat!(
    include_str!("shaders/particles.wgsl"),
    "\n",
    include_str!("shaders/surface.wgsl"),
);

/// Bytes per evaluated particle. Must match `Splat` in surface.wgsl.
pub const SPLAT_BYTES: u64 = 64;

/// Vertices per particle when the surface mode draws glyphs. Must match
/// `GLYPH_VERTS_MAX` in surface.wgsl.
pub const GLYPH_VERTS: u32 = 36;

/// Shadow map edge, in texels. 2048 over a cloud a few units across is a
/// texel of a few thousandths of a unit, finer than the default sprite.
pub const SHADOW_SIZE: u32 = 2048;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Albedo can exceed 1 — brightness runs to 2 — so it is kept in half
/// floats. The normal is a direction and a trust, so bytes are plenty.
const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Per-frame inputs for the surface passes. Layout must match `Surface`
/// in surface.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SurfaceUniforms {
    pub sun_view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub sun_right: [f32; 3],
    pub shadow_on: f32,
    pub sun_up: [f32; 3],
    pub shadow_texel: f32,
    pub room_brightness: f32,
    pub room_fade: f32,
    pub shadow_depth: f32,
    pub solid_param: f32,
    pub count: u32,
    pub solid_kind: u32,
    /// 0 off, 1 outlines, 2 hatching, 3 stipple — see [`InkKind`].
    pub ink: u32,
    pub ink_weight: f32,
}

/// The room, when it is drawn as surfaces: how bright its plaster is and
/// how much it fades towards the back wall, as `/room/brightness` and
/// `/room/fade` already say for the wireframe. Its shape comes from the
/// placement the particle uniforms carry.
#[derive(Debug, Clone, Copy)]
pub struct Walls {
    pub brightness: f32,
    pub fade: f32,
}

/// A fractal drawn as a solid by sphere tracing, into the same G-buffer
/// as the cloud — see `fs_solid` in surface.wgsl.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Solid {
    pub kind: SolidKind,
    /// Power for the Mandelbulb, scale for the Mandelbox, the turn of
    /// the constant for the Julia set, levels for the sponge.
    pub param: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidKind {
    Mandelbulb = 1,
    Mandelbox = 2,
    Julia = 3,
    Menger = 4,
}

impl SolidKind {
    /// From `/particles/solid`: 0 is none.
    pub fn from_index(v: f32) -> Option<Self> {
        match v.round() as i32 {
            1 => Some(Self::Mandelbulb),
            2 => Some(Self::Mandelbox),
            3 => Some(Self::Julia),
            4 => Some(Self::Menger),
            _ => None,
        }
    }

    /// What `/particles/solid_detail`'s 0..1 means for this kind: the
    /// bulb's power from 2 to 12, the box's scale from −2.6 to −1.5 and
    /// then from 2 to 3, the Julia constant's turn through a full circle,
    /// the sponge's levels from 1 to 6.
    pub fn param(self, detail: f32) -> f32 {
        let d = detail.clamp(0.0, 1.0);
        match self {
            Self::Mandelbulb => 2.0 + 10.0 * d,
            // Skipping −1.5..2, where the box is a shapeless lump.
            Self::Mandelbox => {
                if d < 0.5 { -2.6 + 1.1 * (d / 0.5) } else { 2.0 + 1.0 * ((d - 0.5) / 0.5) }
            }
            Self::Julia => d * std::f32::consts::TAU,
            Self::Menger => 1.0 + (5.0 * d).round(),
        }
    }
}

/// The surface drawn as a pen drawing: outlines where the depth breaks,
/// and in the two drawn kinds the tone laid down in ink on paper — see
/// `ink` in surface.wgsl.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ink {
    pub kind: InkKind,
    /// 0..1, from `/particles/ink_weight`: how heavy the pen is. Scales
    /// the line width, the hatching's spacing and the stipple's dots.
    pub weight: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InkKind {
    /// The lit surface with its edges inked.
    Outline = 1,
    /// Paper, edges, and tone as up to three layers of crossed lines.
    Hatch = 2,
    /// Paper, edges, and tone as dots, more and larger where it is dark.
    Stipple = 3,
}

impl InkKind {
    /// From `/particles/ink`: 0 is off.
    pub fn from_index(v: f32) -> Option<Self> {
        match v.round() as i32 {
            1 => Some(Self::Outline),
            2 => Some(Self::Hatch),
            3 => Some(Self::Stipple),
            _ => None,
        }
    }
}

/// Where the sun looks from, for its shadow map.
#[derive(Debug, Clone, Copy)]
pub struct SunFrame {
    pub view_proj: Mat4,
    pub right: Vec3,
    pub up: Vec3,
    /// World units per shadow-map texel.
    pub texel: f32,
    /// World units the depth range spans.
    pub depth: f32,
}

/// Fit the sun's orthographic view to the cloud.
///
/// Centred where the room puts the cloud, and wide enough for the largest
/// shape at this spread — 2.2× covers a unit-box attractor's corners
/// (√3) with the breathing and some wind on top. Only the cloud casts, so
/// only it needs to be in the map; a receiver outside the map is simply
/// lit. The depth range runs well past the cloud so a floor or a back wall
/// far behind it still finds the cloud in front of it.
pub fn sun_frame(u: &Uniforms) -> SunFrame {
    let (centre, scale) = u.room.place([0.0, 0.0, 0.0]);
    let centre = Vec3::from(centre);
    let radius = (u.spread * 2.2 * scale.max(0.05) + u.size * 2.0).max(0.05);
    let toward = Vec3::new(u.sun_dir[0], u.sun_dir[1], u.sun_dir[2]).normalize_or(Vec3::Y);
    let back_off = radius + 1.0;
    let eye = centre + toward * back_off;
    let hint = if toward.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_at_rh(eye, centre, hint);
    let depth = back_off + radius + 60.0;
    let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, depth);
    let forward = -toward;
    let right = forward.cross(hint).normalize();
    let up = right.cross(forward);
    SunFrame {
        view_proj: proj * view,
        right,
        up,
        texel: 2.0 * radius / SHADOW_SIZE as f32,
        depth,
    }
}

struct Splats {
    capacity: u64,
    // Held so the bind groups below keep a live buffer to point at.
    _buffer: wgpu::Buffer,
    draw_bg: wgpu::BindGroup,
    eval_bg: wgpu::BindGroup,
}

/// What the surfel and wall passes leave for the lighting pass, at the
/// target's size.
struct GBuffer {
    size: (u32, u32),
    depth: wgpu::TextureView,
    albedo: wgpu::TextureView,
    normal: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

pub struct Surface {
    eval: wgpu::ComputePipeline,
    shadow: wgpu::RenderPipeline,
    draw: wgpu::RenderPipeline,
    walls: wgpu::RenderPipeline,
    solid: wgpu::RenderPipeline,
    shade: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    draw_bgl: wgpu::BindGroupLayout,
    eval_bgl: wgpu::BindGroupLayout,
    gbuffer_bgl: wgpu::BindGroupLayout,
    shadow_view: wgpu::TextureView,
    shadow_bg: wgpu::BindGroup,
    splats: Option<Splats>,
    gbuffer: Option<GBuffer>,
}

impl Surface {
    /// Built on the first frame drawn in this mode, so a show that never
    /// uses it never pays for the shadow map.
    pub fn new(
        ctx: &GpuContext,
        particle_bgl: &wgpu::BindGroupLayout,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let device = &ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("surface"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("surface-uniforms"),
            size: std::mem::size_of::<SurfaceUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
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
        // Two layouts over the same buffer: the compute pass writes it and
        // the draws read it, and a vertex stage may not see a writable
        // storage binding.
        let draw_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface-draw-bgl"),
            entries: &[uniform_entry, storage(1, true, wgpu::ShaderStages::VERTEX)],
        });
        let eval_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface-eval-bgl"),
            entries: &[uniform_entry, storage(2, false, wgpu::ShaderStages::COMPUTE)],
        });
        let shadow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface-shadow-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });

        let unfiltered = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let gbuffer_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface-gbuffer-bgl"),
            entries: &[
                unfiltered(0),
                unfiltered(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun-shadow-map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_tex.create_view(&Default::default());
        let cmp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sun-shadow-cmp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface-shadow-bg"),
            layout: &shadow_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&cmp),
                },
            ],
        });

        let eval_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface-eval-pl"),
            bind_group_layouts: &[Some(particle_bgl), Some(&eval_bgl)],
            immediate_size: 0,
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface-shadow-pl"),
            bind_group_layouts: &[Some(particle_bgl), Some(&draw_bgl)],
            immediate_size: 0,
        });
        let draw_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface-draw-pl"),
            bind_group_layouts: &[Some(particle_bgl), Some(&draw_bgl)],
            immediate_size: 0,
        });
        let shade_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface-shade-pl"),
            bind_group_layouts: &[
                Some(particle_bgl),
                Some(&draw_bgl),
                Some(&shadow_bgl),
                Some(&gbuffer_bgl),
            ],
            immediate_size: 0,
        });

        let eval = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("surface-eval"),
            layout: Some(&eval_layout),
            module: &shader,
            entry_point: Some("cs_eval"),
            compilation_options: Default::default(),
            cache: None,
        });
        let depth_state = wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        };
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("surface-shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shadow"),
                targets: &[],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth_state.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let opaque = |label, vs, fs| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    // Opaque: a surfel replaces what is behind it.
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format: ALBEDO_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                        Some(wgpu::ColorTargetState {
                            format: NORMAL_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                    ],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(depth_state.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let draw = opaque("surface-draw", "vs_surface", "fs_surface");
        let walls = opaque("surface-walls", "vs_walls", "fs_walls");
        let solid = opaque("surface-solid", "vs_solid", "fs_solid");
        let shade = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("surface-shade"),
            layout: Some(&shade_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shade"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shade"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
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
            eval,
            shadow,
            draw,
            walls,
            solid,
            shade,
            uniforms,
            draw_bgl,
            eval_bgl,
            gbuffer_bgl,
            shadow_view,
            shadow_bg,
            splats: None,
            gbuffer: None,
        }
    }

    /// Make sure the particle buffer holds `count`. Grown in powers of two
    /// so a swept count does not reallocate every frame.
    fn ensure_splats(&mut self, device: &wgpu::Device, count: u32) {
        let need = u64::from(count.max(1));
        if self.splats.as_ref().is_some_and(|s| s.capacity >= need) {
            return;
        }
        let capacity = need.next_power_of_two().max(1024);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("surface-splats"),
            size: capacity * SPLAT_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let group = |layout, binding| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("surface-splats-bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            })
        };
        let draw_bg = group(&self.draw_bgl, 1);
        let eval_bg = group(&self.eval_bgl, 2);
        self.splats = Some(Splats { capacity, _buffer: buffer, draw_bg, eval_bg });
    }

    fn ensure_gbuffer(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.gbuffer.as_ref().is_some_and(|g| g.size == size) {
            return;
        }
        let target = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let depth = target("surface-depth", DEPTH_FORMAT);
        let albedo = target("surface-albedo", ALBEDO_FORMAT);
        let normal = target("surface-normal", NORMAL_FORMAT);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface-gbuffer-bg"),
            layout: &self.gbuffer_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&albedo),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&normal),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
            ],
        });
        self.gbuffer = Some(GBuffer { size, depth, albedo, normal, bind_group });
    }

    /// Encode the surface passes. `uniforms` must already be the ones
    /// written to the particle buffer bound in `particle_bg`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        particle_bg: &wgpu::BindGroup,
        target: &wgpu::TextureView,
        uniforms: &Uniforms,
        count: u32,
        clear: bool,
        background: wgpu::Color,
        walls: Option<Walls>,
        solid: Option<Solid>,
        ink: Option<Ink>,
    ) {
        let device = &ctx.device;
        let size = target.texture().size();
        self.ensure_splats(device, count);
        self.ensure_gbuffer(device, (size.width, size.height));
        let sun = sun_frame(uniforms);
        let shadow_on = uniforms.sun_dir[3] > 0.001 && count > 0;
        let walls_on = walls.is_some_and(|w| w.brightness > 0.002);
        // A disc is a quad; a glyph is a mesh, drawn with the largest
        // mesh's vertex count so every kind fits. See `glyph_vertex`.
        let verts = if uniforms.stroke[3] > 0.5 { GLYPH_VERTS } else { 6 };
        let su = SurfaceUniforms {
            sun_view_proj: sun.view_proj.to_cols_array_2d(),
            inv_view_proj: Mat4::from_cols_array_2d(&uniforms.view_proj)
                .inverse()
                .to_cols_array_2d(),
            sun_right: sun.right.into(),
            shadow_on: if shadow_on { 1.0 } else { 0.0 },
            sun_up: sun.up.into(),
            shadow_texel: sun.texel,
            room_brightness: walls.map_or(0.0, |w| w.brightness),
            room_fade: walls.map_or(0.0, |w| w.fade),
            shadow_depth: sun.depth,
            solid_param: solid.map_or(0.0, |s| s.param),
            count,
            solid_kind: solid.map_or(0, |s| s.kind as u32),
            ink: ink.map_or(0, |i| i.kind as u32),
            ink_weight: ink.map_or(0.5, |i| i.weight),
        };
        ctx.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&su));
        let splats = self.splats.as_ref().expect("allocated above");
        let gbuffer = self.gbuffer.as_ref().expect("allocated above");

        if count > 0 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("surface-eval"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.eval);
            pass.set_bind_group(0, particle_bg, &[]);
            pass.set_bind_group(1, &splats.eval_bg, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }

        if shadow_on {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("surface-shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow);
            pass.set_bind_group(0, particle_bg, &[]);
            pass.set_bind_group(1, &splats.draw_bg, &[]);
            pass.draw(0..count * verts, 0..1);
        }

        // The surfaces: albedo, known normals and depth.
        {
            let clear_to = |view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("surface-gbuffer"),
                color_attachments: &[clear_to(&gbuffer.albedo), clear_to(&gbuffer.normal)],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gbuffer.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, particle_bg, &[]);
            pass.set_bind_group(1, &splats.draw_bg, &[]);
            if count > 0 {
                pass.set_pipeline(&self.draw);
                pass.draw(0..count * verts, 0..1);
            }
            if solid.is_some() {
                pass.set_pipeline(&self.solid);
                pass.draw(0..3, 0..1);
            }
            // After the cloud, so the depth test skips the plaster it hides.
            if walls_on {
                pass.set_pipeline(&self.walls);
                pass.draw(0..5 * 6, 0..1);
            }
        }

        // Light every covered pixel once. Uncovered pixels are left alone,
        // so whatever was drawn before — or the clear — shows through.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("surface-shade"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
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
        pass.set_pipeline(&self.shade);
        pass.set_bind_group(0, particle_bg, &[]);
        pass.set_bind_group(1, &splats.draw_bg, &[]);
        pass.set_bind_group(2, &self.shadow_bg, &[]);
        pass.set_bind_group(3, &gbuffer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::Glyph;

    #[test]
    fn the_uniform_block_matches_the_shader() {
        // 64 for each matrix, then four 16-byte rows.
        assert_eq!(std::mem::size_of::<SurfaceUniforms>(), 192);
    }

    #[test]
    fn the_sun_looks_at_the_cloud_from_the_sun() {
        let mut u: Uniforms = bytemuck::Zeroable::zeroed();
        u.spread = 1.2;
        u.size = 0.015;
        u.sun_dir = [0.3, 0.8, 0.2, 1.0];
        let f = sun_frame(&u);
        // The cloud's centre lands in the middle of the map...
        let c = f.view_proj * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(c.x.abs() < 1e-4 && c.y.abs() < 1e-4, "{c:?}");
        // ...and a point towards the sun is nearer to it than one away.
        let toward = Vec3::new(0.3, 0.8, 0.2).normalize();
        let near = f.view_proj * (toward * 0.5).extend(1.0);
        let far = f.view_proj * (-toward * 0.5).extend(1.0);
        assert!(near.z < c.z && c.z < far.z, "{} {} {}", near.z, c.z, far.z);
        assert!((0.0..1.0).contains(&near.z) && (0.0..1.0).contains(&far.z));
        // The billboard basis is square to the sun.
        assert!(f.right.dot(toward).abs() < 1e-5 && f.up.dot(toward).abs() < 1e-5);
        assert!((f.right.length() - 1.0).abs() < 1e-5 && (f.up.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_floor_far_below_the_cloud_is_still_inside_the_map() {
        let mut u: Uniforms = bytemuck::Zeroable::zeroed();
        u.spread = 1.2;
        u.sun_dir = [0.0, 1.0, 0.0, 1.0];
        let f = sun_frame(&u);
        let floor = f.view_proj * glam::Vec4::new(0.0, -20.0, 0.0, 1.0);
        assert!(floor.z < 1.0, "a floor 20 units down fell off the far plane: {}", floor.z);
    }

    // --- On the GPU -------------------------------------------------------

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

    fn camera() -> crate::camera::Camera {
        crate::camera::Camera { aspect: 1.0, elevation: 0.0, ..Default::default() }
    }

    /// A white solid sphere in front of the camera, unlit.
    fn sphere() -> Uniforms {
        let cu = camera().uniforms();
        Uniforms {
            view_proj: cu.view_proj,
            cam_right: cu.right,
            focus: 3.5,
            cam_up: cu.up,
            defocus: 0.0,
            cam_position: cu.position,
            viewport_h: 0.0,
            time: 0.0,
            aspect: 1.0,
            size: 0.02,
            spread: 0.8,
            hue: 0.5,
            saturation: 0.0,
            brightness: 1.0,
            shape: 0.0,
            morph: 0.0,
            twist: 0.0,
            palette: 0.0,
            color_spread: 0.0,
            color_drive: 0.0,
            cloud_a: 0.0,
            cloud_b: 1.0,
            cloud_morph: 0.0,
            room: Default::default(),
            lamp: Uniforms::UNLIT.lamp,
            lamp_tint: Uniforms::UNLIT.lamp_tint,
            light: Uniforms::UNLIT.light,
            sun_dir: Uniforms::UNLIT.sun_dir,
            sun_tint: Uniforms::UNLIT.sun_tint,
            stroke: [0.0; 4],
            gravity: Default::default(),
            gravity_radius: Default::default(),
            gravity_amount: Default::default(),
            palette_rows: [4.0, 0.0, 0.0, 0.0],
            video: [0.0, 1.0, 0.0, 0.0],
        }
    }

    /// Mean of r, g and b at every pixel, row-major, in linear light.
    fn frame(
        ctx: &GpuContext,
        scene: &ParticleScene,
        u: &Uniforms,
        count: u32,
        surface: bool,
        walls: Option<Walls>,
    ) -> Vec<f32> {
        frame_with(ctx, scene, u, count, surface, walls, None, None)
    }

    /// [`frame`], with a sphere-traced solid or ink in the surface pass.
    #[allow(clippy::too_many_arguments)]
    fn frame_with(
        ctx: &GpuContext,
        scene: &ParticleScene,
        u: &Uniforms,
        count: u32,
        surface: bool,
        walls: Option<Walls>,
        solid: Option<Solid>,
        ink: Option<Ink>,
    ) -> Vec<f32> {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("surface-test-target"),
            size: wgpu::Extent3d { width: W, height: W, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::post::SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("surface-test-readback"),
            size: (W * W * 8) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        if surface {
            scene.render_surface(
                ctx, &mut encoder, &view, u, count, true, wgpu::Color::BLACK, walls, solid, ink,
            );
        } else {
            scene.render(ctx, &mut encoder, &view, u, count, true, wgpu::Color::BLACK);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * 8),
                    rows_per_image: Some(W),
                },
            },
            texture.size(),
        );
        ctx.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = slice.get_mapped_range().unwrap().to_vec();
        let half = |o: usize| {
            let h = u16::from_le_bytes([bytes[o], bytes[o + 1]]);
            let exp = ((h >> 10) & 0x1f) as i32;
            let frac = (h & 0x3ff) as f32;
            match exp {
                0 => frac * 2f32.powi(-24),
                _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
            }
        };
        (0..(W * W) as usize)
            .map(|i| (half(i * 8) + half(i * 8 + 2) + half(i * 8 + 4)) / 3.0)
            .collect()
    }

    /// The pixel a world point lands on.
    fn pixel(u: &Uniforms, p: [f32; 3]) -> (usize, usize) {
        let c = Mat4::from_cols_array_2d(&u.view_proj) * Vec3::from(p).extend(1.0);
        let x = (c.x / c.w * 0.5 + 0.5) * W as f32;
        let y = (0.5 - c.y / c.w * 0.5) * W as f32;
        (x as usize, y as usize)
    }

    /// Mean over a small square of pixels, so one surfel's grain does not
    /// decide a comparison.
    fn around(px: &[f32], (x, y): (usize, usize), r: usize) -> f32 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for yy in y.saturating_sub(r)..=(y + r).min(W as usize - 1) {
            for xx in x.saturating_sub(r)..=(x + r).min(W as usize - 1) {
                sum += px[yy * W as usize + xx];
                n += 1.0;
            }
        }
        sum / n
    }

    #[test]
    fn surfels_hide_each_other_where_sprites_sum() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, crate::post::SCENE_FORMAT);
        let u = sphere();
        let max = |v: Vec<f32>| v.into_iter().fold(0.0f32, f32::max);
        let glow = max(frame(&ctx, &scene, &u, 60_000, false, None));
        let solid = max(frame(&ctx, &scene, &u, 60_000, true, None));
        // White albedo under an ambient of one can be no brighter than
        // one, however many surfels are stacked behind the front one.
        assert!(glow > 2.0, "the additive sphere should pile up light: {glow}");
        assert!(solid <= 1.01 && solid > 0.5, "an opaque sphere is its albedo, lit: {solid}");
    }

    /// Each solid traces into the G-buffer and is lit there: something
    /// covers the middle of the frame and nothing reaches the corners. A
    /// pipeline the device rejects draws nothing at all, so this is also
    /// the check that the solid's shader binds.
    #[test]
    fn solids_trace_where_the_cloud_would_be() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, crate::post::SCENE_FORMAT);
        let u = sphere();
        for kind in [SolidKind::Mandelbulb, SolidKind::Mandelbox, SolidKind::Julia, SolidKind::Menger] {
            let solid = Solid { kind, param: kind.param(0.6) };
            let px = frame_with(&ctx, &scene, &u, 0, true, None, Some(solid), None);
            let w = W as usize;
            let lit = |x0: usize, x1: usize| {
                let mut n = 0;
                for y in x0..x1 {
                    for x in x0..x1 {
                        n += (px[y * w + x] > 0.02) as usize;
                    }
                }
                n as f32 / ((x1 - x0) * (x1 - x0)) as f32
            };
            let middle = lit(w * 3 / 8, w * 5 / 8);
            assert!(middle > 0.3, "{kind:?} should cover the middle: {middle}");
            assert!(px[0] == 0.0 && px[w * w - 1] == 0.0, "{kind:?} should leave the corners");
        }
    }

    /// Hatching and stipple draw the tone: unlit, the sky leaves the top
    /// of a sphere near paper and lays ink on its underside. Outlines
    /// ink the rim, which the plain surface lights.
    #[test]
    fn ink_lays_down_the_tone_and_the_edges() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, crate::post::SCENE_FORMAT);
        // Larger surfels than the other tests, so the sphere's surface is
        // closed at this small a frame and its only edge is its rim.
        let u = Uniforms { size: 0.05, ..sphere() };
        let (top, bottom) = (pixel(&u, [0.0, 0.75, 0.0]), pixel(&u, [0.0, -0.7, 0.0]));
        for kind in [InkKind::Hatch, InkKind::Stipple] {
            let ink = Some(Ink { kind, weight: 0.5 });
            let px = frame_with(&ctx, &scene, &u, 60_000, true, None, None, ink);
            let (t, b) = (around(&px, top, 4), around(&px, bottom, 4));
            assert!(t > 0.7, "{kind:?}: the lit top should be mostly paper: {t}");
            // Measured at 0.62 against 0.79 for hatching on lavapipe.
            assert!(b < t - 0.1, "{kind:?}: the underside should take ink: {b} against {t}");
        }
        let plain = frame(&ctx, &scene, &u, 60_000, true, None);
        let inked = frame_with(
            &ctx, &scene, &u, 60_000, true, None, None,
            Some(Ink { kind: InkKind::Outline, weight: 0.5 }),
        );
        // Along the middle row, the first covered pixel from the left is
        // the rim.
        let row = W as usize / 2 * W as usize;
        let rim = (0..W as usize).find(|&x| plain[row + x] > 0.0).expect("the sphere is in frame");
        assert!(plain[row + rim] > 0.2, "the plain rim is lit: {}", plain[row + rim]);
        assert!(inked[row + rim] < 0.1, "the inked rim is pen: {}", inked[row + rim]);
    }

    #[test]
    fn the_solid_knob_stays_in_each_kinds_range() {
        for d in [0.0, 0.25, 0.49, 0.5, 0.75, 1.0] {
            assert!((2.0..=12.0).contains(&SolidKind::Mandelbulb.param(d)));
            let k = SolidKind::Mandelbox.param(d);
            assert!((-2.6..=-1.5).contains(&k) || (2.0..=3.0).contains(&k), "box scale {k}");
            assert!((0.0..=std::f32::consts::TAU).contains(&SolidKind::Julia.param(d)));
            let levels = SolidKind::Menger.param(d);
            assert!((1.0..=6.0).contains(&levels) && levels.fract() == 0.0);
        }
        assert_eq!(SolidKind::from_index(0.0), None);
        assert_eq!(SolidKind::from_index(4.2), Some(SolidKind::Menger));
    }

    /// Glyphs are solid and lit by their own faces. With the sun off to
    /// the side, square to the view, a sparse field of discs is nearly
    /// dim: each disc faces the camera, and the surface the depth buffer
    /// reads from so few of them is mostly turned the same way. Cubes in
    /// the same place turn faces towards the sun whichever way they are
    /// rolled, and light. Measured at 2.1 times the discs on lavapipe.
    #[test]
    fn glyphs_light_by_their_faces() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, crate::post::SCENE_FORMAT);
        let mut u = sphere();
        u.size = 0.07;
        u.light = [0.0, 1.0, 0.0, 0.0];
        let side = Vec3::from(u.cam_right);
        u.sun_dir = [side.x, side.y, side.z, 3.0];
        let mean_lit = |u: &Uniforms| {
            let px = frame(&ctx, &scene, u, 300, true, None);
            px.iter().sum::<f32>() / W as f32 / W as f32
        };
        let discs = mean_lit(&u);
        u.stroke[3] = Glyph::Cube.lane();
        let cubes = mean_lit(&u);
        let max = frame(&ctx, &scene, &u, 300, true, None).into_iter().fold(0.0f32, f32::max);
        assert!(max < 3.5, "a glyph is opaque, not summed: {max}");
        assert!(cubes > discs * 1.6 && cubes > 1e-3, "cubes {cubes:.4} against discs {discs:.4}");
    }

    #[test]
    fn a_lamp_lights_the_side_of_the_cloud_that_faces_it() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, crate::post::SCENE_FORMAT);
        let mut u = sphere();
        u.light = [0.02, 1.0, 0.0, 0.0];
        u.lamp[0] = [3.0, 0.0, 0.0, 3.0];
        u.lamp_tint[0] = [1.0, 1.0, 1.0, 6.0];
        let px = frame(&ctx, &scene, &u, 60_000, true, None);
        let r = 0.5 * u.spread;
        let right = around(&px, pixel(&u, [r, 0.0, 0.0]), 3);
        let left = around(&px, pixel(&u, [-r, 0.0, 0.0]), 3);
        // Every surfel is a disc facing the camera, so each on its own
        // faces the lamp equally. Only the orientation of the surface they
        // make together can tell the two sides apart — which is what the
        // shading pass reads out of the depth buffer.
        assert!(
            right > left * 2.0,
            "the lamp side is not lit more than the far side: {right:.3} against {left:.3}"
        );
    }

    #[test]
    fn the_sun_casts_the_cloud_on_the_floor() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, crate::post::SCENE_FORMAT);
        let cam = camera();
        let room = crate::room::RoomUniforms::for_camera(&cam, 1.9, 6.0, 1.0, 0.0, 1.0, 0.0, 0.0);
        let mut u = sphere();
        u.spread = 0.35;
        u.room = room.placement(0.4, 1.0);
        u.light = [0.02, 1.0, 0.0, 0.0];
        // Straight down.
        u.sun_dir = [0.0, 1.0, 0.0, 3.0];
        let walls = Some(Walls { brightness: 1.0, fade: 0.0 });
        let lit = frame(&ctx, &scene, &u, 40_000, true, walls);

        let (centre, _) = u.room.place([0.0, 0.0, 0.0]);
        let floor = -room.half_y;
        let below = around(&lit, pixel(&u, [centre[0], floor, centre[2]]), 1);
        let aside = around(&lit, pixel(&u, [centre[0] + 0.7 * room.half_x, floor, centre[2]]), 1);
        assert!(aside > 0.01, "the floor beside the cloud should be sunlit: {aside}");
        assert!(
            below < aside * 0.3,
            "no shadow under the cloud: {below:.4} below it, {aside:.4} beside it"
        );

        // And it is the shadow, not the floor: with the sun off, the two
        // spots are the same plaster under the same sky.
        u.sun_dir[3] = 0.0;
        let unlit = frame(&ctx, &scene, &u, 40_000, true, walls);
        let below = around(&unlit, pixel(&u, [centre[0], floor, centre[2]]), 1);
        let aside = around(&unlit, pixel(&u, [centre[0] + 0.7 * room.half_x, floor, centre[2]]), 1);
        assert!((below - aside).abs() <= 0.25 * aside.max(1e-4), "{below} {aside}");
    }
}
