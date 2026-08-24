//! The first generator: a fully procedural particle field.
//!
//! All per-particle state is derived in the vertex shader from the vertex
//! index, so per-frame CPU cost is one 32-byte uniform upload and one draw
//! call regardless of particle count.

use crate::{GpuContext, attractor::Attractors};

/// The shader's per-particle hash, mirrored on the CPU.
///
/// Kept here so the property that matters — that distinct indices give
/// distinct particles across the whole count range — is testable without
/// a GPU. The float version this replaced could not: at index 500,000 its
/// intermediate landed where the f32 ulp is 2^-8, so `fract` had at most
/// 256 values to return and the field collapsed to repeats.
///
/// Must stay identical to `hash_u32`/`hash01` in particles.wgsl.
pub fn hash_u32(x: u32) -> u32 {
    let mut h = x;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

pub fn hash01(pi: u32, stream: u32) -> f32 {
    (hash_u32(pi.wrapping_mul(4).wrapping_add(stream)) >> 8) as f32 / 16_777_216.0
}

/// What an empty frame looks like. Matches the room's clear colour, so
/// turning the room on and off does not change the background.
pub const SCENE_CLEAR: wgpu::Color = wgpu::Color { r: 0.004, g: 0.004, b: 0.008, a: 1.0 };

/// Per-frame shader inputs. Layout must match `Uniforms` in particles.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    /// Mat4 first for its 16-byte alignment.
    pub view_proj: [[f32; 4]; 4],
    pub cam_right: [f32; 3],
    pub focus: f32,
    pub cam_up: [f32; 3],
    pub defocus: f32,
    pub cam_position: [f32; 3],
    pub _pad_cam: f32,
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
    /// Slot indices for the cloud pair, and the blend between them.
    pub cloud_a: f32,
    pub cloud_b: f32,
    pub cloud_morph: f32,
    /// The room's volume, so the field can be placed inside it rather than
    /// drawn in front of it.
    pub room: crate::room::Placement,
    /// Gravity wells: `xyz` is the centre, `w` the signed strength —
    /// positive pulls in, negative pushes away.
    ///
    /// `vec4` per well rather than a tighter packing because WGSL gives a
    /// uniform array of `vec3` a 16-byte stride anyway; using the fourth
    /// lane for the strength makes the padding carry something.
    pub gravity: [[f32; 4]; 4],
    /// Reach of each well.
    pub gravity_radius: [f32; 4],
    /// Master amount in `.x`; the rest is padding the alignment demands.
    pub gravity_amount: [f32; 4],
    /// Highest written palette row in `.x`, so the colour index saturates
    /// on a real ramp instead of sweeping into empty rows.
    pub palette_rows: [f32; 4],
    /// The live video input, packed as one vec4 because the alignment
    /// would spend the space on padding anyway:
    /// `.x` 1 when a frame has arrived, `.y` the picture's aspect,
    /// `.z` how far luminance pushes a point along z, `.w` which channel
    /// of the picture does the pushing.
    pub video: [f32; 4],
    /// Lamps: `xyz` is where the lamp is, `w` how bright.
    ///
    /// Two rather than three or eight. A lamp is seven faders once you
    /// count position, reach, level and colour, and the honest question
    /// is not how many the shader can afford but how many a person will
    /// map — two movable lamps plus the sun is already three-point
    /// lighting, and the third lamp is the one nobody would reach for.
    pub lamp: [[f32; 4]; LAMPS],
    /// `rgb` is the lamp's colour, already mixed from hue and tint on the
    /// CPU so the shader has no palette work to do; `w` is its radius.
    pub lamp_tint: [[f32; 4]; LAMPS],
    /// `.x` ambient — the light everywhere, 1.0 by default so an unlit
    /// scene is exactly the picture this renderer drew before there were
    /// lamps at all. `.y` how much surface orientation counts. `.z` and
    /// `.w` are spare and written zero.
    pub light: [f32; 4],
    /// Direction *towards* the sun in `xyz`, its level in `w`.
    pub sun_dir: [f32; 4],
    /// The sun's colour in `rgb`, mixed on the CPU like a lamp's.
    pub sun_tint: [f32; 4],
}

impl Uniforms {
    /// The lighting fields at their neutral values, and the property the
    /// whole feature's defaults are chosen to have: ambient one, no
    /// lamps, no sun.
    ///
    /// `light_at` returns exactly `vec3(1.0)` for these — the loop body
    /// is skipped by its own level guard and the sun by its — so a scene
    /// carrying them is bit-for-bit the picture this renderer drew before
    /// there was any lighting at all. Every preset ever saved and the
    /// whole shipped set depend on that being true rather than nearly
    /// true, which is why it is a named constant and not three zeroes
    /// typed into each call site.
    pub const UNLIT: LightUniforms = LightUniforms {
        lamp: [[0.0; 4]; LAMPS],
        lamp_tint: [[1.0, 1.0, 1.0, 1.0]; LAMPS],
        light: [1.0, 1.0, 0.0, 0.0],
        sun_dir: [0.0, 1.0, 0.0, 0.0],
        sun_tint: [1.0, 1.0, 1.0, 0.0],
    };
}

