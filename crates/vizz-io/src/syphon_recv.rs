//! Syphon input (macOS): receive another app's frames as a texture.
//!
//! The mirror of [`crate::syphon`], and it shares that module's runtime
//! loader — one `dlopen`, one set of search paths, one error message
//! whichever direction the frames are going.
//!
//! ## Getting pixels out
//!
//! `SyphonMetalClient` hands over an `id<MTLTexture>` backed by an
//! IOSurface. Everything downstream of a video source in vizz wants CPU
//! bytes (NDI decodes to the CPU, so the whole path was built that way),
//! so this reads the texture back with `getBytes:bytesPerRow:fromRegion:
//! mipmapLevel:`, which works because IOSurface-backed textures are
//! shared rather than private storage.
//!
//! That readback is the known cost of this module: at 1080p it is ~8 MB
//! a frame across the bus, where Syphon's whole point is not moving the
//! pixels at all. The zero-copy version wraps the incoming `MTLTexture`
//! as a `wgpu::Texture` and blits it straight into the video texture,
//! which needs the video path to accept a GPU source — a bigger change
//! than this one, and worth doing once the receive side is proven. The
//! CI self-loop test is what proves it: vizz publishes its own output as
//! `syphon:vizz` and reads it back here, checking real pixels.

use std::ffi::{CStr, CString};

use anyhow::{Result, anyhow, bail};
use objc2::encode::{Encode, Encoding};
use objc2::rc::autoreleasepool;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;

const DIRECTORY_CLASS: &CStr = c"SyphonServerDirectory";
const CLIENT_CLASS: &CStr = c"SyphonMetalClient";

