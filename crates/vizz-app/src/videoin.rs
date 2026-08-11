//! Where a video frame comes from.
//!
//! One trait over the sources, because everything downstream — the
//! texture upload, the cloud slot, the panel readout — wants a frame and
//! does not care who produced it. NDI is the real one; the test pattern
//! is here for a reason beyond testing, which is that "nothing is on
//! screen" has two causes and they need telling apart. Running
//! `--video-source test` puts a known picture through the identical path,
//! so a blank output afterwards is a wiring problem in vizz and a blank
//! output before it is the network or the sender.

use anyhow::Result;

/// A frame handed to the renderer: BGRA rows, `stride` bytes apart.
pub struct VideoFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bgra: &'a [u8],
}

pub trait VideoSource: Send {
    /// Name to show in the panel.
    fn label(&self) -> String;

    /// Whether frames are arriving.
    fn connected(&self) -> bool;

    /// A counter that changes when there is something new to upload, so a
    /// still source costs nothing per frame.
    fn revision(&self) -> u64;

    /// Hand the latest frame to `f`, if one can be had without waiting.
    fn with_latest(&self, f: &mut dyn FnMut(VideoFrame<'_>));
}

/// A live NDI input.
pub struct NdiSource(vizz_io::ndi_recv::NdiInput);

impl NdiSource {
    pub fn connect(needle: &str) -> Result<Self> {
        Ok(Self(vizz_io::ndi_recv::NdiInput::connect(needle)?))
    }
}

impl VideoSource for NdiSource {
    fn label(&self) -> String {
        let needle = self.0.source();
        if needle.is_empty() {
            "ndi: first source".to_string()
        } else {
            format!("ndi: {needle}")
        }
    }

    fn connected(&self) -> bool {
        self.0.connected()
    }

    fn revision(&self) -> u64 {
        self.0.revision()
    }

    fn with_latest(&self, f: &mut dyn FnMut(VideoFrame<'_>)) {
        self.0.with_latest(|frame| {
            f(VideoFrame {
                width: frame.width,
                height: frame.height,
                stride: frame.stride,
                bgra: &frame.pixels,
            })
        });
    }
}

/// A generated test pattern: colour bars with a moving sweep.
///
/// Deliberately not a still image. A static pattern proves a frame was
/// uploaded once; a moving one proves frames are still arriving, which is
/// the question anyone debugging a dead input is actually asking. The
/// relief modes have something to bite on too — the bars differ in hue
/// and in luminance, so switching `/video/relief` visibly changes the
/// shape rather than nudging it.
pub struct TestPattern {
    width: u32,
    height: u32,
    pixels: std::sync::Mutex<Vec<u8>>,
    start: std::time::Instant,
    revision: std::sync::atomic::AtomicU64,
}

impl TestPattern {
    pub fn new() -> Self {
        let (width, height) = (320, 180);
        Self {
            width,
            height,
            pixels: std::sync::Mutex::new(vec![0; (width * height * 4) as usize]),
            start: std::time::Instant::now(),
            revision: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Repaint for the current time. Called from the render thread, which
    /// is where a generated source's cost belongs — it is a few tens of
    /// thousands of pixels and no thread is worth the synchronisation.
    fn repaint(&self) {
        let t = self.start.elapsed().as_secs_f32();
        let Ok(mut px) = self.pixels.lock() else { return };
        // Eight bars, the classic order, so the hue relief has a ramp to
        // follow and the luminance relief has a staircase.
        const BARS: [[u8; 3]; 8] = [
            [255, 255, 255],
            [255, 255, 0],
            [0, 255, 255],
            [0, 255, 0],
            [255, 0, 255],
            [255, 0, 0],
            [0, 0, 255],
            [20, 20, 20],
        ];
        let sweep = (t * 0.25).fract() * self.width as f32;
        for y in 0..self.height {
            for x in 0..self.width {
                let bar = BARS[(x * 8 / self.width) as usize % 8];
                // A vertical gradient over the bars, so a frame has depth
                // variation within a bar as well as between bars.
                let shade = 1.0 - 0.6 * (y as f32 / self.height as f32);
                // The moving band, bright enough to read as motion in the
                // relief as well as in the colour.
                let d = (x as f32 - sweep).abs();
                let band = if d < 6.0 { 1.0 - d / 6.0 } else { 0.0 };
                let mix = |c: u8| {
                    let v = c as f32 * shade + 255.0 * band;
                    v.clamp(0.0, 255.0) as u8
                };
                let i = ((y * self.width + x) * 4) as usize;
                // BGRA, as the renderer's texture expects.
                px[i] = mix(bar[2]);
                px[i + 1] = mix(bar[1]);
                px[i + 2] = mix(bar[0]);
                px[i + 3] = 255;
            }
        }
    }
}

impl Default for TestPattern {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoSource for TestPattern {
    fn label(&self) -> String {
        "test pattern".to_string()
    }

    fn connected(&self) -> bool {
        true
    }

    /// Always new: the pattern moves, so every frame is worth uploading.
    fn revision(&self) -> u64 {
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(1)
    }

    fn with_latest(&self, f: &mut dyn FnMut(VideoFrame<'_>)) {
        self.repaint();
        let Ok(px) = self.pixels.lock() else { return };
        f(VideoFrame {
            width: self.width,
            height: self.height,
            stride: self.width * 4,
            bgra: &px,
        });
    }
}

/// A Syphon server on this Mac, received as frames.
#[cfg(target_os = "macos")]
pub struct SyphonSource(vizz_io::syphon_recv::SyphonInput);

#[cfg(target_os = "macos")]
impl SyphonSource {
    /// The device is the renderer's own: Syphon shares an IOSurface
    /// with it, and a texture created against a different device is not
    /// readable by the upload that follows.
    pub fn connect(device: &wgpu::Device, needle: &str) -> Result<Self> {
        Ok(Self(vizz_io::syphon_recv::SyphonInput::connect(device, needle)?))
    }
}

#[cfg(target_os = "macos")]
impl VideoSource for SyphonSource {
    fn label(&self) -> String {
        format!("syphon: {}", self.0.label())
    }

    fn connected(&self) -> bool {
        self.0.connected()
    }

    fn revision(&self) -> u64 {
        // Pumping here rather than on a thread: Syphon hands frames over
        // on whatever thread asks, and the render thread is the only one
        // that may touch the Metal device.
        self.0.pump();
        self.0.revision()
    }

    fn with_latest(&self, f: &mut dyn FnMut(VideoFrame<'_>)) {
        self.0.with_latest(|w, h, bgra| {
            f(VideoFrame { width: w, height: h, stride: w * 4, bgra })
        });
    }
}

/// What is available to receive from, right now.
///
/// Gathered on demand — when the panel's video section is opened or its
/// rescan is pressed — never per frame: NDI discovery blocks for its
/// announcement window, and enumerating capture devices wakes hardware.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sources {
    pub ndi: Vec<String>,
    pub syphon: Vec<String>,
    pub cameras: Vec<String>,
    /// Why a kind found nothing, when the reason is not "nothing is
    /// there" — no NDI runtime installed, no Syphon.framework. An empty
    /// list and a broken installation look identical otherwise, and the
    /// difference is the whole question when a feed will not appear.
    pub notes: Vec<String>,
}

/// How long to wait for NDI's asynchronous announcements. Long enough to
/// hear a sender that is already running, short enough that the panel
/// does not appear to hang when there is nothing on the network.
const NDI_DISCOVERY_MS: u32 = 600;

pub fn discover() -> Sources {
    let mut out = Sources::default();
    match vizz_io::ndi_recv::sources(NDI_DISCOVERY_MS) {
        Ok(names) => out.ndi = names,
        Err(e) => out.notes.push(format!("NDI: {e}")),
    }
    match syphon_servers() {
        Ok(names) => out.syphon = names,
        Err(e) => out.notes.push(format!("Syphon: {e}")),
    }
    match cameras() {
        Ok(names) => out.cameras = names,
        Err(e) => out.notes.push(format!("cameras: {e}")),
    }
    out
}

#[cfg(target_os = "macos")]
fn syphon_servers() -> Result<Vec<String>> {
    vizz_io::syphon_recv::servers()
}

#[cfg(not(target_os = "macos"))]
fn syphon_servers() -> Result<Vec<String>> {
    Ok(Vec::new())
}

/// Build a source from a spec.
///
/// The prefixed forms name a kind exactly — `ndi:`, `syphon:`,
/// `camera:` — and everything after the colon is matched as a substring,
/// the way `--audio-device` is, because full names carry the host and
/// nobody wants to type `STUDIO-PC (OBS)` exactly. `test` is the
/// built-in pattern.
///
/// A bare name with no prefix still means NDI, which is what
/// `--video-source` accepted before the other kinds existed; breaking
/// that would break every script and every note anyone has written.
pub fn open(spec: &str, device: Option<&wgpu::Device>) -> Result<Box<dyn VideoSource>> {
    let spec = spec.trim();
    if spec.eq_ignore_ascii_case("test") {
        return Ok(Box::new(TestPattern::new()));
    }
    if let Some(name) = spec.strip_prefix("syphon:") {
        let device = device.ok_or_else(|| {
            anyhow::anyhow!("Syphon input needs the renderer's GPU, which is not up yet")
        })?;
        return open_syphon(device, name.trim());
    }
    if let Some(name) = spec.strip_prefix("camera:") {
        return open_camera(name.trim());
    }
    let needle = spec.strip_prefix("ndi:").unwrap_or(spec).trim();
    let needle = if needle.eq_ignore_ascii_case("ndi") { "" } else { needle };
    Ok(Box::new(NdiSource::connect(needle)?))
}

#[cfg(target_os = "macos")]
fn open_syphon(device: &wgpu::Device, name: &str) -> Result<Box<dyn VideoSource>> {
    Ok(Box::new(SyphonSource::connect(device, name)?))
}

#[cfg(not(target_os = "macos"))]
fn open_syphon(_device: &wgpu::Device, _name: &str) -> Result<Box<dyn VideoSource>> {
    anyhow::bail!("Syphon input is macOS only")
}

/// A camera or capture card, on macOS.
///
/// The capture itself is `nokhwa`'s AVFoundation backend rather than
/// hand-written Objective-C. Camera capture is a delegate protocol, a
/// session lifecycle and half a dozen pixel formats to convert from; a
/// maintained crate that already handles all of it is a far better bet
/// than interop written blind, and this crate is only pulled in on
/// macOS so nothing else in the workspace grows a dependency.
///
/// Frames arrive on a worker thread and land in a mutex the render
/// thread reads. Same shape as the NDI receiver: the render thread must
/// never wait on a device.
#[cfg(target_os = "macos")]
pub struct CameraSource {
    name: String,
    latest: std::sync::Arc<std::sync::Mutex<Option<(u32, u32, Vec<u8>)>>>,
    revision: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Cleared on drop, which is how the worker learns to stop.
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(target_os = "macos")]
impl CameraSource {
    pub fn connect(needle: &str) -> Result<Self> {
        use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};

        let devices = nokhwa::query(ApiBackend::AVFoundation)
            .map_err(|e| anyhow::anyhow!("could not list cameras: {e}"))?;
        let found = devices
            .iter()
            .find(|d| needle.is_empty() || d.human_name().to_lowercase().contains(&needle.to_lowercase()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no camera matching {needle:?} — {} connected",
                    devices.len()
                )
            })?;
        let name = found.human_name();
        let index = found.index().clone();

        let latest = Arc::new(Mutex::new(None));
        let revision = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let (latest, revision, running) = (latest.clone(), revision.clone(), running.clone());
            std::thread::Builder::new()
                .name("camera".into())
                .spawn(move || {
                    // Whatever the device likes best: asking for a
                    // specific size is how a capture card that only does
                    // 1080i59.94 ends up refusing to open at all.
                    let format = RequestedFormat::new::<nokhwa::pixel_format::RgbAFormat>(
                        RequestedFormatType::AbsoluteHighestFrameRate,
                    );
                    let mut cam = match nokhwa::Camera::new(index, format) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = tx.send(Err(format!("{e}")));
                            return;
                        }
                    };
                    if let Err(e) = cam.open_stream() {
                        let _ = tx.send(Err(format!("{e}")));
                        return;
                    }
                    let _ = tx.send(Ok(()));
                    while running.load(Ordering::Relaxed) {
                        let Ok(frame) = cam.frame() else {
                            // A dropped frame is not a dead camera; a
                            // dead camera stops reporting connected
                            // because the revision stops moving.
                            std::thread::sleep(std::time::Duration::from_millis(5));
                            continue;
                        };
                        let Ok(rgba) = frame.decode_image::<nokhwa::pixel_format::RgbAFormat>()
                        else {
                            continue;
                        };
                        let (w, h) = (rgba.width(), rgba.height());
                        // The renderer wants BGRA; nokhwa decodes RGBA.
                        let mut bgra = rgba.into_raw();
                        for px in bgra.chunks_exact_mut(4) {
                            px.swap(0, 2);
                        }
                        if let Ok(mut slot) = latest.lock() {
                            *slot = Some((w, h, bgra));
                        }
                        revision.fetch_add(1, Ordering::Relaxed);
                    }
                    let _ = cam.stop_stream();
                })
                .map_err(|e| anyhow::anyhow!("could not start the camera thread: {e}"))?;
        }
        // Wait for the open to succeed or fail, so a bad pick is an error
        // the panel can show rather than a source that silently never
        // produces a frame.
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                running.store(false, Ordering::Relaxed);
                anyhow::bail!("could not open {name}: {e}");
            }
            Err(_) => {
                running.store(false, Ordering::Relaxed);
                anyhow::bail!("{name} did not start within five seconds");
            }
        }
        Ok(Self { name, latest, revision, running })
    }
}

