//! PNG-sequence recorder: the master output to disk, without ever making
//! the render loop wait.
//!
//! The same shape as the NDI sender on purpose: an async [`ReadbackRing`]
//! feeds a bounded channel feeding a worker thread, and every stage drops
//! rather than blocks — a slow disk costs frames on the recording and
//! nothing else. The drops are counted and reported, so a capture that
//! could not keep up says so instead of pretending.
//!
//! PNG rather than a video container because it needs no codec, survives
//! a crash mid-take (every finished frame is a finished file), and
//! assembles with one ffmpeg line. Frames arrive at whatever rate the app
//! actually rendered, so alongside the images the worker writes
//! `frames.csv` — `index,elapsed_ms` per frame — which is what lets a
//! variable-rate take be assembled honestly later.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::{Context as _, Result};

use crate::readback::{MappedFrame, ReadbackRing};

pub struct Recorder {
    dir: PathBuf,
    ring: ReadbackRing,
    tx: Option<SyncSender<MappedFrame>>,
    thread: Option<JoinHandle<()>>,
    /// Frames dropped because the encoder was still busy.
    enc_dropped: Arc<AtomicU64>,
    written: Arc<AtomicU64>,
    /// The worker's terminal complaint (disk full, unwritable dir). Once
    /// set the worker has stopped; the owner should stop recording and
    /// say why.
    error: Arc<Mutex<Option<String>>>,
    started: Instant,
}

impl Recorder {
    /// Start recording `width`x`height` frames into `dir` (created if
    /// missing).
    pub fn new(device: &wgpu::Device, dir: &Path, width: u32, height: u32) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        // Prove the directory is writable *now*, so an unwritable target
        // is a refusal at the button rather than a notice a second later.
        let probe = dir.join(".vizz-write-probe");
        std::fs::write(&probe, b"").with_context(|| format!("writing into {}", dir.display()))?;
        let _ = std::fs::remove_file(&probe);

        // One deeper than the NDI ring: PNG encoding is slower than a
        // network send, and the extra slot smooths encode-time spikes.
        let ring = ReadbackRing::new(device, width, height, 4)?;
        // Depth 2 so one long encode does not immediately cost a frame.
        let (tx, rx) = sync_channel::<MappedFrame>(2);
        let enc_dropped = Arc::new(AtomicU64::new(0));
        let written = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        let started = Instant::now();

        let thread = {
            let dir = dir.to_path_buf();
            let written = Arc::clone(&written);
            let error = Arc::clone(&error);
            std::thread::Builder::new()
                .name("vizz-record".into())
                .spawn(move || encode_loop(&dir, rx, started, &written, &error))?
        };

