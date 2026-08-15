//! NDI output: sends the master texture over the network as an NDI source.
//!
//! ## Why the NDI library is loaded at runtime
//!
//! The NDI SDK is a registration-walled download, and the Rust binding
//! crates fail *at build time* when it is absent. Linking it would mean
//! vizz could not be built or shipped without it. Instead the redistributable
//! NDI runtime is `dlopen`ed at startup, exactly like Syphon: the binary
//! builds and runs everywhere, and the NDI output simply reports itself
//! unavailable when the runtime is not installed.
//!
//! ## Why this does not stall the render thread
//!
//! NDI needs CPU-side pixels, which is the one output that cannot be
//! zero-copy. The work is split:
//!
//! - Render thread: [`ReadbackRing::capture`] encodes a GPU→CPU copy and
//!   returns. It never maps, waits, or memcpys.
//! - Render thread: any *already finished* frame is handed to a bounded
//!   channel. If the channel is full the frame is dropped, never awaited.
//! - Send thread: reads the mapped bytes and calls `NDIlib_send_send_video_v2`,
//!   which copies before returning — so the staging buffer can be unmapped
//!   and recycled as soon as the call completes.
//!
//! Every step that could block happens off the render thread, and every
//! queue is bounded so a slow or absent receiver degrades into dropped
//! frames rather than backpressure onto the visuals.
//!
//! Struct layouts and function signatures below mirror `Processing.NDI.structs.h`
//! and `Processing.NDI.Send.h` field-for-field; the padding `#[repr(C)]`
//! inserts matches the C compiler's.

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;

use anyhow::{Context as _, Result, anyhow, bail};

use crate::readback::{MappedFrame, ReadbackRing};
use crate::FrameSender;

/// `NDI_LIB_FOURCC('B','G','R','A')` — matches our Bgra8UnormSrgb master.
const FOURCC_BGRA: c_int = 0x4152_4742;
const FRAME_FORMAT_PROGRESSIVE: c_int = 1;
/// `NDIlib_send_timecode_synthesize`: let NDI stamp timing itself.
const TIMECODE_SYNTHESIZE: i64 = i64::MAX;

#[repr(C)]
struct SendCreate {
    p_ndi_name: *const c_char,
    p_groups: *const c_char,
    clock_video: bool,
    clock_audio: bool,
}

#[repr(C)]
struct VideoFrameV2 {
    xres: c_int,
    yres: c_int,
    four_cc: c_int,
    frame_rate_n: c_int,
    frame_rate_d: c_int,
    picture_aspect_ratio: f32,
    frame_format_type: c_int,
    timecode: i64,
    p_data: *const u8,
    /// Union with `data_size_in_bytes` in C; for uncompressed formats this
    /// is the line stride, which lets us hand NDI the padded staging rows
    /// directly instead of repacking them.
    line_stride_in_bytes: c_int,
    p_metadata: *const c_char,
    timestamp: i64,
}

type SendInstance = *mut c_void;

/// Moves the send instance to the send thread, which is its only user for
/// the rest of its life — creation hands it over and never touches it again.
struct OwnedInstance(SendInstance);
unsafe impl Send for OwnedInstance {}

/// The five entry points needed to send video, resolved once from the
/// runtime library.
struct NdiLib {
    _lib: libloading::Library,
    initialize: unsafe extern "C" fn() -> bool,
    destroy: unsafe extern "C" fn(),
    send_create: unsafe extern "C" fn(*const SendCreate) -> SendInstance,
    send_destroy: unsafe extern "C" fn(SendInstance),
    send_video_v2: unsafe extern "C" fn(SendInstance, *const VideoFrameV2),
}

// The NDI library is documented as thread-safe; the send instance is used
// only from the send thread.
unsafe impl Send for NdiLib {}
unsafe impl Sync for NdiLib {}