#[cfg(target_os = "macos")]
impl Drop for CameraSource {
    fn drop(&mut self) {
        self.running
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(target_os = "macos")]
impl VideoSource for CameraSource {
    fn label(&self) -> String {
        format!("camera: {}", self.name)
    }

    fn connected(&self) -> bool {
        self.latest.lock().map(|l| l.is_some()).unwrap_or(false)
    }

    fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn with_latest(&self, f: &mut dyn FnMut(VideoFrame<'_>)) {
        let Ok(slot) = self.latest.lock() else { return };
        if let Some((w, h, bgra)) = slot.as_ref() {
            f(VideoFrame { width: *w, height: *h, stride: w * 4, bgra });
        }
    }
}

#[cfg(target_os = "macos")]
fn cameras() -> Result<Vec<String>> {
    let devices = nokhwa::query(nokhwa::utils::ApiBackend::AVFoundation)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(devices.iter().map(|d| d.human_name()).collect())
}

#[cfg(not(target_os = "macos"))]
fn cameras() -> Result<Vec<String>> {
    Ok(Vec::new())
}

#[cfg(target_os = "macos")]
fn open_camera(name: &str) -> Result<Box<dyn VideoSource>> {
    Ok(Box::new(CameraSource::connect(name)?))
}

#[cfg(not(target_os = "macos"))]
fn open_camera(_name: &str) -> Result<Box<dyn VideoSource>> {
    anyhow::bail!("camera capture is macOS only in this build")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundle must ask for the camera, in both of the two places
    /// macOS requires.
    ///
    /// A capture device needs `NSCameraUsageDescription` in the
    /// Info.plist *and* `com.apple.security.device.camera` in the
    /// entitlements once the hardened runtime is on — which it is,
    /// because notarization requires it. With either missing,
    /// AVFoundation refuses with "add new input: Rejected", an error
    /// that names the input rather than the permission and sends you
    /// looking at the camera. This shipped exactly that way.
    ///
    /// Checked by reading the build script rather than a built bundle,
    /// so it fails in every CI run on every platform, not only where an
    /// .app can be made.
    #[test]
    fn the_bundle_asks_for_camera_permission_both_ways() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let plist = std::fs::read_to_string(root.join("scripts/make-app.sh"))
            .expect("make-app.sh missing");
        assert!(
            plist.contains("NSCameraUsageDescription"),
            "the Info.plist has no camera usage description — capture will be refused"
        );
        assert!(
            plist.contains("NSMicrophoneUsageDescription"),
            "the Info.plist lost its microphone usage description"
        );
        // Receiving a live point cloud from a phone on the same wifi is
        // local network access, gated the same way since macOS 15 — and
        // it fails silently, which is worse than failing loudly.
        assert!(
            plist.contains("NSLocalNetworkUsageDescription"),
            "the Info.plist has no local network usage description — \
             receiving from a device on the wifi will silently do nothing"
        );
        let ents = std::fs::read_to_string(root.join("scripts/vizz.entitlements"))
            .expect("vizz.entitlements missing");
        assert!(
            ents.contains("com.apple.security.device.camera"),
            "the hardened runtime has no camera entitlement — capture will be refused"
        );
        assert!(
            ents.contains("com.apple.security.device.audio-input"),
            "the hardened runtime lost its audio-input entitlement"
        );
    }

