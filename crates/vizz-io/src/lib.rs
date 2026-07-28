//! Video I/O backends: how vizz's output reaches Resolume/TouchDesigner/
//! MadMapper, and how external sources come in.
//!
//! Planned backends behind these traits:
//!
//! | Backend | Platform | Transport | Cost |
//! |---------|----------|-----------|------|
//! | Syphon  | macOS    | IOSurface-backed `MTLTexture` share | zero-copy |
//! | Spout   | Windows  | DXGI shared handle (keyed mutex)    | zero-copy |
//! | NDI     | all      | network, SpeedHQ codec              | CPU encode + readback |
//!
//! Design rules the backends must obey:
//!
//! 1. **Never stall the render thread.** NDI needs CPU-side pixels, so it
//!    uses a ring of staging buffers with async `map_async` readback and a
//!    dedicated send thread — the render thread only encodes a
//!    `copy_texture_to_buffer` and moves on. If the ring is full the frame
//!    is *dropped for that output*, never awaited.
//! 2. **Fail soft.** A sender that errors logs, tears down, and retries in
//!    the background. Output loss must never propagate to the render loop.
//! 3. **Zero-copy where the OS allows.** Syphon/Spout publish the render
//!    target itself (or a blit into a shared texture) — no CPU pixels.

use anyhow::Result;

pub mod ndi;
pub mod ndi_recv;
pub mod readback;
#[cfg(target_os = "macos")]
pub mod syphon;

/// A sink that publishes frames to the outside world (Syphon/Spout/NDI).
///
/// `publish` is called on the render thread right after the frame's work
/// has been submitted to `queue`; it must return without blocking on GPU
/// or network work. Implementations that need GPU copies enqueue them
/// (ordered after the submitted frame) and move on.
pub trait FrameSender: Send {
    fn name(&self) -> &str;

    /// Publish `texture` (the finished master frame). Implementations
    /// either share it zero-copy or enqueue an async copy — they must
    /// not wait.
    fn publish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Result<()>;
}

/// A source that delivers external frames (NDI/Syphon/Spout receive) as a
/// texture the scene graph can consume like any generator.
pub trait FrameReceiver: Send {
    fn name(&self) -> &str;

    /// Latest available frame, or `None` if the source has nothing new /
    /// has disappeared. Must not block.
    fn latest(&mut self, device: &wgpu::Device, queue: &wgpu::Queue)
    -> Option<wgpu::TextureView>;
}

/// No-op sender: keeps the output plumbing exercised (and benchmarkable)
/// before the real backends land.
pub struct NullSender;

impl FrameSender for NullSender {
    fn name(&self) -> &str {
        "null"
    }

    fn publish(
        &mut self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _texture: &wgpu::Texture,
    ) -> Result<()> {
        Ok(())
    }
}
