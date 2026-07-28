//! NDI **input**: receiving another machine's or another app's video.
//!
//! Everything else in this crate sends. This is the first path that takes
//! something in, which is what makes vizz mixable rather than only a
//! source — you can bring in Resolume's output, a camera over NDI, or
//! another vizz instance, and run it through the same effects chain.
//!
//! Same shape as the sender, for the same reasons:
//!
//! 1. **The render thread never waits.** A receive thread owns the NDI
//!    instance and writes finished frames into a slot; the renderer takes
//!    whatever is there. A stalled network drops frames rather than
//!    missing vsync.
//! 2. **Fail soft.** No runtime, no source, a source that vanishes
//!    mid-set — all of these log and leave the input reporting itself
//!    unavailable. None of them stop the visuals.
//!
//! The FFI is hand-declared rather than bindgen'd, matching `ndi.rs`, so
//! the layout assertions in the tests below are the only thing standing
//! between a header change and silent memory corruption.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Result, anyhow, bail};

/// A source as NDI advertises it on the network.
#[repr(C)]
#[derive(Clone, Copy)]
struct Source {
    p_ndi_name: *const c_char,
    /// Union with `p_ip_address` in C. Same size either way, and we only
    /// ever read the name.
    p_url_address: *const c_char,
}

#[repr(C)]
struct FindCreate {
    show_local_sources: bool,
    p_groups: *const c_char,
    p_extra_ips: *const c_char,
}

#[repr(C)]
struct RecvCreateV3 {
    source_to_connect_to: Source,
    color_format: c_int,
    bandwidth: c_int,
    allow_video_fields: bool,
    p_ndi_recv_name: *const c_char,
}

/// Mirrors `NDIlib_video_frame_v2_t`. Identical to the sender's copy, but
/// declared here too: sharing it would couple the two modules' ABI
/// assertions, and the point of asserting layout is that each declaration
/// stands on its own.
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
    p_data: *mut u8,
    line_stride_in_bytes: c_int,
    p_metadata: *const c_char,
    timestamp: i64,
}

impl Default for VideoFrameV2 {
    fn default() -> Self {
        // Zeroed is what the SDK expects for an out-parameter; capture
        // fills it and `recv_free_video_v2` takes it back.
        Self {
            xres: 0,
            yres: 0,
            four_cc: 0,
            frame_rate_n: 0,
            frame_rate_d: 0,
            picture_aspect_ratio: 0.0,
            frame_format_type: 0,
            timecode: 0,
            p_data: std::ptr::null_mut(),
            line_stride_in_bytes: 0,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        }
    }
}

type FindInstance = *mut c_void;
type RecvInstance = *mut c_void;

/// `NDIlib_frame_type_e`. Only video is acted on; the rest are either
/// ignored or, for `Error`, a reason to reconnect.
const FRAME_TYPE_NONE: c_int = 0;
const FRAME_TYPE_VIDEO: c_int = 1;
const FRAME_TYPE_ERROR: c_int = 4;

/// `NDIlib_recv_color_format_BGRX_BGRA`: ask for BGRA regardless of what
/// the sender uses, so the conversion happens inside NDI's own optimised
/// path rather than in ours. It matches the texture format the renderer
/// already uploads.
const COLOR_FORMAT_BGRX_BGRA: c_int = 0;
/// `NDIlib_recv_bandwidth_highest`.
const BANDWIDTH_HIGHEST: c_int = 100;

struct RecvLib {
    _lib: libloading::Library,
    initialize: unsafe extern "C" fn() -> bool,
    find_create_v2: unsafe extern "C" fn(*const FindCreate) -> FindInstance,
    find_destroy: unsafe extern "C" fn(FindInstance),
    find_wait_for_sources: unsafe extern "C" fn(FindInstance, u32) -> bool,
    find_get_current_sources: unsafe extern "C" fn(FindInstance, *mut u32) -> *const Source,
    recv_create_v3: unsafe extern "C" fn(*const RecvCreateV3) -> RecvInstance,
    recv_destroy: unsafe extern "C" fn(RecvInstance),
    recv_capture_v2: unsafe extern "C" fn(
        RecvInstance,
        *mut VideoFrameV2,
        *mut c_void,
        *mut c_void,
        u32,
    ) -> c_int,
    recv_free_video_v2: unsafe extern "C" fn(RecvInstance, *const VideoFrameV2),
}

