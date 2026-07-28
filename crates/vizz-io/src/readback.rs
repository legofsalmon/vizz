//! Async GPU→CPU readback ring.
//!
//! NDI needs CPU-side pixels, but the render thread must never wait for the
//! GPU. This ring is how those two facts coexist:
//!
//! 1. [`ReadbackRing::capture`] encodes a `copy_texture_to_buffer` into a
//!    free staging buffer, submits it, and calls `map_async` — then returns
//!    immediately. Nothing blocks.
//! 2. Some frames later the map callback fires and the slot becomes ready.
//! 3. [`ReadbackRing::take_ready`] hands the mapped slot out as a
//!    [`MappedFrame`], which a *consumer thread* reads. Dropping it unmaps
//!    the buffer and returns the slot to the pool.
//!
//! If every slot is busy — a slow consumer, or a GPU running behind — the
//! frame is **dropped for this output** and counted, never awaited. Losing
//! an NDI frame is survivable; missing vsync is not.
//!
//! Rows are padded to wgpu's 256-byte copy alignment. Rather than repacking
//! on the CPU, the padded stride is reported as [`MappedFrame::stride`] and
//! passed straight to consumers that accept a line stride (NDI does), so
//! the pixels are never touched between GPU and wire.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use anyhow::{Context as _, Result, bail};

const FREE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const INFLIGHT: u8 = 3;

struct Slot {
    buffer: Arc<wgpu::Buffer>,
    state: Arc<AtomicU8>,
}

/// A finished CPU-side frame. Read it with [`MappedFrame::with_bytes`];
/// dropping it releases the slot back to the ring.
pub struct MappedFrame {
    buffer: Arc<wgpu::Buffer>,
    state: Arc<AtomicU8>,
    pub width: u32,
    pub height: u32,
    /// Bytes per row *including* padding to the copy alignment. Rows are
    /// contiguous at this stride; the trailing bytes of each row are junk.
    pub stride: u32,
}

impl MappedFrame {
    /// Borrow the mapped bytes. The closure form keeps the mapping alive
    /// for exactly as long as the borrow.
    pub fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> Result<R> {
        let view = self
            .buffer
            .slice(..)
            .get_mapped_range()
            .context("staging buffer was not mapped")?;
        Ok(f(&view))
    }
}

impl Drop for MappedFrame {
    fn drop(&mut self) {
        self.buffer.unmap();
        self.state.store(FREE, Ordering::Release);
    }
}

// The consumer runs on its own thread (NDI send, encoder, etc.).
unsafe impl Send for MappedFrame {}

pub struct ReadbackRing {
    slots: Vec<Slot>,
    width: u32,
    height: u32,
    stride: u32,
    /// Frames skipped because every slot was busy.
    dropped: Arc<AtomicU64>,
    captured: u64,
}