fn load_library() -> Result<NdiLib> {
    let mut tried = Vec::new();
    for path in candidate_paths() {
        let lib = match unsafe { libloading::Library::new(&path) } {
            Ok(lib) => lib,
            Err(e) => {
                tried.push(format!("{} ({e})", path.display()));
                continue;
            }
        };
        let resolved = (|| unsafe {
            Ok::<_, libloading::Error>(NdiLib {
                initialize: *lib.get(b"NDIlib_initialize\0")?,
                destroy: *lib.get(b"NDIlib_destroy\0")?,
                send_create: *lib.get(b"NDIlib_send_create\0")?,
                send_destroy: *lib.get(b"NDIlib_send_destroy\0")?,
                send_video_v2: *lib.get(b"NDIlib_send_send_video_v2\0")?,
                _lib: lib,
            })
        })();
        match resolved {
            Ok(ndi) => {
                log::info!("loaded NDI runtime from {}", path.display());
                return Ok(ndi);
            }
            Err(e) => tried.push(format!("{} (missing symbol: {e})", path.display())),
        }
    }
    Err(anyhow!(
        "NDI runtime not found. Tried:\n  {}\nInstall the NDI Tools/Runtime redistributable, \
         or set VIZZ_NDI_RUNTIME to the library path.",
        tried.join("\n  ")
    ))
}

/// Shared with the receive path: both need the same search order, and
/// two copies would drift the moment one platform's layout changed.
pub(crate) fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(explicit) = std::env::var("VIZZ_NDI_RUNTIME") {
        paths.push(PathBuf::from(explicit));
    }
    // The SDK sets this to the redistributable directory on install.
    for var in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
        if let Ok(dir) = std::env::var(var) {
            paths.push(PathBuf::from(&dir).join(lib_file_name()));
        }
    }
    paths.push(PathBuf::from(lib_file_name())); // loader search path
    #[cfg(target_os = "macos")]
    {
        paths.push("/usr/local/lib/libndi.dylib".into());
        paths.push("/opt/homebrew/lib/libndi.dylib".into());
        paths.push("/Library/NDI SDK for Apple/lib/macOS/libndi.dylib".into());
    }
    #[cfg(target_os = "linux")]
    {
        paths.push("/usr/local/lib/libndi.so.6".into());
        paths.push("/usr/lib/libndi.so.6".into());
        paths.push("/usr/local/lib/libndi.so".into());
    }
    #[cfg(target_os = "windows")]
    {
        paths.push(r"C:\Program Files\NDI\NDI 6 Runtime\v6\Processing.NDI.Lib.x64.dll".into());
    }
    paths
}

const fn lib_file_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libndi.dylib"
    }
    #[cfg(target_os = "linux")]
    {
        "libndi.so.6"
    }
    #[cfg(target_os = "windows")]
    {
        "Processing.NDI.Lib.x64.dll"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "libndi.so"
    }
}

/// What the send thread receives. The frame unmaps itself on drop, so a
/// dropped message automatically recycles its staging slot.
struct Outgoing {
    frame: MappedFrame,
    fps_n: c_int,
    fps_d: c_int,
}

pub struct NdiSender {
    name: String,
    ring: ReadbackRing,
    tx: Option<SyncSender<Outgoing>>,
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    /// Frames dropped because the send thread was still busy.
    send_dropped: Arc<AtomicU64>,
    fps_n: c_int,
    fps_d: c_int,
}