// Documented thread-safe; each instance is used from one thread only.
unsafe impl Send for RecvLib {}
unsafe impl Sync for RecvLib {}

fn load() -> Result<RecvLib> {
    let mut tried = Vec::new();
    for path in crate::ndi::candidate_paths() {
        let lib = match unsafe { libloading::Library::new(&path) } {
            Ok(lib) => lib,
            Err(e) => {
                tried.push(format!("{} ({e})", path.display()));
                continue;
            }
        };
        let resolved = (|| unsafe {
            Ok::<_, libloading::Error>(RecvLib {
                initialize: *lib.get(b"NDIlib_initialize\0")?,
                find_create_v2: *lib.get(b"NDIlib_find_create_v2\0")?,
                find_destroy: *lib.get(b"NDIlib_find_destroy\0")?,
                find_wait_for_sources: *lib.get(b"NDIlib_find_wait_for_sources\0")?,
                find_get_current_sources: *lib.get(b"NDIlib_find_get_current_sources\0")?,
                recv_create_v3: *lib.get(b"NDIlib_recv_create_v3\0")?,
                recv_destroy: *lib.get(b"NDIlib_recv_destroy\0")?,
                recv_capture_v2: *lib.get(b"NDIlib_recv_capture_v2\0")?,
                recv_free_video_v2: *lib.get(b"NDIlib_recv_free_video_v2\0")?,
                _lib: lib,
            })
        })();
        match resolved {
            Ok(ndi) => return Ok(ndi),
            Err(e) => tried.push(format!("{} (missing symbol: {e})", path.display())),
        }
    }
    Err(anyhow!(
        "NDI runtime not found. Tried:\n  {}\nInstall the NDI Tools/Runtime redistributable, \
         or set VIZZ_NDI_RUNTIME to the library path.",
        tried.join("\n  ")
    ))
}

/// Names of the NDI sources currently visible, for `--list-ndi` and the
/// panel's source picker.
///
/// Blocks up to `wait_ms` for the first announcements: NDI discovery is
/// asynchronous, and asking immediately after creating the finder
/// reliably returns nothing, which reads as "no sources on the network"
/// when it means "not yet".
pub fn sources(wait_ms: u32) -> Result<Vec<String>> {
    let ndi = load()?;
    unsafe {
        if !(ndi.initialize)() {
            bail!("NDIlib_initialize failed — the CPU may be unsupported by the NDI runtime");
        }
        let create = FindCreate {
            // Include this machine's own sources: running vizz into vizz
            // on one box is a legitimate way to test, and hiding local
            // sources makes that look broken.
            show_local_sources: true,
            p_groups: std::ptr::null(),
            p_extra_ips: std::ptr::null(),
        };
        let finder = (ndi.find_create_v2)(&create);
        if finder.is_null() {
            bail!("NDIlib_find_create_v2 returned null");
        }
        (ndi.find_wait_for_sources)(finder, wait_ms);
        let mut count: u32 = 0;
        let list = (ndi.find_get_current_sources)(finder, &mut count);
        let mut names = Vec::new();
        if !list.is_null() {
            for i in 0..count as usize {
                let src = &*list.add(i);
                if src.p_ndi_name.is_null() {
                    continue;
                }
                names.push(CStr::from_ptr(src.p_ndi_name).to_string_lossy().into_owned());
            }
        }
        // The source list is owned by the finder, so it has to be read
        // before this call and not held past it.
        (ndi.find_destroy)(finder);
        names.sort();
        Ok(names)
    }
}

/// A received frame: BGRA rows, possibly padded.
#[derive(Default)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Bytes per row as NDI delivered them. May exceed `width * 4`, and
    /// is passed through to the texture upload rather than repacked.
    pub stride: u32,
    pub pixels: Vec<u8>,
}

/// Shared slot holding the most recent frame.
///
/// One slot, not a queue: for a visual input, the newest frame is the only
/// one worth having. Queueing would trade latency for frames nobody sees.
#[derive(Default)]
struct Slot {
    frame: Mutex<Frame>,
    /// Bumped on every write, so the renderer can tell a new frame from a
    /// repeat without comparing pixels.
    revision: AtomicU64,
    connected: AtomicBool,
    dropped: AtomicU64,
}

