//! GPU context and scenes.
//!
//! The renderer draws to any `wgpu::TextureView` it is handed — a swapchain
//! frame in windowed mode, an offscreen texture in headless mode, and later
//! the shared textures behind Syphon/Spout/NDI outputs. Nothing in here may
//! block: no locks shared with control threads, no synchronous readbacks.

use anyhow::{Context as _, Result};

pub mod attractor;
pub mod blit;
pub mod camera;
pub mod output;
pub mod particles;
pub mod plystream;
pub mod pointcloud;
pub mod post;
pub mod room;

/// Owned GPU handles, shared by scenes and (later) I/O backends.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Bring up an adapter + device. Pass the surface in windowed mode so
    /// the adapter is guaranteed to be able to present to it.
    pub async fn new(compatible_surface: Option<&wgpu::Surface<'_>>) -> Result<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .context("no compatible GPU adapter found")?;

        let info = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}, {:?} backend)",
            info.name,
            info.device_type,
            info.backend
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("vizz-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .context("failed to create GPU device")?;

        // A lost device mid-set must be loud in logs, not a silent hang.
        device.set_device_lost_callback(|reason, msg| {
            log::error!("GPU device lost ({reason:?}): {msg}");
        });

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}