        Ok(Self {
            dir: dir.to_path_buf(),
            ring,
            tx: Some(tx),
            thread: Some(thread),
            enc_dropped,
            written,
            error,
            started,
        })
    }

    /// Capture this frame's master. Non-blocking at every stage.
    pub fn publish(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) {
        self.ring.capture(device, queue, texture);
        if let Some(frame) = self.ring.take_ready()
            && let Some(tx) = &self.tx
        {
            match tx.try_send(frame) {
                Ok(()) => {}
                // Encoder behind: drop the frame, count it. Never wait.
                Err(TrySendError::Full(_)) => {
                    self.enc_dropped.fetch_add(1, Ordering::Relaxed);
                }
                // Worker dead — its error slot says why; the owner polls.
                Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Frames written so far and frames dropped (ring + encoder queue).
    pub fn progress(&self) -> (u64, u64) {
        (
            self.written.load(Ordering::Relaxed),
            self.ring.dropped() + self.enc_dropped.load(Ordering::Relaxed),
        )
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }

    /// The worker's terminal error, if it has died. Taking it clears it.
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut e| e.take())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Closing the channel wakes a worker parked in recv(); it drains
        // what it holds and exits, so the last frames still land.
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Encode until the channel closes or the disk refuses.
fn encode_loop(
    dir: &Path,
    rx: Receiver<MappedFrame>,
    started: Instant,
    written: &AtomicU64,
    error: &Mutex<Option<String>>,
) {
    let index = || written.load(Ordering::Relaxed) + 1;
    let mut csv = match std::fs::File::create(dir.join("frames.csv")) {
        Ok(f) => std::io::BufWriter::new(f),
        Err(e) => {
            if let Ok(mut slot) = error.lock() {
                *slot = Some(format!("creating frames.csv: {e}"));
            }
            return;
        }
    };
    let _ = writeln!(csv, "frame,elapsed_ms");

    while let Ok(frame) = rx.recv() {
        let elapsed_ms = started.elapsed().as_millis();
        let result = write_frame(dir, index(), &frame).and_then(|()| {
            writeln!(csv, "{},{elapsed_ms}", index()).context("appending frames.csv")?;
            Ok(())
        });
        match result {
            Ok(()) => {
                written.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                // Disk full or gone: one terminal complaint, then stop.
                // Retrying sixty times a second against a full disk is
                // the storm the MIDI-map save already taught us about.
                log::error!("recording stopped: {e:#}");
                if let Ok(mut slot) = error.lock() {
                    *slot = Some(format!("{e:#}"));
                }
                return;
            }
        }
    }
    let _ = csv.flush();
}

/// One frame to one PNG. The master is BGRA with padded rows; PNG wants
/// tight RGBA.
fn write_frame(dir: &Path, index: u64, frame: &MappedFrame) -> Result<()> {
    let (w, h, stride) = (frame.width as usize, frame.height as usize, frame.stride as usize);
    let rgba = frame.with_bytes(|bytes| {
        let mut out = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let line = &bytes[row * stride..row * stride + w * 4];
            for px in line.chunks_exact(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
        out
    })?;
    let path = dir.join(format!("frame_{index:06}.png"));
    let file = std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        std::io::BufWriter::new(file),
        // Fast is the only setting that approaches frame rate; the files
        // are bigger and that is the right trade for a live capture.
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );
    image::ImageEncoder::write_image(
        encoder,
        &rgba,
        w as u32,
        h as u32,
        image::ExtendedColorType::Rgba8,
    )
    .with_context(|| format!("encoding {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Headless GPU (llvmpipe in CI) plus an 8x8 BGRA texture cleared to
    /// a known colour. Mirrors the readback tests' skip/require policy.
    fn gpu() -> Option<(wgpu::Device, wgpu::Queue, wgpu::Texture)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .ok();
        let Some(adapter) = adapter else {
            if std::env::var_os("VIZZ_REQUIRE_GPU").is_some() {
                panic!("VIZZ_REQUIRE_GPU is set but no GPU adapter was found");
            }
            eprintln!("no GPU adapter available; skipping GPU test");
            return None;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("recorder-test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .ok()?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("recorder-test-target"),
            size: wgpu::Extent3d { width: 8, height: 8, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut enc = device.create_command_encoder(&Default::default());
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 200.0 / 255.0,
                        g: 32.0 / 255.0,
                        b: 64.0 / 255.0,
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
        queue.submit([enc.finish()]);
        Some((device, queue, texture))
    }

    /// The full path: render a known colour, record a few frames, decode
    /// the PNGs back and check pixels, numbering and the timing log.
    #[test]
    fn recorded_frames_decode_back_to_what_was_rendered() {
        let Some((device, queue, texture)) = gpu() else { return };
        let dir = std::env::temp_dir().join(format!("vizz-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut rec = Recorder::new(&device, &dir, 8, 8).expect("recorder starts");
        // More publishes than frames: the ring needs a few calls before
        // the first map completes.
        for _ in 0..20 {
            rec.publish(&device, &queue, &texture);
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (written, _dropped) = rec.progress();
        drop(rec); // joins the worker, flushing everything queued
        let _ = written;

        let mut frames: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".png"))
            .collect();
        frames.sort();
        assert!(!frames.is_empty(), "no frames were written");
        assert_eq!(frames[0], "frame_000001.png", "numbering is off: {frames:?}");

        let img = image::open(dir.join(&frames[0])).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (8, 8));
        // The texture was cleared to BGRA [64,32,200,255]; the decoded
        // RGBA must be the same colour with the channels back in order.
        assert_eq!(img.get_pixel(4, 4).0, [200, 32, 64, 255], "channel order is wrong");

        let csv = std::fs::read_to_string(dir.join("frames.csv")).unwrap();
        assert!(csv.starts_with("frame,elapsed_ms"), "csv header missing");
        assert_eq!(
            csv.lines().count() - 1,
            frames.len(),
            "csv rows do not match written frames"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An unwritable target is a refusal at the button, not a mystery
    /// notice a second into a take.
    #[test]
    fn an_unwritable_directory_is_refused_up_front() {
        let Some((device, _queue, _texture)) = gpu() else { return };
        // A path whose parent is a plain *file* cannot become a directory
        // on any platform — unlike /proc, which Windows happily creates
        // as a real, writable D:\proc.
        let blocker = std::env::temp_dir().join(format!("vizz-rec-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"").unwrap();
        let err = Recorder::new(&device, &blocker.join("take"), 8, 8);
        assert!(err.is_err(), "recording under a plain file was accepted");
        let _ = std::fs::remove_file(&blocker);
    }
}
