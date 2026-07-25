//! Output sender construction and the fail-soft publish loop.

use vizz_io::FrameSender;

// Some fields/args are only touched on the platforms whose backend uses them.
#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct OutputOpts {
    pub syphon: bool,
    pub syphon_name: String,
    pub syphon_flip: bool,
    pub ndi: bool,
    pub ndi_name: String,
    /// Master output size and rate, needed by senders that describe the
    /// stream up front (NDI).
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Build every sender that can come up. A sender failing to start is a
/// warning, never a startup failure — the show runs without it.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables, unused_mut))]
pub fn build_senders(device: &wgpu::Device, opts: &OutputOpts) -> Vec<Box<dyn FrameSender>> {
    let mut senders: Vec<Box<dyn FrameSender>> = Vec::new();

    #[cfg(target_os = "macos")]
    if opts.syphon {
        match vizz_io::syphon::SyphonSender::new(device, &opts.syphon_name, opts.syphon_flip) {
            Ok(sender) => {
                log::info!("Syphon output '{}' is live", opts.syphon_name);
                senders.push(Box::new(sender));
            }
            Err(e) => log::warn!("Syphon output unavailable: {e:#}"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    if opts.syphon {
        log::debug!("Syphon is macOS-only; no sender started");
    }

    if opts.ndi {
        match vizz_io::ndi::NdiSender::new(
            device,
            &opts.ndi_name,
            opts.width,
            opts.height,
            opts.fps,
            1,
        ) {
            Ok(sender) => {
                log::info!("NDI output '{}' is live", opts.ndi_name);
                senders.push(Box::new(sender));
            }
            Err(e) => log::warn!("NDI output unavailable: {e:#}"),
        }
    }

    if senders.is_empty() {
        log::info!("no video outputs active (preview/headless only)");
    }
    senders
}

/// Publish the master texture to every sender. A sender that errors is
/// logged and dropped — output loss must never take down the render loop.
pub fn publish_all(
    senders: &mut Vec<Box<dyn FrameSender>>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) {
    senders.retain_mut(|s| match s.publish(device, queue, texture) {
        Ok(()) => true,
        Err(e) => {
            log::error!("output '{}' failed and was disabled: {e:#}", s.name());
            false
        }
    });
}