/// A running NDI input.
pub struct NdiInput {
    slot: Arc<Slot>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    source: String,
}

impl NdiInput {
    /// Connect to the first source whose name contains `needle`, or the
    /// first source at all when it is empty.
    ///
    /// Returns immediately; connection happens on the receive thread, so a
    /// source that is not up yet is a wait rather than a failure.
    pub fn connect(needle: &str) -> Result<Self> {
        let ndi = load()?;
        let slot = Arc::new(Slot::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st, needle_owned) =
            (Arc::clone(&slot), Arc::clone(&stop), needle.to_string());
        let thread = std::thread::Builder::new()
            .name("ndi-recv".into())
            .spawn(move || receive_loop(ndi, &needle_owned, &s, &st))?;
        Ok(Self {
            slot,
            stop,
            thread: Some(thread),
            source: needle.to_string(),
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn connected(&self) -> bool {
        self.slot.connected.load(Ordering::Relaxed)
    }

    /// Frames the receive thread could not hand over because the renderer
    /// still held the slot. Dropping is correct — see [`Slot`] — but the
    /// count is worth showing.
    pub fn dropped(&self) -> u64 {
        self.slot.dropped.load(Ordering::Relaxed)
    }

    /// Current revision, for deciding whether an upload is needed.
    pub fn revision(&self) -> u64 {
        self.slot.revision.load(Ordering::Acquire)
    }

    /// Run `f` over the latest frame, if the slot is free.
    ///
    /// `try_lock`, not `lock`: this is called from the render thread, and
    /// waiting on the receive thread to finish a memcpy is exactly the
    /// stall the whole design exists to avoid. A missed frame is one
    /// frame of staleness.
    pub fn with_latest<R>(&self, f: impl FnOnce(&Frame) -> R) -> Option<R> {
        self.slot.frame.try_lock().ok().map(|frame| f(&frame))
    }
}

impl Drop for NdiInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            // The capture call has a timeout, so the thread checks `stop`
            // at least that often and joining cannot hang.
            let _ = t.join();
        }
    }
}

fn receive_loop(ndi: RecvLib, needle: &str, slot: &Arc<Slot>, stop: &Arc<AtomicBool>) {
    unsafe {
        if !(ndi.initialize)() {
            log::error!("NDI input: NDIlib_initialize failed");
            return;
        }
    }
    while !stop.load(Ordering::Relaxed) {
        let Some(receiver) = connect_to(&ndi, needle, stop) else {
            continue;
        };
        slot.connected.store(true, Ordering::Relaxed);
        log::info!("NDI input connected");
        capture_until_error(&ndi, receiver, slot, stop);
        slot.connected.store(false, Ordering::Relaxed);
        unsafe { (ndi.recv_destroy)(receiver) };
        if !stop.load(Ordering::Relaxed) {
            log::warn!("NDI input lost the source — retrying");
        }
    }
}

/// Find a matching source and open a receiver on it, or `None` if none
/// appeared this round. Reconnection is the normal case, not the
/// exception: sources come and go as other apps start and stop.
fn connect_to(ndi: &RecvLib, needle: &str, stop: &Arc<AtomicBool>) -> Option<RecvInstance> {
    unsafe {
        let create = FindCreate {
            show_local_sources: true,
            p_groups: std::ptr::null(),
            p_extra_ips: std::ptr::null(),
        };
        let finder = (ndi.find_create_v2)(&create);
        if finder.is_null() {
            log::error!("NDI input: find_create_v2 returned null");
            return None;
        }
        (ndi.find_wait_for_sources)(finder, 1000);
        if stop.load(Ordering::Relaxed) {
            (ndi.find_destroy)(finder);
            return None;
        }
        let mut count: u32 = 0;
        let list = (ndi.find_get_current_sources)(finder, &mut count);
        let mut chosen: Option<Source> = None;
        if !list.is_null() {
            for i in 0..count as usize {
                let src = *list.add(i);
                if src.p_ndi_name.is_null() {
                    continue;
                }
                let name = CStr::from_ptr(src.p_ndi_name).to_string_lossy();
                if needle.is_empty() || name.to_lowercase().contains(&needle.to_lowercase()) {
                    log::info!("NDI input: connecting to {name}");
                    chosen = Some(src);
                    break;
                }
            }
        }
        let Some(source) = chosen else {
            (ndi.find_destroy)(finder);
            return None;
        };
        // The receiver copies what it needs from the source struct during
        // create, so the finder can go immediately after.
        let name = CString::new("vizz").unwrap_or_default();
        let create = RecvCreateV3 {
            source_to_connect_to: source,
            color_format: COLOR_FORMAT_BGRX_BGRA,
            bandwidth: BANDWIDTH_HIGHEST,
            allow_video_fields: false,
            p_ndi_recv_name: name.as_ptr(),
        };
        let receiver = (ndi.recv_create_v3)(&create);
        (ndi.find_destroy)(finder);
        if receiver.is_null() {
            log::error!("NDI input: recv_create_v3 returned null");
            return None;
        }
        Some(receiver)
    }
}

