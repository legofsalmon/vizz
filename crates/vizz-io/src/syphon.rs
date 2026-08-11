//! Syphon output (macOS): zero-copy publishing of the master texture to
//! Resolume / VDMX / MadMapper etc. via `SyphonMetalServer`.
//!
//! ## How this works
//!
//! Syphon.framework is loaded at *runtime* with `dlopen` — there is no
//! link-time dependency, so the `vizz` binary starts fine on machines
//! without Syphon installed (the sender just reports unavailable). All
//! Objective-C calls go through `objc2::msg_send!` against dynamically
//! looked-up classes.
//!
//! Frame ordering: `publishFrameTexture:onCommandBuffer:` encodes Syphon's
//! internal copy onto a command buffer we create from **wgpu's own
//! `MTLCommandQueue`** (via `Queue::as_hal`). Metal executes command
//! buffers on one queue in commit order, so committing right after
//! `wgpu::Queue::submit` guarantees the copy sees the finished frame —
//! no cross-queue events, no blocking, no added latency.

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context as _, Result, anyhow, bail};
use objc2::encode::{Encode, Encoding};
use objc2::rc::autoreleasepool;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;

use crate::FrameSender;

// CGRect and friends, declared locally so we don't need any Apple framework
// crate. CGFloat is f64 on all 64-bit Apple targets. The Encoding strings
// must match the real Objective-C type encodings exactly — objc2 verifies
// them against the method signature in debug builds.

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

const SERVER_CLASS: &CStr = c"SyphonMetalServer";

/// Load Syphon.framework once per process. Returns the server class.
/// Failure is sticky (retrying dlopen every frame would be pointless);
/// the error string explains every path that was tried.
fn syphon_class() -> Result<&'static AnyClass> {
    class_named(SERVER_CLASS)
}

/// Any class from Syphon.framework, loading the framework if needed.
///
/// Shared with the *receive* side (`syphon_recv`), which needs
/// `SyphonServerDirectory` and `SyphonMetalClient` out of the same
/// bundle — one dlopen, one error message, one place that knows where a
/// framework might live.
pub(crate) fn class_named(name: &CStr) -> Result<&'static AnyClass> {
    load_framework()?;
    AnyClass::get(name).ok_or_else(|| {
        anyhow!(
            "Syphon.framework loaded but has no {} class — it is older than this build expects",
            name.to_string_lossy()
        )
    })
}

