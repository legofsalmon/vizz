//! A wgpu 30 paint backend for egui.
//!
//! The published `egui-wgpu` crate still targets wgpu 29, and two wgpu
//! versions cannot share a device — so rather than downgrade vizz's
//! verified renderer, this implements the (small, well-defined) backend:
//! upload egui's texture deltas, concatenate its meshes into one
//! vertex/index buffer, and draw each clipped primitive with a scissor.

use std::collections::HashMap;

use anyhow::Result;

/// Matches `epaint::Vertex`: two floats of position, two of UV, four
/// bytes of premultiplied sRGB colour.
const VERTEX_SIZE: u64 = 20;

pub struct EguiRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: HashMap<egui::TextureId, (wgpu::Texture, wgpu::BindGroup)>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    vertex_capacity: u64,
    index_capacity: u64,
    srgb_target: bool,
}

impl EguiRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("egui"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui-uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-uniform-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // The fragment stage reads srgb_target from the same block.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("egui-uniform-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-texture-bgl"),
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("egui-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("egui-pl"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("egui-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_SIZE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Unorm8x4,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    // egui colours are premultiplied.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
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

        let (vertex_capacity, index_capacity) = (1024 * VERTEX_SIZE, 1024 * 4);
        Self {
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            texture_layout,
            sampler,
            textures: HashMap::new(),
            vertices: alloc(device, "egui-vertices", vertex_capacity, wgpu::BufferUsages::VERTEX),
            indices: alloc(device, "egui-indices", index_capacity, wgpu::BufferUsages::INDEX),
            vertex_capacity,
            index_capacity,
            srgb_target: target_format.is_srgb(),
        }
    }

    /// Apply egui's texture deltas. Must run before `render` for the frame.
    pub fn update_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        delta: &egui::TexturesDelta,
    ) {
        for (id, image_delta) in &delta.set {
            self.set_texture(device, queue, *id, image_delta);
        }
        // Freeing after the sets, as egui may free and re-set in one frame.
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    fn set_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: egui::TextureId,
        delta: &egui::epaint::ImageDelta,
    ) {
        // egui 0.35 delivers every atlas (fonts included) as premultiplied
        // sRGBA, which is exactly the atlas format.
        let egui::ImageData::Color(image) = &delta.image;
        let size = image.size;
        let pixels: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
        let (width, height) = (size[0] as u32, size[1] as u32);

        // A delta with `pos` patches part of an existing atlas; without it,
        // the texture is (re)created at this size.
        let whole = delta.pos.is_none();
        if whole || !self.textures.contains_key(&id) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("egui-atlas"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("egui-atlas-bg"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.textures.insert(id, (texture, bind_group));
        }

        let Some((texture, _)) = self.textures.get(&id) else { return };
        let [x, y] = delta.pos.unwrap_or([0, 0]);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: x as u32, y: y as u32, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }

    /// Draw `primitives` over `target`. `size_px` is the framebuffer size;
    /// `pixels_per_point` converts egui's points into those pixels.
    ///
    /// Wide by the same reasoning as the particle pass: these are the
    /// wgpu handles and the frame's own measurements, held separately by
    /// the caller and bundled nowhere else.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        primitives: &[egui::ClippedPrimitive],
        size_px: [u32; 2],
        pixels_per_point: f32,
    ) -> Result<()> {
        let screen_points =
            [size_px[0] as f32 / pixels_per_point, size_px[1] as f32 / pixels_per_point];
        let flags = [u32::from(self.srgb_target), 0u32];
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&screen_points));
        queue.write_buffer(&self.uniform_buffer, 8, bytemuck::cast_slice(&flags));

        // Concatenate every mesh once, remembering where each one landed.
        let mut all_vertices: Vec<u8> = Vec::new();
        let mut all_indices: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
        for prim in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &prim.primitive else {
                // Callback primitives are a user-shader escape hatch vizz
                // does not use; skipping keeps them from corrupting state.
                log::debug!("skipping unsupported egui callback primitive");
                continue;
            };
            if mesh.indices.is_empty() || !self.textures.contains_key(&mesh.texture_id) {
                continue;
            }
            let base_vertex = (all_vertices.len() as u64 / VERTEX_SIZE) as i32;
            let index_start = all_indices.len() as u32;
            all_vertices.extend_from_slice(bytemuck::cast_slice(&mesh.vertices));
            all_indices.extend_from_slice(&mesh.indices);
            draws.push((
                mesh.texture_id,
                prim.clip_rect,
                index_start,
                mesh.indices.len() as u32,
                base_vertex,
            ));
        }
        if draws.is_empty() {
            return Ok(());
        }

        self.ensure_capacity(device, all_vertices.len() as u64, (all_indices.len() * 4) as u64);
        queue.write_buffer(&self.vertices, 0, &all_vertices);
        queue.write_buffer(&self.indices, 0, bytemuck::cast_slice(&all_indices));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                // Load: the UI composites over the already-drawn preview.
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);

        for (texture_id, clip, index_start, index_count, base_vertex) in draws {
            let Some((_, bind_group)) = self.textures.get(&texture_id) else { continue };
            // Clip rect is in points; scissor is in pixels and must stay
            // inside the target or the pass is invalid.
            let x = (clip.min.x * pixels_per_point).round().max(0.0) as u32;
            let y = (clip.min.y * pixels_per_point).round().max(0.0) as u32;
            let max_x = (clip.max.x * pixels_per_point).round().max(0.0) as u32;
            let max_y = (clip.max.y * pixels_per_point).round().max(0.0) as u32;
            let x = x.min(size_px[0]);
            let y = y.min(size_px[1]);
            let w = max_x.min(size_px[0]).saturating_sub(x);
            let h = max_y.min(size_px[1]).saturating_sub(y);
            if w == 0 || h == 0 {
                continue; // fully clipped
            }
            pass.set_scissor_rect(x, y, w, h);
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw_indexed(index_start..index_start + index_count, base_vertex, 0..1);
        }
        Ok(())
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, vertex_bytes: u64, index_bytes: u64) {
        if vertex_bytes > self.vertex_capacity {
            self.vertex_capacity = (vertex_bytes * 2).next_power_of_two();
            self.vertices = alloc(
                device,
                "egui-vertices",
                self.vertex_capacity,
                wgpu::BufferUsages::VERTEX,
            );
        }
        if index_bytes > self.index_capacity {
            self.index_capacity = (index_bytes * 2).next_power_of_two();
            self.indices =
                alloc(device, "egui-indices", self.index_capacity, wgpu::BufferUsages::INDEX);
        }
    }
}

