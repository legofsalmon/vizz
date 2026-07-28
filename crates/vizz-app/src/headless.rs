//! Headless mode: render offscreen at a fixed timestep, then emit a JSON
//! health report. This is the benchmark/CI entry point — and on macOS it
//! doubles as a windowless Syphon source, since outputs publish here too.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use serde::Serialize;
use vizz_health::HealthSnapshot;
use vizz_render::{GpuContext, output::OutputTarget, particles::ParticleScene, post::PostChain};

use crate::engine::FrameEngine;
use crate::outputs::{self, OutputOpts};
use crate::params::AppParams;

#[derive(Serialize)]
struct BenchReport {
    width: u32,
    height: u32,
    frames_requested: u32,
    adapter: String,
    backend: String,
    wall_time_s: f64,
    health: HealthSnapshot,
}

pub struct HeadlessOpts {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub dump: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub outputs: OutputOpts,
    /// Substring match against an input device name; None picks the default.
    pub audio_device: Option<String>,
    /// Point clouds to load into the loadable slots, in order.
    pub clouds: Vec<PathBuf>,
}

pub fn run(params: Arc<AppParams>, opts: HeadlessOpts) -> Result<()> {
    let ctx = pollster::block_on(GpuContext::new(None))?;
    let room = vizz_render::room::Room::new(&ctx, vizz_render::post::SCENE_FORMAT);
    let mut post = PostChain::new(&ctx, opts.width, opts.height, vizz_render::output::OUTPUT_FORMAT);
    let mut scene = ParticleScene::new(&ctx, vizz_render::post::SCENE_FORMAT);
    scene.load_clouds(&ctx, &opts.clouds);
    let mut engine = FrameEngine::new(params, vizz_audio::AudioEngine::start(opts.audio_device.as_deref()));
    let output = OutputTarget::new(&ctx.device, opts.width, opts.height);
    let mut senders = outputs::build_senders(&ctx.device, &opts.outputs);
    let fixed_dt = Duration::from_nanos(16_666_667);

    log::info!(
        "headless: {}x{} for {} frames",
        opts.width,
        opts.height,
        opts.frames
    );
    let run_start = Instant::now();
    for _ in 0..opts.frames {
        let frame_start = Instant::now();
        let inputs = engine.begin_frame(output.aspect(), Some(fixed_dt));
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        // Room first: it clears, and the particles then add on top.
        if inputs.room_visible {
            room.render(&ctx, &mut encoder, &post.scene_view, &inputs.room);
        }
        scene.render(&ctx, &mut encoder, &post.scene_view, &inputs.uniforms, inputs.count, !inputs.room_visible);
        post.render(&ctx, &mut encoder, &output.view, &inputs.post);
        ctx.queue.submit([encoder.finish()]);
        outputs::publish_all(&mut senders, &ctx.device, &ctx.queue, &output.texture);
        // Headless has no vsync backpressure: wait for the GPU so frame
        // times measure real work, not queue depth.
        ctx.device
            .poll(wgpu::PollType::wait_indefinitely())
            .context("device poll failed")?;
        if let Some(snap) = engine.end_frame(frame_start.elapsed()) {
            log::info!("{}", snap.log_line());
        }
    }
    let wall = run_start.elapsed();

    let health = engine.health.snapshot();
    log::info!("final: {}", health.log_line());

    if let Some(path) = &opts.report {
        let info = ctx.adapter.get_info();
        let report = BenchReport {
            width: opts.width,
            height: opts.height,
            frames_requested: opts.frames,
            adapter: info.name.clone(),
            backend: format!("{:?}", info.backend),
            wall_time_s: wall.as_secs_f64(),
            health,
        };
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)
            .with_context(|| format!("writing report to {}", path.display()))?;
        log::info!("wrote benchmark report to {}", path.display());
    }

    if let Some(path) = &opts.dump {
        dump_png(&ctx, &output.texture, opts.width, opts.height, path)?;
        log::info!("wrote last frame to {}", path.display());
    }
    Ok(())
}

/// Synchronous readback of the final frame. Fine here — this runs once,
/// after the loop. The realtime NDI path will use an async staging ring
/// instead; never copy this pattern onto the render loop.
fn dump_png(ctx: &GpuContext, texture: &wgpu::Texture, width: u32, height: u32, path: &Path) -> Result<()> {
    const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let unpadded = width * 4;
    let padded = unpadded.div_ceil(ALIGN) * ALIGN;

    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
    rx.recv().context("map_async callback dropped")??;

    let data = slice.get_mapped_range().context("mapped range invalid")?;
    let mut pixels = Vec::with_capacity((unpadded * height) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        pixels.extend_from_slice(&data[start..start + unpadded as usize]);
    }
    drop(data);
    buffer.unmap();

    // Master texture is BGRA; PNG wants RGBA.
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    image::RgbaImage::from_raw(width, height, pixels)
        .context("readback size mismatch")?
        .save(path)?;
    Ok(())
}
