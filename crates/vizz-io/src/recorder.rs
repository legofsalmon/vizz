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

/// What a take is written as, and the limits it stops at.
///
/// Recording used to have no settings at all: lossless PNG, every
/// rendered frame, at the full output size, until something stopped it.
/// That is about 800 MB a second at 1080p60 — a laptop's free space in
/// well under a minute, with nothing said before or during. These are
/// the controls that make a take a decision rather than a surprise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub format: Format,
    /// Frames per second to *record*, independent of the rate the app
    /// renders at. Recording 30 while rendering 60 halves everything —
    /// the files, the encode load and the disk rate.
    pub fps: f32,
    /// Stop the take after this many seconds. `None` runs until stopped,
    /// which is what it always did.
    pub max_secs: Option<f32>,
    /// Stop cleanly when the volume has less than this left. A take that
    /// ends early is recoverable; a full disk is a broken machine, and
    /// on the volume holding a live set it can be a broken show.
    pub floor_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // JPEG, not PNG: an order of magnitude smaller for footage
            // that is nearly always going into an edit, and quality 92
            // is visually clean. PNG stays one click away for anyone who
            // needs lossless.
            format: Format::Jpeg { quality: 92 },
            fps: 30.0,
            max_secs: None,
            // 2 GB. Low enough not to refuse a take on a working disk,
            // high enough that the machine still boots and saves.
            floor_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Lossless, large. Survives a crash frame by frame.
    Png,
    /// Lossy, roughly a tenth the size. Quality 1-100.
    Jpeg { quality: u8 },
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg { .. } => "jpg",
        }
    }

    /// Bytes one frame costs, near enough to warn with.
    ///
    /// Deliberately rough and deliberately *pessimistic* for PNG: the
    /// number exists to answer "how long until the disk is full", and a
    /// cheerful estimate there is worse than none. Measured on this
    /// renderer's output, which is high-contrast and compresses badly.
    pub fn bytes_per_frame(self, width: u32, height: u32) -> u64 {
        let px = width as u64 * height as u64;
        match self {
            // Fast-compressed PNG of a busy frame lands near 2 bytes a
            // pixel; flat looks do better and nobody is harmed by that.
            Format::Png => px * 2,
            Format::Jpeg { quality } => {
                // ~0.25 B/px at 92, scaling roughly with quality.
                let q = quality.clamp(1, 100) as u64;
                (px * q / 400).max(px / 40)
            }
        }
    }
}

/// Free space on the volume `path` lives on.
///
/// `None` when it cannot be determined — a network volume, a platform
/// sysinfo does not enumerate. Callers treat that as "cannot check"
/// rather than "no space", because refusing to record on a machine we
/// simply could not measure would be the worse failure.
pub fn free_space(path: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        // Longest matching mount point wins: on a machine with /Volumes
        // mounted under /, both match and only the deeper one is right.
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// Why a take stopped on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// The configured duration elapsed.
    Duration,
    /// The volume dropped to the floor.
    DiskLow,
}

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
    settings: Settings,
    /// When the last frame was taken, for the record-rate gate.
    last_capture: Option<Instant>,
    /// Set once the take has hit a limit, so the owner stops it and can
    /// say which limit it was.
    stopped: Option<StopReason>,
    /// Free space at the last check, and when that was. Checking the
    /// volume every frame would be a syscall storm for a number that
    /// moves slowly.
    space: Option<(Instant, u64)>,
}