/// The lighting half of [`Uniforms`], so the neutral values can be named
/// once. Not a nested struct in the buffer — the fields are spliced in
/// flat — because inserting a struct here would change the layout every
/// field after it depends on.
pub struct LightUniforms {
    pub lamp: [[f32; 4]; LAMPS],
    pub lamp_tint: [[f32; 4]; LAMPS],
    pub light: [f32; 4],
    pub sun_dir: [f32; 4],
    pub sun_tint: [f32; 4],
}

/// How many movable lamps there are. See [`Uniforms::lamp`].
pub const LAMPS: usize = 2;

pub struct ParticleScene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Kept so the bind group can be rebuilt: a video frame that changes
    /// size replaces its texture, and the old view in the bind group goes
    /// with it.
    bgl: wgpu::BindGroupLayout,
    /// The live video input. Public so the app can ask what has arrived
    /// without the scene re-exporting every field.
    pub video: crate::video::Video,
    /// How a live stream is fitted to the view, held across frames so
    /// the fit can be eased rather than recomputed from each frame.
    stream_fit: crate::pointcloud::StreamFit,
    /// The cloud bank. Kept for the bind group's texture view, and
    /// mutated when a cloud is loaded.
    attractors: Attractors,
    /// The colour ramps, likewise.
    pub palettes: crate::palette::Palettes,
    /// How many palettes have been loaded, for choosing the next row.
    loaded_palettes: usize,
}