impl NdiSender {
    /// Start an NDI source named `name` publishing `width`x`height` at
    /// `fps_n/fps_d` (e.g. 60/1).
    pub fn new(
        device: &wgpu::Device,
        name: &str,
        width: u32,
        height: u32,
        fps_n: u32,
        fps_d: u32,
    ) -> Result<Self> {
        if fps_n == 0 || fps_d == 0 {
            bail!("NDI frame rate must be non-zero, got {fps_n}/{fps_d}");
        }
        let lib = load_library()?;
        if !unsafe { (lib.initialize)() } {
            // Documented failure mode: CPU lacks required instruction sets.
            bail!("NDIlib_initialize failed — this CPU may be unsupported by the NDI runtime");
        }

        let c_name = CString::new(name).context("NDI source name contains NUL")?;
        let settings = SendCreate {
            p_ndi_name: c_name.as_ptr(),
            p_groups: std::ptr::null(),
            // We pace frames ourselves from the render loop; letting NDI
            // clock video too would add a second, conflicting rate limiter.
            clock_video: false,
            clock_audio: false,
        };
        let instance = unsafe { (lib.send_create)(&settings) };
        if instance.is_null() {
            unsafe { (lib.destroy)() };
            bail!("NDIlib_send_create returned null for source '{name}'");
        }

        // 3 slots: one filling on the GPU, one mapping, one being sent.
        let ring = ReadbackRing::new(device, width, height, 3)?;
        // Depth 1: if the sender is still busy we would rather drop the
        // newest frame than build a latency queue in a live visual feed.
        let (tx, rx) = sync_channel::<Outgoing>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let send_dropped = Arc::new(AtomicU64::new(0));

        let thread = {
            let stop = Arc::clone(&stop);
            let name = name.to_owned();
            let owned = OwnedInstance(instance);
            std::thread::Builder::new()
                .name("vizz-ndi".into())
                .spawn(move || send_loop(lib, owned, rx, stop, &name))?
        };

        Ok(Self {
            name: name.to_owned(),
            ring,
            tx: Some(tx),
            thread: Some(thread),
            stop,
            send_dropped,
            fps_n: fps_n as c_int,
            fps_d: fps_d as c_int,
        })
    }

    /// Frames dropped by the readback ring (GPU behind) and by the send
    /// channel (network/receiver behind).
    pub fn dropped(&self) -> (u64, u64) {
        (self.ring.dropped(), self.send_dropped.load(Ordering::Relaxed))
    }
}

/// Owns the NDI instance for its lifetime and tears it down in order.
fn send_loop(
    lib: NdiLib,
    owned: OwnedInstance,
    rx: Receiver<Outgoing>,
    stop: Arc<AtomicBool>,
    name: &str,
) {
    let instance = owned.0;
    log::info!("NDI send thread for '{name}' started");
    while !stop.load(Ordering::Relaxed) {
        let Ok(msg) = rx.recv() else { break }; // sender dropped: shutting down
        let frame = &msg.frame;
        let result = frame.with_bytes(|bytes| {
            let video = VideoFrameV2 {
                xres: frame.width as c_int,
                yres: frame.height as c_int,
                four_cc: FOURCC_BGRA,
                frame_rate_n: msg.fps_n,
                frame_rate_d: msg.fps_d,
                picture_aspect_ratio: 0.0, // 0 = derive from resolution
                frame_format_type: FRAME_FORMAT_PROGRESSIVE,
                timecode: TIMECODE_SYNTHESIZE,
                p_data: bytes.as_ptr(),
                line_stride_in_bytes: frame.stride as c_int,
                p_metadata: std::ptr::null(),
                timestamp: 0,
            };
            // Synchronous send: NDI copies the pixels before returning, so
            // the staging buffer is free the moment this call ends. This
            // blocks the *send* thread only, never the renderer.
            unsafe { (lib.send_video_v2)(instance, &video) };
        });
        if let Err(e) = result {
            log::error!("NDI '{name}': could not read staged frame: {e:#}");
        }
        // Dropping `msg` unmaps the buffer and frees the ring slot.
    }
    unsafe {
        (lib.send_destroy)(instance);
        (lib.destroy)();
    }
    log::info!("NDI send thread for '{name}' stopped");
}

impl FrameSender for NdiSender {
    fn name(&self) -> &str {
        &self.name
    }