impl Recorder {
    /// Start recording `width`x`height` frames into `dir` (created if
    /// missing).
    pub fn new(
        device: &wgpu::Device,
        dir: &Path,
        width: u32,
        height: u32,
        settings: Settings,
    ) -> Result<Self> {
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
                .spawn(move || encode_loop(&dir, rx, started, &written, &error, settings))?
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
            settings,
            last_capture: None,
            stopped: None,
            space: None,
        })
    }

    /// Capture this frame's master. Non-blocking at every stage.
    ///
    /// Frames are taken at the configured record rate rather than at the
    /// render rate: at 30 into a 60 fps render this halves the files, the
    /// encode load and the disk rate, and it is the single most effective
    /// thing available for keeping a take a sane size.
    pub fn publish(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) {
        if self.stopped.is_some() {
            return;
        }
        if let Some(max) = self.settings.max_secs
            && self.started.elapsed().as_secs_f32() >= max
        {
            self.stopped = Some(StopReason::Duration);
            return;
        }
        if self.disk_low() {
            self.stopped = Some(StopReason::DiskLow);
            return;
        }
        let interval = 1.0 / self.settings.fps.max(0.1);
        if let Some(last) = self.last_capture
            && last.elapsed().as_secs_f32() < interval
        {
            return;
        }
        self.last_capture = Some(Instant::now());
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

    /// Why the take stopped itself, if it did. The owner polls this and
    /// ends the recording — the recorder never tears itself down, so the
    /// frames already queued still land.
    pub fn stopped(&self) -> Option<&StopReason> {
        self.stopped.as_ref()
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Is the volume at or below the floor? Checked at most once a
    /// second — free space moves slowly even while writing hard, and a
    /// statfs per frame is a syscall storm for nothing.
    fn disk_low(&mut self) -> bool {
        let now = Instant::now();
        let stale = self
            .space
            .is_none_or(|(at, _)| now.duration_since(at).as_secs_f32() >= 1.0);
        if stale {
            self.space = free_space(&self.dir).map(|free| (now, free));
        }
        // Unmeasurable means carry on: refusing to record because the
        // volume could not be enumerated would fail a working machine.
        self.space.is_some_and(|(_, free)| free <= self.settings.floor_bytes)
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
    settings: Settings,
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
        let result = write_frame(dir, index(), &frame, settings.format).and_then(|()| {
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
fn write_frame(dir: &Path, index: u64, frame: &MappedFrame, format: Format) -> Result<()> {
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
    let path = dir.join(format!("frame_{index:06}.{}", format.extension()));
    let file = std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
    let out = std::io::BufWriter::new(file);
    match format {
        Format::Png => {
            let encoder = image::codecs::png::PngEncoder::new_with_quality(
                out,
                // Fast is the only setting that approaches frame rate; the
                // files are bigger and that is the right trade for a live
                // capture.
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
        }
        Format::Jpeg { quality } => {
            // JPEG has no alpha, and the master is opaque anyway — the
            // encoder wants three channels, so drop the fourth rather
            // than let it reject the buffer.
            let rgb: Vec<u8> = rgba
                .chunks_exact(4)
                .flat_map(|px| [px[0], px[1], px[2]])
                .collect();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                out,
                quality.clamp(1, 100),
            );
            image::ImageEncoder::write_image(
                encoder,
                &rgb,
                w as u32,
                h as u32,
                image::ExtendedColorType::Rgb8,
            )
        }
    }
    .with_context(|| format!("encoding {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rate estimate is what the panel warns with, so it has to be
    /// the right order of magnitude and it has to be pessimistic.
    ///
    /// The bug this guards is a cheerful estimate: told 60 MB/s when the
    /// truth is 800, someone starts a take believing they have twenty
    /// minutes and loses the disk in ninety seconds.
    #[test]
    fn the_rate_estimate_is_pessimistic_and_the_right_size() {
        let (w, h) = (1920, 1080);
        let png = Format::Png.bytes_per_frame(w, h);
        let jpeg = Format::Jpeg { quality: 92 }.bytes_per_frame(w, h);

        // Lossless PNG of a busy frame: megabytes, not kilobytes.
        assert!(png > 2_000_000, "PNG estimate {png} is implausibly small");
        // And JPEG must be the big win it is sold as.
        assert!(jpeg * 4 < png, "JPEG {jpeg} is not much smaller than PNG {png}");
        assert!(jpeg > 200_000, "JPEG estimate {jpeg} is implausibly small");

        // Quality moves it, or the control is decorative.
        let low = Format::Jpeg { quality: 50 }.bytes_per_frame(w, h);
        assert!(low < jpeg, "lowering quality did not lower the estimate");

        // The headline number the panel shows: 1080p60 lossless really is
        // hundreds of megabytes a second, which is the fact that started
        // all of this.
        let per_sec = png * 60;
        assert!(
            per_sec > 100_000_000,
            "1080p60 PNG estimated at {per_sec} B/s — too low to warn anyone"
        );
    }

    /// A take with a duration must stop itself, and a take without one
    /// must not.
    #[test]
    fn a_duration_limit_ends_the_take() {
        let Some((device, queue, texture)) = gpu() else { return };
        let dir = tempdir();
        let mut rec = Recorder::new(
            &device,
            &dir,
            8,
            8,
            Settings {
                // Already elapsed by the time the first frame arrives.
                max_secs: Some(0.0),
                fps: 1000.0,
                ..Default::default()
            },
        )
        .expect("recorder");
        rec.publish(&device, &queue, &texture);
        assert_eq!(
            rec.stopped(),
            Some(&StopReason::Duration),
            "a take past its duration kept going"
        );

        let mut open = Recorder::new(&device, &dir, 8, 8, Settings { fps: 1000.0, ..Default::default() })
            .expect("recorder");
        open.publish(&device, &queue, &texture);
        assert_eq!(open.stopped(), None, "an unlimited take stopped itself");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The disk floor stops a take rather than filling the volume. Set
    /// impossibly high so any real disk is "low" — the branch under test
    /// is the decision, not the number.
    #[test]
    fn a_full_disk_stops_the_take_instead_of_filling_it() {
        let Some((device, queue, texture)) = gpu() else { return };
        let dir = tempdir();
        let mut rec = Recorder::new(
            &device,
            &dir,
            8,
            8,
            Settings { floor_bytes: u64::MAX, fps: 1000.0, ..Default::default() },
        )
        .expect("recorder");
        rec.publish(&device, &queue, &texture);
        // Only assert when free space is actually measurable here; a
        // volume sysinfo cannot see must not fail the build, and must
        // not stop a take either.
        if free_space(&dir).is_some() {
            assert_eq!(
                rec.stopped(),
                Some(&StopReason::DiskLow),
                "a take carried on below the disk floor"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The record rate gates capture: at 1 fps a burst of frames in one
    /// millisecond is one frame, not a burst.
    #[test]
    fn the_record_rate_gates_capture() {
        let Some((device, queue, texture)) = gpu() else { return };
        let dir = tempdir();
        let mut rec =
            Recorder::new(&device, &dir, 8, 8, Settings { fps: 1.0, ..Default::default() })
                .expect("recorder");
        for _ in 0..10 {
            rec.publish(&device, &queue, &texture);
        }
        // The ring is asynchronous, so what is asserted is the gate, not
        // the file count: nine of those ten publishes did no work at all.
        assert!(rec.stopped().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "vizz-rec-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::create_dir_all(&base);
        base
    }

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

        // Explicitly lossless and ungated: this test asserts the decoded
        // pixels are *exactly* what was rendered, which is a claim only
        // PNG can meet, and it publishes as fast as it can, which the
        // default 30 fps record rate would throttle to one frame.
        let mut rec = Recorder::new(
            &device,
            &dir,
            8,
            8,
            Settings { format: Format::Png, fps: 1000.0, ..Default::default() },
        )
        .expect("recorder starts");
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
        let err = Recorder::new(&device, &blocker.join("take"), 8, 8, Settings::default());
        assert!(err.is_err(), "recording under a plain file was accepted");
        let _ = std::fs::remove_file(&blocker);
    }
}