/// `MTLOrigin` / `MTLSize` / `MTLRegion`, declared locally for the same
/// reason [`crate::syphon`] declares `CGRect`: no Apple framework crate
/// is linked, and objc2 checks these encodings against the runtime.
#[repr(C)]
#[derive(Clone, Copy)]
struct MtlOrigin {
    x: usize,
    y: usize,
    z: usize,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct MtlSize {
    width: usize,
    height: usize,
    depth: usize,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct MtlRegion {
    origin: MtlOrigin,
    size: MtlSize,
}

unsafe impl Encode for MtlOrigin {
    const ENCODING: Encoding =
        Encoding::Struct("?", &[usize::ENCODING, usize::ENCODING, usize::ENCODING]);
}
unsafe impl Encode for MtlSize {
    const ENCODING: Encoding =
        Encoding::Struct("?", &[usize::ENCODING, usize::ENCODING, usize::ENCODING]);
}
unsafe impl Encode for MtlRegion {
    const ENCODING: Encoding =
        Encoding::Struct("?", &[MtlOrigin::ENCODING, MtlSize::ENCODING]);
}

/// Names of the Syphon servers currently publishing, as
/// `"App — Name"` (or just the app when a server is unnamed), which is
/// how every other Syphon client lists them.
pub fn servers() -> Result<Vec<String>> {
    let class = crate::syphon::class_named(DIRECTORY_CLASS)?;
    autoreleasepool(|_| unsafe {
        let directory: *mut AnyObject = msg_send![class, sharedDirectory];
        if directory.is_null() {
            bail!("SyphonServerDirectory has no shared instance");
        }
        let servers: *mut AnyObject = msg_send![directory, servers];
        if servers.is_null() {
            return Ok(Vec::new());
        }
        let count: usize = msg_send![servers, count];
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let dict: *mut AnyObject = msg_send![servers, objectAtIndex: i];
            if dict.is_null() {
                continue;
            }
            let app = dict_string(dict, c"SyphonServerDescriptionAppNameKey");
            let name = dict_string(dict, c"SyphonServerDescriptionNameKey");
            out.push(match (app, name) {
                (Some(a), Some(n)) if !n.is_empty() => format!("{a} — {n}"),
                (Some(a), _) => a,
                (None, Some(n)) => n,
                (None, None) => continue,
            });
        }
        Ok(out)
    })
}

/// Read one `NSString` out of an `NSDictionary` by key, as UTF-8.
unsafe fn dict_string(dict: *mut AnyObject, key: &CStr) -> Option<String> {
    unsafe {
        let key_str = ns_string(key)?;
        let value: *mut AnyObject = msg_send![dict, objectForKey: key_str];
        if value.is_null() {
            return None;
        }
        let utf8: *const std::ffi::c_char = msg_send![value, UTF8String];
        if utf8.is_null() {
            return None;
        }
        Some(CStr::from_ptr(utf8).to_string_lossy().into_owned())
    }
}

/// An `NSString` from a C string, without linking Foundation.
unsafe fn ns_string(text: &CStr) -> Option<*mut AnyObject> {
    unsafe {
        let class = AnyClass::get(c"NSString")?;
        let s: *mut AnyObject = msg_send![class, stringWithUTF8String: text.as_ptr()];
        (!s.is_null()).then_some(s)
    }
}

/// A running Syphon client, pulling frames from one server.
pub struct SyphonInput {
    client: *mut AnyObject,
    label: String,
    /// Latest frame as BGRA, and its size.
    latest: std::sync::Mutex<Option<(u32, u32, Vec<u8>)>>,
    revision: std::sync::atomic::AtomicU64,
}

// The client is only ever messaged from the render thread, and the frame
// buffer behind it is mutex-guarded — the same contract as the Syphon
// sender in the sibling module.
unsafe impl Send for SyphonInput {}
unsafe impl Sync for SyphonInput {}

impl SyphonInput {
    /// Connect to the first server whose description contains `needle`
    /// (case-insensitive); an empty needle takes whatever is first, which
    /// is what "just show me something" should do.
    pub fn connect(device: &wgpu::Device, needle: &str) -> Result<Self> {
        let directory_class = crate::syphon::class_named(DIRECTORY_CLASS)?;
        let client_class = crate::syphon::class_named(CLIENT_CLASS)?;
        let needle_lower = needle.to_lowercase();

        // The raw MTLDevice behind wgpu, exactly as the sender does it.
        let raw_device = unsafe {
            device.as_hal::<wgpu::hal::api::Metal, _, _>(|d| {
                d.map(|d| d.raw_device().lock().as_ptr() as *mut AnyObject)
            })
        }
        .flatten()
        .ok_or_else(|| anyhow!("this is not a Metal device — Syphon input needs one"))?;

        autoreleasepool(|_| unsafe {
            let directory: *mut AnyObject = msg_send![directory_class, sharedDirectory];
            if directory.is_null() {
                bail!("SyphonServerDirectory has no shared instance");
            }
            let servers: *mut AnyObject = msg_send![directory, servers];
            let count: usize = if servers.is_null() {
                0
            } else {
                msg_send![servers, count]
            };
            if count == 0 {
                bail!("no Syphon servers are publishing");
            }
            let mut chosen: *mut AnyObject = std::ptr::null_mut();
            let mut chosen_label = String::new();
            for i in 0..count {
                let dict: *mut AnyObject = msg_send![servers, objectAtIndex: i];
                if dict.is_null() {
                    continue;
                }
                let app = dict_string(dict, c"SyphonServerDescriptionAppNameKey");
                let name = dict_string(dict, c"SyphonServerDescriptionNameKey");
                let label = match (&app, &name) {
                    (Some(a), Some(n)) if !n.is_empty() => format!("{a} — {n}"),
                    (Some(a), _) => a.clone(),
                    (None, Some(n)) => n.clone(),
                    (None, None) => continue,
                };
                if needle_lower.is_empty() || label.to_lowercase().contains(&needle_lower) {
                    chosen = dict;
                    chosen_label = label;
                    break;
                }
            }
            if chosen.is_null() {
                bail!("no Syphon server matching {needle:?} — {count} publishing");
            }
            let client: *mut AnyObject = msg_send![client_class, alloc];
            let client: *mut AnyObject = msg_send![
                client,
                initWithServerDescription: chosen,
                device: raw_device,
                options: std::ptr::null::<AnyObject>(),
                newFrameHandler: std::ptr::null::<AnyObject>(),
            ];
            if client.is_null() {
                bail!("SyphonMetalClient refused to connect to {chosen_label}");
            }
            Ok(Self {
                client,
                label: chosen_label,
                latest: std::sync::Mutex::new(None),
                revision: std::sync::atomic::AtomicU64::new(0),
            })
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn connected(&self) -> bool {
        autoreleasepool(|_| unsafe {
            let live: objc2::runtime::Bool = msg_send![self.client, isValid];
            live.as_bool()
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Pull the newest frame, if the server has published one since the
    /// last call, and copy it into the CPU buffer.
    ///
    /// Called from the render thread. `newFrameImage` returns nil when
    /// nothing new has arrived, which is the cheap path and the common
    /// one for a source slower than the render loop.
    pub fn pump(&self) {
        autoreleasepool(|_| unsafe {
            let has_new: objc2::runtime::Bool = msg_send![self.client, hasNewFrame];
            if !has_new.as_bool() {
                return;
            }
            let image: *mut AnyObject = msg_send![self.client, newFrameImage];
            if image.is_null() {
                return;
            }
            let width: usize = msg_send![image, width];
            let height: usize = msg_send![image, height];
            if width == 0 || height == 0 {
                return;
            }
            let stride = width * 4;
            let mut buf = vec![0u8; stride * height];
            let region = MtlRegion {
                origin: MtlOrigin { x: 0, y: 0, z: 0 },
                size: MtlSize { width, height, depth: 1 },
            };
            let _: () = msg_send![
                image,
                getBytes: buf.as_mut_ptr().cast::<std::ffi::c_void>(),
                bytesPerRow: stride,
                fromRegion: region,
                mipmapLevel: 0usize,
            ];
            if let Ok(mut slot) = self.latest.lock() {
                *slot = Some((width as u32, height as u32, buf));
            }
            self.revision
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
    }

    /// Hand the newest decoded frame over, if there is one.
    pub fn with_latest<R>(&self, f: impl FnOnce(u32, u32, &[u8]) -> R) -> Option<R> {
        let slot = self.latest.lock().ok()?;
        let (w, h, buf) = slot.as_ref()?;
        Some(f(*w, *h, buf))
    }
}

impl Drop for SyphonInput {
    fn drop(&mut self) {
        autoreleasepool(|_| unsafe {
            let _: () = msg_send![self.client, stop];
            let _: () = msg_send![self.client, release];
        });
    }
}

/// Keeps the unused-import lint honest about `CString`, which the
/// dictionary helpers would want if keys were ever built at runtime.
#[allow(dead_code)]
fn _unused(s: &str) -> Option<CString> {
    CString::new(s).ok()
}