fn load_framework() -> Result<()> {
    static LOADED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let result = LOADED.get_or_init(|| {
        // Already present? (statically linked, or a host app loaded it)
        if AnyClass::get(SERVER_CLASS).is_some() {
            return Ok(());
        }
        let mut tried = Vec::new();
        for path in candidate_paths() {
            if !path.is_file() {
                tried.push(format!("{} (not found)", path.display()));
                continue;
            }
            // Frameworks must stay loaded once their classes are
            // registered with the runtime, so leak the handle on success.
            match unsafe { libloading::Library::new(&path) } {
                Ok(lib) => {
                    std::mem::forget(lib);
                    if AnyClass::get(SERVER_CLASS).is_some() {
                        log::info!("loaded Syphon from {}", path.display());
                        return Ok(());
                    }
                    tried.push(format!(
                        "{} (loaded, but no SyphonMetalServer class)",
                        path.display()
                    ));
                }
                Err(e) => tried.push(format!("{} ({e})", path.display())),
            }
        }
        Err(format!(
            "Syphon.framework not found. Tried:\n  {}\nInstall it to ~/Library/Frameworks \
             or set VIZZ_SYPHON_FRAMEWORK to the .framework directory.",
            tried.join("\n  ")
        ))
    });
    match result {
        Ok(()) => Ok(()),
        Err(msg) => Err(anyhow!("{msg}")),
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(env) = std::env::var("VIZZ_SYPHON_FRAMEWORK") {
        let p = PathBuf::from(env);
        // Accept either the .framework directory or the binary inside it.
        if p.extension().is_some_and(|e| e == "framework") {
            paths.push(p.join("Syphon"));
        } else {
            paths.push(p);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        // App-bundle layout, then loose next to the binary (cargo run).
        paths.push(dir.join("../Frameworks/Syphon.framework/Syphon"));
        paths.push(dir.join("Syphon.framework/Syphon"));
    }
    paths.push(PathBuf::from("vendor/Syphon.framework/Syphon"));
    paths.push(PathBuf::from("Syphon.framework/Syphon"));
    if let Some(home) = std::env::home_dir() {
        paths.push(home.join("Library/Frameworks/Syphon.framework/Syphon"));
    }
    paths.push(PathBuf::from("/Library/Frameworks/Syphon.framework/Syphon"));
    paths
}

/// A running Syphon server publishing vizz's master texture.
/// `MTLPixelFormatBGRA8Unorm`. The non-sRGB sibling of the master
/// texture's format, which is what Syphon receivers expect.
const MTL_PIXEL_FORMAT_BGRA8UNORM: usize = 80;

pub struct SyphonSender {
    server: *mut AnyObject,
    name: String,
    flipped: bool,
    /// Non-sRGB view of the master texture, created once and reused.
    ///
    /// Syphon's convention is that a published texture holds display-ready
    /// bytes and receivers sample it as plain BGRA8. The master texture is
    /// `Bgra8UnormSrgb`, so publishing it directly tells the receiver the
    /// bytes are sRGB: it linearises them, its own output re-encodes, and
    /// that double decode turns bright additive output into a white
    /// rectangle. The window preview looked correct throughout, because
    /// nothing about the render was wrong.
    ///
    /// A view of the same memory with the format Metal reports as plain
    /// BGRA8 hands over exactly the bytes we wrote.
    linear_view: *mut AnyObject,
}

// The raw pointer blocks the auto-impl. Syphon servers are documented
// thread-safe, and FrameSender access is exclusive (&mut) anyway.
unsafe impl Send for SyphonSender {}

impl SyphonSender {
    /// Start a Syphon server named `name` on the Metal device wgpu is using.
    ///
    /// `flipped` should normally be true, and the caller's default says
    /// so. Metal renders with the origin at the top left; Syphon's
    /// convention is OpenGL's, origin at the bottom. Publishing a Metal
    /// texture without the flag therefore hands receivers an image that
    /// is upside down by exactly one convention — which is what shipped,
    /// and what every receiving app showed.
    pub fn new(device: &wgpu::Device, name: &str, flipped: bool) -> Result<Self> {
        let class = syphon_class()?;
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
            .context("Syphon requires the Metal backend")?;
        let mtl_device = (&**hal.raw_device()) as *const _ as *mut AnyObject;

        let name_c = CString::new(name).context("server name contains NUL")?;
        let server = autoreleasepool(|_| unsafe {
            let ns_string = AnyClass::get(c"NSString").expect("Foundation always present");
            let ns_name: *mut AnyObject =
                msg_send![ns_string, stringWithUTF8String: name_c.as_ptr() as *const c_char];
            let alloc: *mut AnyObject = msg_send![class, alloc];
            let server: *mut AnyObject = msg_send![
                alloc,
                initWithName: ns_name,
                device: mtl_device,
                options: std::ptr::null::<AnyObject>(),
            ];
            server
        });
        if server.is_null() {
            bail!("SyphonMetalServer failed to initialize");
        }
        Ok(Self {
            server,
            name: name.to_owned(),
            flipped,
            linear_view: std::ptr::null_mut(),
        })
    }
}

impl FrameSender for SyphonSender {
    fn name(&self) -> &str {
        &self.name
    }

    fn publish(
        &mut self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Result<()> {
        let size = texture.size();
        let region = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: size.width as f64,
                height: size.height as f64,
            },
        };
        // Guards must outlive the raw pointers derived from them.
        let hal_queue = unsafe { queue.as_hal::<wgpu::hal::api::Metal>() }
            .context("Syphon requires the Metal backend")?;
        let hal_texture = unsafe { texture.as_hal::<wgpu::hal::api::Metal>() }
            .context("texture is not a Metal texture")?;
        let queue_ptr = hal_queue.as_raw() as *const _ as *mut AnyObject;
        let texture_ptr = hal_texture.raw_handle() as *const _ as *mut AnyObject;

        // Created on first publish and kept: the master texture lives for
        // the run, and building a view every frame would be pure waste.
        if self.linear_view.is_null() {
            self.linear_view = unsafe {
                msg_send![texture_ptr, newTextureViewWithPixelFormat: MTL_PIXEL_FORMAT_BGRA8UNORM]
            };
            if self.linear_view.is_null() {
                bail!("could not create a non-sRGB Metal view for Syphon");
            }
        }
        let publish_ptr = self.linear_view;

        autoreleasepool(|_| unsafe {
            let cmd_buf: *mut AnyObject = msg_send![queue_ptr, commandBuffer];
            if cmd_buf.is_null() {
                bail!("could not create Metal command buffer for Syphon publish");
            }
            let _: () = msg_send![
                self.server,
                publishFrameTexture: publish_ptr,
                onCommandBuffer: cmd_buf,
                imageRegion: region,
                flipped: Bool::new(self.flipped),
            ];
            let _: () = msg_send![cmd_buf, commit];
            Ok(())
        })
    }
}

impl Drop for SyphonSender {
    fn drop(&mut self) {
        autoreleasepool(|_| unsafe {
            // `newTextureView…` returns +1, so this owns a reference.
            if !self.linear_view.is_null() {
                let _: () = msg_send![self.linear_view, release];
            }
            let _: () = msg_send![self.server, stop];
            let _: () = msg_send![self.server, release];
        });
    }
}