fn capture_until_error(
    ndi: &RecvLib,
    receiver: RecvInstance,
    slot: &Arc<Slot>,
    stop: &Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        let mut video = VideoFrameV2::default();
        // 200 ms: long enough not to spin, short enough that stopping is
        // responsive and a dead source is noticed promptly.
        let kind = unsafe {
            (ndi.recv_capture_v2)(
                receiver,
                &mut video,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                200,
            )
        };
        match kind {
            FRAME_TYPE_VIDEO => {
                copy_into(slot, &video);
                // Always freed, including when the copy was skipped —
                // the frame belongs to NDI and leaking it is a leak per
                // frame received, which at 60 fps is a fast one.
                unsafe { (ndi.recv_free_video_v2)(receiver, &video) };
            }
            FRAME_TYPE_ERROR => return,
            // None means the timeout elapsed with nothing to show, which
            // is normal for a source that is idle rather than gone.
            FRAME_TYPE_NONE => {}
            _ => {}
        }
    }
}

fn copy_into(slot: &Arc<Slot>, video: &VideoFrameV2) {
    if video.p_data.is_null() || video.xres <= 0 || video.yres <= 0 {
        return;
    }
    let Ok(mut frame) = slot.frame.try_lock() else {
        slot.dropped.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let height = video.yres as usize;
    let stride = video.line_stride_in_bytes.max(video.xres * 4) as usize;
    let len = stride * height;
    frame.width = video.xres as u32;
    frame.height = video.yres as u32;
    frame.stride = stride as u32;
    frame.pixels.clear();
    frame.pixels.reserve(len);
    // SAFETY: NDI guarantees `p_data` covers `line_stride * yres` bytes
    // for an uncompressed frame, and the frame is not freed until this
    // returns.
    unsafe {
        frame
            .pixels
            .extend_from_slice(std::slice::from_raw_parts(video.p_data, len));
    }
    slot.revision.fetch_add(1, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layout is the one place a mistake corrupts memory rather than
    /// failing loudly. Asserted against the C headers explicitly, exactly
    /// as the sender's structs are.
    #[test]
    fn source_layout_matches_the_sdk_header() {
        use std::mem::{offset_of, size_of};
        assert_eq!(offset_of!(Source, p_ndi_name), 0);
        assert_eq!(offset_of!(Source, p_url_address), 8);
        assert_eq!(size_of::<Source>(), 16);
    }

    #[test]
    fn find_create_layout_matches_the_sdk_header() {
        use std::mem::{offset_of, size_of};
        // bool first, then two pointers — so 7 bytes of padding after it.
        assert_eq!(offset_of!(FindCreate, show_local_sources), 0);
        assert_eq!(offset_of!(FindCreate, p_groups), 8);
        assert_eq!(offset_of!(FindCreate, p_extra_ips), 16);
        assert_eq!(size_of::<FindCreate>(), 24);
    }

    #[test]
    fn recv_create_layout_matches_the_sdk_header() {
        use std::mem::{offset_of, size_of};
        assert_eq!(offset_of!(RecvCreateV3, source_to_connect_to), 0);
        assert_eq!(offset_of!(RecvCreateV3, color_format), 16);
        assert_eq!(offset_of!(RecvCreateV3, bandwidth), 20);
        assert_eq!(offset_of!(RecvCreateV3, allow_video_fields), 24);
        // 7 bytes of padding before the pointer.
        assert_eq!(offset_of!(RecvCreateV3, p_ndi_recv_name), 32);
        assert_eq!(size_of::<RecvCreateV3>(), 40);
    }

    /// The receiver's copy of the video frame must match the sender's,
    /// since both describe the same C struct. Declared twice deliberately;
    /// this is what makes that safe.
    #[test]
    fn the_video_frame_matches_the_senders_declaration() {
        use std::mem::{align_of, size_of};
        assert_eq!(size_of::<VideoFrameV2>(), 72);
        assert_eq!(align_of::<VideoFrameV2>(), 8);
        assert_eq!(std::mem::offset_of!(VideoFrameV2, p_data), 40);
        assert_eq!(std::mem::offset_of!(VideoFrameV2, line_stride_in_bytes), 48);
    }

    /// A frame with a padded stride must be copied whole. Repacking rows
    /// here would cost a pass over every pixel for something the texture
    /// upload can express directly.
    #[test]
    fn a_padded_frame_is_copied_at_its_own_stride() {
        let slot = Arc::new(Slot::default());
        // 330 px wide: 1320 bytes of pixels, padded to 1408.
        let (w, h, stride) = (330i32, 4i32, 1408i32);
        let mut data = vec![0u8; (stride * h) as usize];
        data[0] = 7;
        data[(stride as usize) + 1] = 9;
        let video = VideoFrameV2 {
            xres: w,
            yres: h,
            p_data: data.as_mut_ptr(),
            line_stride_in_bytes: stride,
            ..Default::default()
        };
        copy_into(&slot, &video);

        let frame = slot.frame.lock().unwrap();
        assert_eq!(frame.width, 330);
        assert_eq!(frame.height, 4);
        assert_eq!(frame.stride, 1408, "stride was repacked");
        assert_eq!(frame.pixels.len(), (stride * h) as usize);
        assert_eq!(frame.pixels[0], 7);
        assert_eq!(frame.pixels[stride as usize + 1], 9, "rows landed at the wrong offset");
        assert_eq!(slot.revision.load(Ordering::Acquire), 1);
    }

    /// A sender that reports no stride at all must not produce a
    /// zero-length copy — some sources leave it unset for packed data.
    #[test]
    fn a_missing_stride_falls_back_to_packed_rows() {
        let slot = Arc::new(Slot::default());
        let mut data = vec![3u8; 64 * 4 * 2];
        let video = VideoFrameV2 {
            xres: 64,
            yres: 2,
            p_data: data.as_mut_ptr(),
            line_stride_in_bytes: 0,
            ..Default::default()
        };
        copy_into(&slot, &video);
        let frame = slot.frame.lock().unwrap();
        assert_eq!(frame.stride, 64 * 4);
        assert_eq!(frame.pixels.len(), 64 * 4 * 2);
    }

    /// A garbage frame must be ignored rather than turned into a huge
    /// allocation or a read through a null pointer.
    #[test]
    fn a_malformed_frame_is_ignored() {
        let slot = Arc::new(Slot::default());
        for video in [
            VideoFrameV2 { xres: 64, yres: 64, ..Default::default() },
            VideoFrameV2 { xres: -1, yres: 64, p_data: 1 as *mut u8, ..Default::default() },
            VideoFrameV2 { xres: 64, yres: 0, p_data: 1 as *mut u8, ..Default::default() },
        ] {
            copy_into(&slot, &video);
        }
        assert_eq!(slot.revision.load(Ordering::Acquire), 0, "a bad frame was accepted");
        assert!(slot.frame.lock().unwrap().pixels.is_empty());
    }

    /// Without a runtime, connecting must report every path it tried
    /// rather than a bare "not found" — the usual cause is the library
    /// being somewhere unusual, and the list is the fix.
    #[test]
    fn missing_runtime_reports_every_path_tried() {
        // SAFETY: single-threaded test process for this variable.
        unsafe { std::env::set_var("VIZZ_NDI_RUNTIME", "/nonexistent/libndi.so") };
        let err = sources(1).map(|_| ()).unwrap_err().to_string();
        unsafe { std::env::remove_var("VIZZ_NDI_RUNTIME") };
        assert!(err.contains("/nonexistent/libndi.so"), "path not reported: {err}");
        assert!(err.contains("VIZZ_NDI_RUNTIME"), "no hint about the override: {err}");
    }
}