impl ReadbackRing {
    /// `depth` staging buffers. 3 is the useful default: one being written
    /// by the GPU, one mapping, one being consumed.
    pub fn new(device: &wgpu::Device, width: u32, height: u32, depth: usize) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("readback ring needs a non-zero size, got {width}x{height}");
        }
        if depth == 0 {
            bail!("readback ring needs at least one slot");
        }
        const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let stride = (width * 4).div_ceil(ALIGN) * ALIGN;
        let size = (stride as u64) * (height as u64);

        let slots = (0..depth)
            .map(|i| Slot {
                buffer: Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("readback-{i}")),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();

        Ok(Self {
            slots,
            width,
            height,
            stride,
            dropped: Arc::new(AtomicU64::new(0)),
            captured: 0,
        })
    }

    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// Frames skipped because no slot was free.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Frames successfully enqueued for readback.
    pub fn captured(&self) -> u64 {
        self.captured
    }

    /// Enqueue a readback of `texture`. Returns `false` if the frame was
    /// dropped because every slot was busy — which is a normal, survivable
    /// outcome, not an error. Never blocks.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> bool {
        let Some(slot) = self.slots.iter().find(|s| {
            s.state
                .compare_exchange(FREE, MAPPING, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        }) else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        };

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        // Submitted after the caller's frame work on the same queue, so the
        // copy always sees the finished frame.
        queue.submit([encoder.finish()]);

        let state = Arc::clone(&slot.state);
        slot.buffer.slice(..).map_async(wgpu::MapMode::Read, move |res| {
            // A failed map must release the slot, or the ring bleeds
            // capacity one buffer at a time until it silently stops.
            state.store(if res.is_ok() { READY } else { FREE }, Ordering::Release);
        });
        self.captured += 1;
        true
    }

    /// Take the next completed frame, if any. Non-blocking.
    ///
    /// Map callbacks only fire while the device is polled; call this after
    /// the frame's submit (which polls) or pair it with `device.poll`.
    pub fn take_ready(&mut self) -> Option<MappedFrame> {
        let slot = self.slots.iter().find(|s| {
            s.state
                .compare_exchange(READY, INFLIGHT, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        })?;
        Some(MappedFrame {
            buffer: Arc::clone(&slot.buffer),
            state: Arc::clone(&slot.state),
            width: self.width,
            height: self.height,
            stride: self.stride,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 64;
    const H: u32 = 32;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

    /// Headless GPU (llvmpipe in CI) plus a texture cleared to a known color.
    ///
    /// Returns `None` when no adapter exists so a developer without a GPU
    /// can still run the suite — but CI sets `VIZZ_REQUIRE_GPU=1`, which
    /// turns that into a failure. Silently skipping the only tests that
    /// exercise real GPU behaviour would look identical to passing them.
    fn gpu() -> Option<(wgpu::Device, wgpu::Queue, wgpu::Texture)> {
        match try_gpu() {
            Some(g) => Some(g),
            None if std::env::var_os("VIZZ_REQUIRE_GPU").is_some() => {
                panic!("VIZZ_REQUIRE_GPU is set but no GPU adapter was found")
            }
            None => {
                eprintln!("no GPU adapter available; skipping GPU test");
                None
            }
        }
    }

    fn try_gpu() -> Option<(wgpu::Device, wgpu::Queue, wgpu::Texture)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("readback-test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .ok()?;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test-target"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Some((device, queue, texture))
    }

    /// Clear the texture to an exact BGRA value.
    fn clear(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture, c: wgpu::Color) {
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(c), store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        queue.submit([enc.finish()]);
    }

    /// Pump the device until a frame lands, with a bounded number of tries
    /// so a genuine hang fails the test instead of spinning forever.
    fn wait_ready(device: &wgpu::Device, ring: &mut ReadbackRing) -> Option<MappedFrame> {
        for _ in 0..100 {
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            if let Some(f) = ring.take_ready() {
                return Some(f);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn readback_returns_the_rendered_pixels() {
        let Some((device, queue, tex)) = gpu() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        // Opaque orange in BGRA: B=0x00, G=0x80, R=0xFF, A=0xFF.
        clear(&device, &queue, &tex, wgpu::Color { r: 1.0, g: 0.502, b: 0.0, a: 1.0 });

        let mut ring = ReadbackRing::new(&device, W, H, 3).unwrap();
        assert!(ring.capture(&device, &queue, &tex));

        let frame = wait_ready(&device, &mut ring).expect("frame never became ready");
        assert_eq!(frame.width, W);
        assert_eq!(frame.height, H);
        assert!(frame.stride >= W * 4 && frame.stride.is_multiple_of(256), "stride {}", frame.stride);

        frame
            .with_bytes(|bytes| {
                assert_eq!(bytes.len(), (frame.stride * H) as usize);
                // Check a pixel from the middle row, not just row 0, so a
                // stride mistake cannot pass by accident.
                let row = (H / 2) as usize * frame.stride as usize;
                for px in 0..W as usize {
                    let p = &bytes[row + px * 4..row + px * 4 + 4];
                    assert_eq!(p[0], 0x00, "blue @{px}");
                    assert!((p[1] as i32 - 0x80).abs() <= 1, "green @{px}: {}", p[1]);
                    assert_eq!(p[2], 0xFF, "red @{px}");
                    assert_eq!(p[3], 0xFF, "alpha @{px}");
                }
            })
            .unwrap();
    }

    #[test]
    fn saturation_drops_frames_instead_of_blocking() {
        let Some((device, queue, tex)) = gpu() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        clear(&device, &queue, &tex, wgpu::Color::BLACK);
        let mut ring = ReadbackRing::new(&device, W, H, 2).unwrap();

        // Never consume: the first `depth` captures take the slots, the
        // rest must be dropped rather than stalling.
        assert!(ring.capture(&device, &queue, &tex));
        assert!(ring.capture(&device, &queue, &tex));
        for _ in 0..10 {
            assert!(!ring.capture(&device, &queue, &tex), "should have dropped");
        }
        assert_eq!(ring.captured(), 2);
        assert_eq!(ring.dropped(), 10);
    }

    #[test]
    fn slots_recycle_after_the_frame_is_dropped() {
        let Some((device, queue, tex)) = gpu() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        clear(&device, &queue, &tex, wgpu::Color::BLACK);
        let mut ring = ReadbackRing::new(&device, W, H, 1).unwrap();

        // One slot, reused across many frames — this is the steady state of
        // a live output, so a leak here would surface as output death.
        for i in 0..8 {
            assert!(ring.capture(&device, &queue, &tex), "capture {i} found no free slot");
            let frame = wait_ready(&device, &mut ring).unwrap_or_else(|| panic!("frame {i} stuck"));
            drop(frame);
        }
        assert_eq!(ring.captured(), 8);
        assert_eq!(ring.dropped(), 0);
    }

    #[test]
    fn rejects_degenerate_configuration() {
        let Some((device, _, _)) = gpu() else { return };
        assert!(ReadbackRing::new(&device, 0, 8, 3).is_err());
        assert!(ReadbackRing::new(&device, 8, 8, 0).is_err());
    }
}
