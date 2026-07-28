//! The first generator: a fully procedural particle field.
//!
//! All per-particle state is derived in the vertex shader from the vertex
//! index, so per-frame CPU cost is one 32-byte uniform upload and one draw
//! call regardless of particle count.

use crate::{GpuContext, attractor::Attractors};

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
}

pub struct ParticleScene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&palettes.view),
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
    pub const LOADABLE: usize = crate::attractor::SLOTS - (crate::attractor::SLOT_AIZAWA + 1);

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

    /// The slot a live stream writes into: the last loadable one, so a
    /// `--cloud` file and a live feed can be held at once and morphed
    /// between with `/cloud/morph`.
    pub const LIVE_SLOT: usize = crate::attractor::SLOTS - 1;

    /// Encode one frame into `target`. `count` is the number of particles.
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
        ctx.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));

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
            gravity: Default::default(),
            gravity_radius: Default::default(),
            gravity_amount: Default::default(),
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
            gravity: Default::default(),
            gravity_radius: Default::default(),
            gravity_amount: Default::default(),
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