fn alloc(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 512;
    const H: u32 = 384;
    // Same family as the real preview swapchain, so the shader's
    // gamma handling is exercised the way it will be in the app.
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }));
        let adapter = match adapter {
            Ok(a) => a,
            Err(_) if std::env::var_os("VIZZ_REQUIRE_GPU").is_some() => {
                panic!("VIZZ_REQUIRE_GPU is set but no GPU adapter was found")
            }
            Err(_) => {
                eprintln!("no GPU adapter available; skipping GPU test");
                return None;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("egui-render-test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .ok()?;
        Some((device, queue))
    }

    /// Paint a real egui pass through the backend and read the pixels
    /// back. This is the end-to-end check that the hand-written wgpu 30
    /// backend — pipeline, vertex layout, atlas upload, scissors — draws
    /// anything at all, which no amount of type-checking proves.
    #[test]
    fn backend_paints_egui_output_to_a_texture() {
        let Some((device, queue)) = gpu() else { return };

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("egui-target"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        // Clear to black first; anything non-black afterwards is UI.
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
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
        queue.submit([enc.finish()]);

        // Two passes: egui measures a fresh Window on the first.
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(W as f32, H as f32),
            )),
            ..Default::default()
        };
        // Apply each pass's texture delta as it arrives, exactly as the
        // app does per frame: the font atlas is delivered on the FIRST
        // pass, so keeping only the last output loses it entirely.
        let mut renderer = EguiRenderer::new(&device, FORMAT);
        let mut full = None;
        for _ in 0..2 {
            ctx.begin_pass(input.clone());
            egui::Window::new("vizz").show(&ctx, |ui| {
                ui.heading("60 fps");
                ui.label("particles/count");
                let mut v = 0.5f32;
                ui.add(egui::Slider::new(&mut v, 0.0..=1.0));
            });
            let out = ctx.end_pass();
            renderer.update_textures(&device, &queue, &out.textures_delta);
            full = Some(out);
        }
        let full = full.unwrap();

        let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
        assert!(!primitives.is_empty(), "egui produced nothing to draw");

        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        renderer
            .render(&device, &queue, &mut enc, &view, &primitives, [W, H], full.pixels_per_point)
            .expect("render failed");
        queue.submit([enc.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

        let lit = count_non_black(&device, &queue, &target);
        // The panel covers a real area; a handful of stray pixels would
        // mean the pipeline ran but drew essentially nothing.
        assert!(
            lit > 2000,
            "expected the panel to cover a meaningful area, only {lit} pixels were painted"
        );
    }

    fn count_non_black(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture) -> usize {
        const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = (W * 4).div_ceil(ALIGN) * ALIGN;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui-readback"),
            size: (padded * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range().unwrap();
        let count = (0..H as usize)
            .flat_map(|row| {
                let start = row * padded as usize;
                data[start..start + (W * 4) as usize].chunks_exact(4)
            })
            .filter(|px| px[0] > 8 || px[1] > 8 || px[2] > 8)
            .count();
        drop(data);
        buffer.unmap();
        count
    }
}