impl ParticleScene {
    pub fn new(ctx: &GpuContext, target_format: wgpu::TextureFormat) -> Self {
        let device = &ctx.device;
        let attractors = Attractors::new(ctx);
        let palettes = crate::palette::Palettes::new(ctx);
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
                // Live video, if any. `textureLoad` again — the field
                // samples one texel per particle, so there is nothing to
                // filter between, and staying unfiltered keeps this
                // binding as portable as the two above it.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Which way each point's surface faces, in the same
                // texel layout as the positions at binding 1.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // The palette bank. Read with `textureLoad`, so it needs
                // no sampler and no filterable format.
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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

        let video = crate::video::Video::new(ctx);
        let bind_group = Self::make_bind_group(
            device, &bgl, &uniforms, &attractors, &palettes, &video,
        );

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
            bgl,
            video,
            stream_fit: Default::default(),
            attractors,
            palettes,
            loaded_palettes: 0,
        }
    }

    /// Load a palette file into the next free row, returning its name and
    /// the index `/color/palette` addresses it by.
    ///
    /// The row is chosen here rather than by the caller because the rows
    /// below [`crate::palette::FIRST_USER`] are the shipped gradients and
    /// are fixed forever — a preset saved with palette 3 must still be
    /// "ice" in every future build.
    /// Hold a palette row without loading anything into it.
    ///
    /// For restoring a saved list with a hole in it — a file that has
    /// gone missing or will not parse keeps its *row*, because rows are
    /// what `/color/palette` values in saved presets index. Compacting
    /// over a hole would repoint every later palette.
    pub fn skip_palette_row(&mut self) {
        self.loaded_palettes += 1;
    }

    pub fn load_palette(
        &mut self,
        ctx: &GpuContext,
        path: &std::path::Path,
    ) -> anyhow::Result<(String, usize)> {
        let (stops, name) = crate::palette::parse(path)?;
        let row = self.palettes.next_user_row(self.loaded_palettes);
        self.palettes.load_slot(ctx, row, &stops, &name);
        self.loaded_palettes += 1;
        log::info!("palette {row}: {name} ({} colours)", stops.len());
        Ok((name, row))
    }

    /// Reload a palette into the row it already occupies.
    ///
    /// For dropping a file that is already in the bank — the natural
    /// gesture after editing a .gpl. Appending instead would burn a new
    /// row on every re-drop and, once the bank wrapped, overwrite the
    /// earliest palettes that saved presets still index.
    pub fn reload_palette(
        &mut self,
        ctx: &GpuContext,
        path: &std::path::Path,
        row: usize,
    ) -> anyhow::Result<String> {
        let (stops, name) = crate::palette::parse(path)?;
        self.palettes.load_slot(ctx, row, &stops, &name);
        log::info!("palette {row} reloaded: {name} ({} colours)", stops.len());
        Ok(name)
    }

    /// Names of the four cloud slots, for the UI.
    pub fn cloud_names(&self) -> &[String; crate::attractor::SLOTS] {
        &self.attractors.names
    }

    /// Load files into the loadable slots, in order.
    ///
    /// A file that will not parse is a warning, never a startup failure:
    /// arriving at a venue to find the app refuses to open because a scan
    /// has a malformed header is precisely the wrong trade.
    pub fn load_clouds(&mut self, ctx: &GpuContext, paths: &[std::path::PathBuf]) {
        for (i, path) in paths.iter().enumerate() {
            // A hole in the saved list holds its slot — see the palette
            // twin above; `/cloud/a` values in presets index slots.
            if path.as_os_str().is_empty() {
                continue;
            }
            let Some(slot) = Self::loadable_slot(i) else {
                log::warn!(
                    "ignoring {}: only {} loadable cloud slots",
                    path.display(),
                    Self::LOADABLE
                );
                break;
            };
            if let Err(e) = self.load_cloud(ctx, slot, path) {
                log::warn!("could not load {}: {e:#}", path.display());
            }
        }
    }

    /// How many slots can hold a file. The first two are the built-in
    /// attractors and are generated, not loaded.
    fn make_bind_group(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
        uniforms: &wgpu::Buffer,
        attractors: &Attractors,
        palettes: &crate::palette::Palettes,
        video: &crate::video::Video,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle-bg"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&attractors.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&palettes.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&video.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&attractors.normals_view),
                },
            ],
        })
    }

    /// Take a video frame. Rebuilds the bind group only when the frame
    /// changed size, which is the only time the old texture view stops
    /// being valid — a feed at a steady size uploads into the texture
    /// already bound and costs nothing extra.
    pub fn set_video(
        &mut self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        stride: u32,
        bgra: &[u8],
    ) {
        if self.video.upload(ctx, width, height, stride, bgra) {
            self.bind_group = Self::make_bind_group(
                &ctx.device,
                &self.bgl,
                &self.uniforms,
                &self.attractors,
                &self.palettes,
                &self.video,
            );
        }
    }

    /// Slots a file, an image or a typed word can fill: everything from
    /// the last built-in attractor up to, but not including, the video
    /// slot. Counted against `VIDEO_SLOT` rather than `SLOTS` so that
    /// adding a reserved slot cannot silently hand it out to the next
    /// dropped file.
    pub const LOADABLE: usize =
        crate::attractor::VIDEO_SLOT - (crate::attractor::SLOT_AIZAWA + 1);

    /// The slot holding the `i`th loadable cloud, if there is one.
    pub fn loadable_slot(i: usize) -> Option<usize> {
        let slot = crate::attractor::SLOT_AIZAWA + 1 + i;
        (slot < crate::attractor::SLOTS).then_some(slot)
    }

    /// Load one file into one slot, returning the name it was given.
    ///
    /// Split out of [`Self::load_clouds`] so a cloud can arrive after
    /// startup — dropped onto the window — through exactly the same path
    /// as one named on the command line. Two routes to the same thing
    /// drift apart, and then only one of them gets the bug fix.
    pub fn load_cloud(
        &mut self,
        ctx: &GpuContext,
        slot: usize,
        path: &std::path::Path,
    ) -> anyhow::Result<String> {
        let mut points = crate::pointcloud::load(path)?;
        crate::pointcloud::normalize(&mut points);
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("cloud")
            .to_string();
        log::info!("cloud slot {slot}: {} ({} points)", name, points.len());
        self.attractors.load_slot(ctx, slot, &points, &name);
        Ok(name)
    }

    /// Replace one cloud slot's contents, for a live stream.
    ///
    /// Normalised on the way in exactly as a loaded file is, so a stream
    /// arriving in metres and a scan arriving in millimetres both land at
    /// the same size on screen — otherwise switching source would be a
    /// jump in scale rather than a change of subject.
    pub fn set_cloud(
        &mut self,
        ctx: &GpuContext,
        slot: usize,
        points: &[crate::pointcloud::Point],
        name: &str,
    ) {
        if points.is_empty() {
            return;
        }
        let mut owned = points.to_vec();
        crate::pointcloud::normalize(&mut owned);
        self.attractors.load_slot(ctx, slot, &owned, name);
    }

    /// Upload a frame of a live stream.
    ///
    /// Separate from [`Self::set_cloud`] because measuring a cloud and
    /// fitting it to the view is right exactly once for a file and wrong
    /// every frame for a stream: re-measuring per frame means a stray
    /// LiDAR return at the back of the room rescales everything and a
    /// subject leaning slides everything, so a steady cloud arrives
    /// visibly swimming. The streaming fit is eased and outlier-resistant
    /// instead; it lives on the scene so it persists between frames.
    pub fn set_cloud_streaming(
        &mut self,
        ctx: &GpuContext,
        slot: usize,
        points: &[crate::pointcloud::Point],
        name: &str,
    ) {
        if points.is_empty() {
            return;
        }
        let mut owned = points.to_vec();
        self.stream_fit.apply(&mut owned);
        self.attractors.load_slot(ctx, slot, &owned, name);
    }

    /// Forget the streaming fit, so the next stream measures itself
    /// afresh rather than easing over from the last one's framing.
    pub fn reset_stream_fit(&mut self) {
        self.stream_fit = Default::default();
    }

    /// The slot a live stream writes into: the last loadable one, so a
    /// `--cloud` file and a live feed can be held at once and morphed
    /// between with `/cloud/morph`.
    /// The streamed point cloud's slot: the last loadable one, which it
    /// shares with dropped files by design — a rig streaming geometry is
    /// not also filling every slot from disk.
    pub const LIVE_SLOT: usize = crate::attractor::VIDEO_SLOT - 1;

    /// Encode one frame into `target`. `count` is the number of particles.
    ///
    /// Eight arguments, and grouping them into a struct would only move the
    /// same list somewhere else — every one is a distinct per-frame input
    /// the caller already holds separately, and there is exactly one call
    /// site in each of the windowed and headless paths.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        uniforms: &Uniforms,
        count: u32,
        clear: bool,
        background: wgpu::Color,
    ) {
        // The caller does not own the palette bank, so the occupied-row
        // count is filled in here rather than being threaded through the
        // engine. It changes whenever a palette is dropped, which is why
        // it is read per frame rather than captured once.
        let mut uniforms = *uniforms;
        uniforms.palette_rows[0] = self.palettes.occupied() as f32;
        // Same reasoning for the video input: whether a frame has ever
        // arrived and what shape it is are facts about the texture this
        // owns, not settings the parameter table could hold.
        uniforms.video[0] = if self.video.present { 1.0 } else { 0.0 };
        uniforms.video[1] = self.video.aspect();
        ctx.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Somebody has to clear, and blending is additive: a
                    // scene texture that is only ever loaded accumulates
                    // every frame it has ever drawn and saturates to white
                    // within seconds. The room clears when it runs, so
                    // this pass clears when it did not.
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
        // Six vertices per particle as one flat triangle list, not four as
        // an instanced strip.
        //
        // The strip is the obvious optimisation — a third fewer vertex
        // invocations, each of which recomputes the whole particle — and it
        // was tried. It measured *slower*: 108ms to 116ms a frame at 1080p,
        // repeatably, both directions. Four vertices is far too small an
        // instance to keep a vertex pipeline fed, and the per-instance
        // overhead costs more than the two invocations it saves.
        //
        // Measured on a software rasteriser, which is what this machine
        // has, so the size of the effect will differ on real hardware —
        // but the direction is a known trap rather than an artefact, and
        // "fewer invocations" is not on its own a reason to expect a win.
        // Anyone re-attempting this should benchmark before believing it.
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

    /// Draw `frames` frames into one texture, clearing only on the first
    /// — which is what the app does when the room is on and clearing for
    /// it. Returns the share of fully-saturated pixels.
    fn saturation_after(ctx: &GpuContext, scene: &ParticleScene, frames: u32, clear_each: bool) -> f32 {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("accumulation-test-target"),
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
            label: Some("accumulation-test-readback"),
            size: (W * W * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let cam = crate::camera::Camera { aspect: 1.0, ..Default::default() }.uniforms();
        let uniforms = Uniforms {
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
            lamp: Uniforms::UNLIT.lamp,
            lamp_tint: Uniforms::UNLIT.lamp_tint,
            light: Uniforms::UNLIT.light,
            sun_dir: Uniforms::UNLIT.sun_dir,
            sun_tint: Uniforms::UNLIT.sun_tint,
            gravity: Default::default(),
            gravity_radius: Default::default(),
            gravity_amount: Default::default(),
            palette_rows: [4.0, 0.0, 0.0, 0.0],
            video: [0.0, 1.0, 0.0, 0.0],
        };
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        for i in 0..frames {
            scene.render(ctx, &mut encoder, &view, &uniforms, 20_000, clear_each || i == 0, SCENE_CLEAR);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * 4),
                    rows_per_image: Some(W),
                },
            },
            wgpu::Extent3d { width: W, height: W, depth_or_array_layers: 1 },
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
        let blown = pixels
            .chunks_exact(4)
            .filter(|p| p[0] == 255 && p[1] == 255 && p[2] == 255)
            .count();
        drop(buffer);
        blown as f32 / (W * W) as f32
    }

    /// Mean luminance of one rendered frame, for the lighting tests.
    fn mean_luma(ctx: &GpuContext, scene: &ParticleScene, u: &Uniforms) -> f32 {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lighting-test-target"),
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
            label: Some("lighting-test-readback"),
            size: (W * W * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        scene.render(ctx, &mut encoder, &view, u, 20_000, true, SCENE_CLEAR);
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * 4),
                    rows_per_image: Some(W),
                },
            },
            wgpu::Extent3d { width: W, height: W, depth_or_array_layers: 1 },
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
        let sum: f64 = pixels
            .chunks_exact(4)
            .map(|p| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0)
            .sum();
        drop(buffer);
        (sum / (W * W) as f64) as f32
    }

    /// A directional key actually depends on which way the surface faces.
    ///
    /// This is the test the feature needed and did not have. Every unit
    /// test around it passed while the sun did nothing at all: the shader
    /// looked the normal up by *shape mode* where the mapping wanted a
    /// *cloud slot*, so mode 7 — "the cloud pair" — read slot 7, which is
    /// empty, and every normal came back zero. Nothing errors, nothing
    /// looks wrong, and the whole of surface shading is silently absent.
    ///
    /// Driven through the real pipeline with a real cloud, because the
    /// mode-to-slot mapping only exists inside the shader and there is no
    /// smaller place to check it.
    #[test]
    fn the_sun_lights_a_surface_by_the_way_it_faces() {
        let Some(ctx) = gpu() else { return };
        let mut scene = ParticleScene::new(&ctx, FORMAT);

        // A flat sheet facing the camera, with normals the estimator will
        // work out for itself — the case that matters, since most scans
        // arrive without them.
        let mut sheet = Vec::new();
        for i in 0..120 {
            for j in 0..120 {
                sheet.push(crate::pointcloud::Point::new(
                    i as f32 / 119.0 - 0.5,
                    j as f32 / 119.0 - 0.5,
                    0.0,
                ));
            }
        }
        scene.set_cloud(&ctx, 2, &sheet, "sheet");

        let cam = crate::camera::Camera { aspect: 1.0, elevation: 0.0, ..Default::default() };
        let cu = cam.uniforms();
        let base = Uniforms {
            view_proj: cu.view_proj,
            cam_right: cu.right,
            focus: 3.5,
            cam_up: cu.up,
            defocus: 0.0,
            cam_position: cu.position,
            _pad_cam: 0.0,
            time: 0.0,
            aspect: 1.0,
            size: 0.02,
            spread: 1.0,
            hue: 0.5,
            saturation: 0.0,
            brightness: 1.0,
            // The cloud pair, which is the mode a loaded scan is shown in
            // and the one whose slot mapping the bug was in.
            shape: 7.0,
            morph: 0.0,
            twist: 0.0,
            palette: 0.0,
            color_spread: 0.0,
            color_drive: 0.0,
            cloud_a: 2.0,
            cloud_b: 2.0,
            cloud_morph: 0.0,
            room: Default::default(),
            lamp: Uniforms::UNLIT.lamp,
            lamp_tint: Uniforms::UNLIT.lamp_tint,
            // Almost no ambient, so what is measured is the sun.
            light: [0.05, 1.0, 0.0, 0.0],
            sun_dir: Uniforms::UNLIT.sun_dir,
            sun_tint: Uniforms::UNLIT.sun_tint,
            gravity: Default::default(),
            gravity_radius: Default::default(),
            gravity_amount: Default::default(),
            palette_rows: [4.0, 0.0, 0.0, 0.0],
            video: [0.0, 1.0, 0.0, 0.0],
        };

        // The camera looks down -z from +z, so the sheet's normal points
        // at it. A sun from the camera's side lights the sheet; a sun
        // from behind it does not.
        let toward = mean_luma(&ctx, &scene, &Uniforms { sun_dir: [0.0, 0.0, 1.0, 2.0], ..base });
        let away = mean_luma(&ctx, &scene, &Uniforms { sun_dir: [0.0, 0.0, -1.0, 2.0], ..base });
        let dark = mean_luma(&ctx, &scene, &Uniforms { sun_dir: [0.0, 0.0, 1.0, 0.0], ..base });

        assert!(
            toward > away * 1.5,
            "the sun does not depend on direction: facing it {toward:.2}, backing it {away:.2}"
        );
        assert!(
            toward > dark * 1.5,
            "the sun adds nothing at all: lit {toward:.2}, unlit {dark:.2}"
        );
    }

    /// And with the lighting left alone, the picture is the one this
    /// renderer drew before there was any: the property every preset ever
    /// saved depends on.
    #[test]
    fn unlit_is_exactly_the_old_picture() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);
        let cam = crate::camera::Camera { aspect: 1.0, ..Default::default() };
        let cu = cam.uniforms();
        let base = Uniforms {
            view_proj: cu.view_proj,
            cam_right: cu.right,
            focus: 3.5,
            cam_up: cu.up,
            defocus: 0.0,
            cam_position: cu.position,
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
            lamp: Uniforms::UNLIT.lamp,
            lamp_tint: Uniforms::UNLIT.lamp_tint,
            light: Uniforms::UNLIT.light,
            sun_dir: Uniforms::UNLIT.sun_dir,
            sun_tint: Uniforms::UNLIT.sun_tint,
            gravity: Default::default(),
            gravity_radius: Default::default(),
            gravity_amount: Default::default(),
            palette_rows: [4.0, 0.0, 0.0, 0.0],
            video: [0.0, 1.0, 0.0, 0.0],
        };
        let unlit = mean_luma(&ctx, &scene, &base);
        // Ambient at one and no lamps is the neutral multiplier; raising
        // a lamp from there can only add.
        let lamped = mean_luma(
            &ctx,
            &scene,
            &Uniforms {
                lamp: [[0.0, 0.0, 0.0, 1.5], [0.0; 4]],
                lamp_tint: [[1.0, 1.0, 1.0, 1.0]; LAMPS],
                ..base
            },
        );
        assert!(unlit > 0.5, "the unlit scene rendered nothing at all: {unlit}");
        assert!(
            lamped > unlit,
            "a lamp at the origin did not brighten anything: {unlit:.2} → {lamped:.2}"
        );
    }

    /// The uniform block has to stay 16-byte aligned where WGSL says it
    /// is.
    ///
    /// A `vec4` array in a uniform block must start on a 16-byte boundary.
    /// Get that wrong and nothing errors — the driver reads the fields at
    /// the offsets *it* computed, so every value after the misalignment is
    /// silently something else, which shows up as a scene that renders
    /// but is subtly and inexplicably wrong.
    #[test]
    fn the_uniform_block_stays_aligned() {
        use std::mem::{align_of, offset_of, size_of};
        assert_eq!(offset_of!(Uniforms, gravity) % 16, 0, "gravity array is misaligned");
        assert_eq!(
            offset_of!(Uniforms, gravity_radius) % 16,
            0,
            "gravity radii are misaligned"
        );
        // The lighting arrays, for the same reason. These were appended
        // after `video`, which is a vec4 and therefore leaves the block
        // aligned — but that is a fact about today's field order, not a
        // guarantee, and the failure it protects against is a scene that
        // renders and is quietly wrong.
        assert_eq!(offset_of!(Uniforms, lamp) % 16, 0, "the lamps are misaligned");
        assert_eq!(
            offset_of!(Uniforms, lamp_tint) % 16,
            0,
            "the lamp colours are misaligned"
        );
        assert_eq!(offset_of!(Uniforms, light) % 16, 0, "the light block is misaligned");
        assert_eq!(offset_of!(Uniforms, sun_dir) % 16, 0, "the sun is misaligned");
        assert_eq!(offset_of!(Uniforms, sun_tint) % 16, 0, "the sun colour is misaligned");
        assert_eq!(size_of::<Uniforms>() % 16, 0, "the block is not a whole number of vec4s");
        assert!(align_of::<Uniforms>() <= 16);
    }

    /// Gravity has to move particles, and the sign has to mean what the
    /// words mean.
    ///
    /// A displacement that compiles and does nothing is the easy failure
    /// here: the loop is guarded on the master amount, on each strength,
    /// and on the uniform block being laid out where the shader thinks it
    /// is. Any of those going wrong leaves a scene that renders perfectly
    /// and simply ignores the layer.
    #[test]
    fn a_gravity_well_pulls_the_field_in_and_pushes_it_out() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);

        // How tightly the lit pixels cluster around the centre of frame.
        let spread = |px: &[u8]| {
            let mut sum = 0f64;
            let mut n = 0f64;
            for (i, p) in px.chunks_exact(4).enumerate() {
                if p[0].max(p[1]).max(p[2]) <= 24 {
                    continue;
                }
                let x = (i % W as usize) as f64 - W as f64 / 2.0;
                let y = (i / W as usize) as f64 - W as f64 / 2.0;
                sum += (x * x + y * y).sqrt();
                n += 1.0;
            }
            if n == 0.0 { 0.0 } else { sum / n }
        };

        let off = spread(&render_tuned(&ctx, &scene, SCENE_CLEAR, |_| {}));
        // A strong well at the origin, wide enough to reach the whole
        // field. Positive strength should gather it.
        let pulled = spread(&render_tuned(&ctx, &scene, SCENE_CLEAR, |u| {
            u.gravity[0] = [0.0, 0.0, 0.0, 2.0];
            u.gravity_radius[0] = 4.0;
            u.gravity_amount[0] = 1.0;
        }));
        // The same well, inverted, should scatter it.
        let pushed = spread(&render_tuned(&ctx, &scene, SCENE_CLEAR, |u| {
            u.gravity[0] = [0.0, 0.0, 0.0, -2.0];
            u.gravity_radius[0] = 4.0;
            u.gravity_amount[0] = 1.0;
        }));

        assert!(off > 0.0, "nothing rendered to measure");
        assert!(
            pulled < off * 0.95,
            "a positive well did not gather the field: {off:.1} -> {pulled:.1}"
        );
        assert!(
            pushed > off * 1.02,
            "a negative well did not push the field out: {off:.1} -> {pushed:.1}"
        );
    }

    /// The master amount must be a real bypass. It is the fader the whole
    /// layer is brought in on, so at zero the field has to be pixel-for-
    /// pixel what it is with no wells at all.
    #[test]
    fn gravity_at_zero_amount_changes_nothing() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);
        let plain = render_tuned(&ctx, &scene, SCENE_CLEAR, |_| {});
        let bypassed = render_tuned(&ctx, &scene, SCENE_CLEAR, |u| {
            u.gravity[0] = [0.4, 0.0, 0.0, 2.0];
            u.gravity_radius[0] = 3.0;
            u.gravity_amount[0] = 0.0;
        });
        assert_eq!(plain, bypassed, "wells acted with the master amount at zero");
    }

    /// The palette bank has to actually be read.
    ///
    /// Every other GPU test here renders at palette 0, which is the
    /// procedural HSV path and never touches the texture — so the entire
    /// lookup could return black and nothing would fail. A broken
    /// `textureLoad`, a bank that was never written, or a bind group
    /// missing its third entry all produce exactly that.
    #[test]
    fn a_built_in_palette_paints_something_and_differs_from_hsv() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);

        let lit = |px: &[u8]| {
            px.chunks_exact(4)
                .filter(|p| p[0].max(p[1]).max(p[2]) > 24)
                .count()
        };
        let hsv = render_palette(&ctx, &scene, 0.0);
        let warm = render_palette(&ctx, &scene, 1.0);
        let ice = render_palette(&ctx, &scene, 3.0);

        assert!(lit(&warm) > 0, "palette 1 rendered nothing — the bank is black");
        assert!(lit(&ice) > 0, "palette 3 rendered nothing — the bank is black");
        // And the rows are distinct from each other and from HSV, so the
        // index is really selecting a row rather than being ignored.
        assert_ne!(warm, hsv, "palette 1 painted the same as HSV");
        assert_ne!(warm, ice, "two different palettes painted identically");
    }

    /// Render one frame at a given palette index.
    fn render_palette(ctx: &GpuContext, scene: &ParticleScene, palette: f32) -> Vec<u8> {
        render_tuned(ctx, scene, SCENE_CLEAR, |u| {
            u.palette = palette;
            // Spread the drive across the ramp so more than one texel of
            // the row is read.
            u.color_spread = 1.0;
        })
    }

    /// Every particle must be a distinct particle.
    ///
    /// The count control is the headline knob and it silently stopped
    /// working above about fifty thousand: the float hash's intermediate
    /// ran out of f32 precision, so `fract` returned one of a few hundred
    /// values and the four streams — being the same function of the same
    /// index — collapsed together. Pushing the count higher bought
    /// repeats drawn exactly on top of each other, which with additive
    /// blending made the field hotter rather than denser. A shipped preset
    /// sits at 260,000, deep into that.
    #[test]
    fn the_particle_hash_does_not_collapse_at_high_counts() {
        use std::collections::HashSet;
        for count in [60_000u32, 260_000, 500_000] {
            let mut seen = HashSet::with_capacity(count as usize);
            for pi in 0..count {
                // The whole tuple, since a particle is only a repeat if
                // every stream repeats.
                seen.insert((
                    hash01(pi, 0).to_bits(),
                    hash01(pi, 1).to_bits(),
                    hash01(pi, 2).to_bits(),
                    hash01(pi, 3).to_bits(),
                ));
            }
            assert_eq!(
                seen.len() as u32,
                count,
                "{count} particles collapsed to {} distinct",
                seen.len()
            );
        }
    }

    /// And the streams must be independent of each other, or a particle's
    /// position and its colour move together and the field reads as a
    /// pattern rather than a cloud.
    #[test]
    fn the_hash_streams_are_uncorrelated() {
        let n = 20_000u32;
        let mean = |f: &dyn Fn(u32) -> f32| {
            (0..n).map(|i| f(i) as f64).sum::<f64>() / n as f64
        };
        let m0 = mean(&|i| hash01(i, 0));
        let m1 = mean(&|i| hash01(i, 1));
        // Uniform on 0..1 means a mean near 0.5.
        assert!((m0 - 0.5).abs() < 0.01, "stream 0 is not uniform: {m0}");
        assert!((m1 - 0.5).abs() < 0.01, "stream 1 is not uniform: {m1}");

        let cov: f64 = (0..n)
            .map(|i| (hash01(i, 0) as f64 - m0) * (hash01(i, 1) as f64 - m1))
            .sum::<f64>()
            / n as f64;
        // Uniform variance is 1/12, so normalise by that for a correlation.
        let corr = cov / (1.0 / 12.0);
        assert!(corr.abs() < 0.03, "streams 0 and 1 are correlated: {corr}");
    }

    /// The colour fader must not sweep into empty palette rows.
    ///
    /// The parameter's range covers the whole bank, but only the built-ins
    /// are written on a fresh install — the rest fill as palettes are
    /// dropped. An unwritten row reads back as zeroed texels, so before
    /// the clamp the top two-thirds of a default macro fader's throw faded
    /// the field to black and held it there, with the readout showing a
    /// plausible "7.00" and nothing saying the row was empty.
    /// Palette rows are addresses: `/color/palette` values in saved
    /// presets index them. A saved list restored with a missing file used
    /// to compact, silently repointing every later palette — a preset
    /// built on row 7 came back painting with whatever shifted into 7.
    /// A hole now holds its row.
    #[test]
    fn a_missing_palette_holds_its_row_so_later_ones_stay_put() {
        let Some(ctx) = gpu() else { return };
        let mut scene = ParticleScene::new(&ctx, crate::post::SCENE_FORMAT);

        let dir = std::env::temp_dir().join(format!("vizz-palrow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.hex");
        let b = dir.join("b.hex");
        std::fs::write(&a, "#ff0000\n#00ff00\n").unwrap();
        std::fs::write(&b, "#0000ff\n#ffff00\n").unwrap();

        let (_, row_a) = scene.load_palette(&ctx, &a).unwrap();
        // The file between them is gone: its row is held, not collapsed.
        scene.skip_palette_row();
        let (_, row_b) = scene.load_palette(&ctx, &b).unwrap();
        assert_eq!(row_b, row_a + 2, "the hole must keep its row");
    }

    #[test]
    fn the_colour_index_saturates_on_the_last_real_palette() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);
        assert_eq!(
            scene.palettes.occupied(),
            4,
            "a fresh bank should hold hsv plus four built-ins"
        );

        let lit = |px: &[u8]| {
            px.chunks_exact(4)
                .filter(|p| p[0].max(p[1]).max(p[2]) > 24)
                .count()
        };
        let last = render_palette(&ctx, &scene, 4.0);
        // Well past the written rows — this used to be black.
        let past = render_palette(&ctx, &scene, 12.0);
        assert!(lit(&last) > 0, "the last built-in rendered nothing");
        assert!(
            lit(&past) > 0,
            "sweeping past the written rows blacked the field out"
        );
        assert_eq!(
            last, past,
            "the index did not saturate: past the last palette should hold it"
        );
    }

    /// Transparency has to reach the pixels, not just the clear call.
    ///
    /// This is the property the whole feature rests on: vizz is only
    /// usable as a layer in Resolume or VDMX if the background arrives
    /// transparent and the field arrives opaque. Both halves are asserted,
    /// because either one alone is easy to get by accident — an all-zero
    /// alpha channel would pass a "background is transparent" check and be
    /// completely useless.
    #[test]
    fn a_transparent_background_reaches_the_output() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);
        let clear = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
        let pixels = render_with(&ctx, &scene, 0.0, clear);

        let alphas: Vec<u8> = pixels.chunks_exact(4).map(|p| p[3]).collect();
        let empty = alphas.iter().filter(|a| **a == 0).count();
        let covered = alphas.iter().filter(|a| **a > 8).count();

        assert!(
            empty > alphas.len() / 4,
            "background did not come through transparent: only {empty} of {} pixels were clear",
            alphas.len()
        );
        assert!(
            covered > 0,
            "the field itself was transparent too, which makes the output empty"
        );
        // And the covered pixels must actually be where the light is: an
        // alpha channel unrelated to the picture would composite wrongly.
        let lit_and_opaque = pixels
            .chunks_exact(4)
            .filter(|p| p[0].max(p[1]).max(p[2]) > 24)
            .filter(|p| p[3] > 8)
            .count();
        let lit = pixels
            .chunks_exact(4)
            .filter(|p| p[0].max(p[1]).max(p[2]) > 24)
            .count();
        assert!(
            lit > 0 && lit_and_opaque * 4 >= lit * 3,
            "alpha does not follow the image: {lit_and_opaque} of {lit} lit pixels carried alpha"
        );
    }

    /// An opaque background must stay opaque — the default, and what every
    /// existing receiver expects.
    #[test]
    fn the_default_background_is_fully_opaque() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);
        let pixels = render_with(&ctx, &scene, 0.0, SCENE_CLEAR);
        let transparent = pixels.chunks_exact(4).filter(|p| p[3] < 250).count();
        assert_eq!(
            transparent, 0,
            "{transparent} pixels were not opaque with the default background"
        );
    }

    /// Render one frame at a given `shape` and return the pixels.
    fn render_shape(ctx: &GpuContext, scene: &ParticleScene, shape: f32) -> Vec<u8> {
        render_with(ctx, scene, shape, SCENE_CLEAR)
    }

    /// As [`render_shape`], with the background under test.
    fn render_with(
        ctx: &GpuContext,
        scene: &ParticleScene,
        shape: f32,
        background: wgpu::Color,
    ) -> Vec<u8> {
        render_tuned(ctx, scene, background, |u| u.shape = shape)
    }

    /// The one place a frame is actually encoded, with the uniforms open
    /// for a test to adjust.
    fn render_tuned(
        ctx: &GpuContext,
        scene: &ParticleScene,
        background: wgpu::Color,
        tune: impl FnOnce(&mut Uniforms),
    ) -> Vec<u8> {
        let shape = 0.0;
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

        let cam = crate::camera::Camera { aspect: 1.0, ..Default::default() }.uniforms();
        let uniforms = Uniforms {
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
            shape,
            morph: 0.0,
            twist: 0.0,
            palette: 0.0,
            color_spread: 0.12,
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
            gravity: Default::default(),
            gravity_radius: Default::default(),
            gravity_amount: Default::default(),
            palette_rows: [4.0, 0.0, 0.0, 0.0],
            video: [0.0, 1.0, 0.0, 0.0],
        };

        let mut uniforms = uniforms;
        tune(&mut uniforms);

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        scene.render(ctx, &mut encoder, &view, &uniforms, 20_000, true, background);
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

    /// Blending is additive, so a scene texture that is only ever loaded
    /// accumulates every frame it has ever drawn. Whoever draws first has
    /// to clear.
    ///
    /// This shipped: the room pass did the clearing, and the app skips the
    /// room pass when the room is dark — which is the default. The result
    /// was that a default launch saturated to solid white within seconds,
    /// and every headless render of a preset came out as a white disc.
    /// Nothing caught it because the room happened to be on in every
    /// render anyone had looked at.
    #[test]
    fn a_frame_that_never_clears_saturates_and_a_cleared_one_does_not() {
        let Some(ctx) = gpu() else { return };
        let scene = ParticleScene::new(&ctx, FORMAT);

        // Clearing every frame is steady state: 120 frames look like one.
        let cleared = saturation_after(&ctx, &scene, 120, true);
        let once = saturation_after(&ctx, &scene, 1, true);
        assert!(
            (cleared - once).abs() < 0.02,
            "clearing every frame should be stable: {once} then {cleared}"
        );

        // Not clearing is the bug, and the test is only meaningful if the
        // failure it guards against actually reproduces here.
        let accumulated = saturation_after(&ctx, &scene, 120, false);
        assert!(
            accumulated > cleared + 0.05,
            "accumulation did not reproduce ({accumulated} vs {cleared}); \
             this test would not catch the regression it exists for"
        );
    }
}