    /// The pattern must be a well-formed frame, because it is the thing
    /// people will reach for to decide whether the *rest* of the path
    /// works. A malformed one would send them looking in the wrong place.
    #[test]
    fn the_test_pattern_is_a_well_formed_bgra_frame() {
        let p = TestPattern::new();
        let mut seen = 0;
        p.with_latest(&mut |f| {
            seen += 1;
            assert_eq!(f.stride, f.width * 4, "stride must match a packed frame");
            assert_eq!(
                f.bgra.len(),
                (f.width * f.height * 4) as usize,
                "buffer is not the size the dimensions claim"
            );
            assert!(
                f.bgra.chunks_exact(4).all(|p| p[3] == 255),
                "alpha must be opaque, or the picture arrives see-through"
            );
            assert!(
                f.bgra.chunks_exact(4).any(|p| p[..3] != [0, 0, 0]),
                "the pattern is entirely black, which is what it exists to rule out"
            );
        });
        assert_eq!(seen, 1, "with_latest did not hand over a frame");
    }

    /// `open` has to route by name, since getting this wrong means
    /// `--video-source test` tries to reach the network and fails on a
    /// machine with no NDI runtime — exactly when someone is using it to
    /// prove the runtime is not the problem.
    #[test]
    fn the_test_spec_never_touches_ndi() {
        let s = open("test", None).expect("the test pattern needs no runtime");
        assert_eq!(s.label(), "test pattern");
        assert!(s.connected());
    }
}