    fn publish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Result<()> {
        // 1. Enqueue this frame's GPU→CPU copy (non-blocking).
        self.ring.capture(device, queue, texture);

        // 2. Forward whatever finished earlier. Both the ring and the
        //    channel drop rather than wait, so a stalled receiver costs
        //    frames on this output and nothing else.
        if let Some(frame) = self.ring.take_ready() {
            let msg = Outgoing { frame, fps_n: self.fps_n, fps_d: self.fps_d };
            if let Some(tx) = &self.tx {
                match tx.try_send(msg) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        self.send_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        bail!("NDI send thread has stopped");
                    }
                }
            }
        }
        Ok(())
    }
}

impl Drop for NdiSender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Close the channel so a thread parked in recv() wakes and exits.
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Layout is the one place a mistake corrupts memory rather than
    // failing loudly, so assert it against the C headers explicitly.
    #[test]
    fn video_frame_layout_matches_the_sdk_header() {
        use std::mem::{align_of, offset_of, size_of};
        assert_eq!(offset_of!(VideoFrameV2, xres), 0);
        assert_eq!(offset_of!(VideoFrameV2, yres), 4);
        assert_eq!(offset_of!(VideoFrameV2, four_cc), 8);
        assert_eq!(offset_of!(VideoFrameV2, frame_rate_n), 12);
        assert_eq!(offset_of!(VideoFrameV2, frame_rate_d), 16);
        assert_eq!(offset_of!(VideoFrameV2, picture_aspect_ratio), 20);
        assert_eq!(offset_of!(VideoFrameV2, frame_format_type), 24);
        // 7 x 4-byte fields = 28, then 4 bytes of padding to 8-align i64.
        assert_eq!(offset_of!(VideoFrameV2, timecode), 32);
        assert_eq!(offset_of!(VideoFrameV2, p_data), 40);
        assert_eq!(offset_of!(VideoFrameV2, line_stride_in_bytes), 48);
        assert_eq!(offset_of!(VideoFrameV2, p_metadata), 56);
        assert_eq!(offset_of!(VideoFrameV2, timestamp), 64);
        assert_eq!(size_of::<VideoFrameV2>(), 72);
        assert_eq!(align_of::<VideoFrameV2>(), 8);
    }

    #[test]
    fn send_create_layout_matches_the_sdk_header() {
        use std::mem::{offset_of, size_of};
        assert_eq!(offset_of!(SendCreate, p_ndi_name), 0);
        assert_eq!(offset_of!(SendCreate, p_groups), 8);
        assert_eq!(offset_of!(SendCreate, clock_video), 16);
        assert_eq!(offset_of!(SendCreate, clock_audio), 17);
        assert_eq!(size_of::<SendCreate>(), 24);
    }

    #[test]
    fn fourcc_bgra_matches_the_sdk_macro() {
        // NDI_LIB_FOURCC(a,b,c,d) = a | b<<8 | c<<16 | d<<24
        let expected = (b'B' as c_int)
            | (b'G' as c_int) << 8
            | (b'R' as c_int) << 16
            | (b'A' as c_int) << 24;
        assert_eq!(FOURCC_BGRA, expected);
    }

    #[test]
    fn missing_runtime_reports_every_path_tried() {
        // Without the NDI runtime installed this must fail with a helpful
        // message rather than panicking or aborting the process.
        //
        // Guarded because the receiver's twin sets the same variable in
        // this same binary; see `crate::test_env`.
        let _guard = crate::test_env::env_guard();
        // SAFETY: the guard makes this the only thread touching the
        // variable for as long as it is held.
        unsafe { std::env::set_var("VIZZ_NDI_RUNTIME", "/nonexistent/libndi.dylib") };
        let msg = match load_library() {
            Ok(_) => panic!("should not have found a runtime at a nonexistent path"),
            Err(e) => format!("{e:#}"),
        };
        assert!(msg.contains("/nonexistent/libndi.dylib"), "{msg}");
        assert!(msg.contains("VIZZ_NDI_RUNTIME"), "{msg}");
        unsafe { std::env::remove_var("VIZZ_NDI_RUNTIME") };
    }
}
