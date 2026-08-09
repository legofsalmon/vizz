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

/// Build a source from what the user asked for on the command line.
///
/// `test` is the built-in pattern; anything else is matched against NDI
/// source names as a substring, the way `--audio-device` is, because
/// full NDI names carry the host and nobody wants to type
/// `STUDIO-PC (OBS)` exactly.
pub fn open(spec: &str) -> Result<Box<dyn VideoSource>> {
    if spec.eq_ignore_ascii_case("test") {
        return Ok(Box::new(TestPattern::new()));
    }
    let needle = if spec.eq_ignore_ascii_case("ndi") { "" } else { spec };
    Ok(Box::new(NdiSource::connect(needle)?))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let s = open("test").expect("the test pattern needs no runtime");
        assert_eq!(s.label(), "test pattern");
        assert!(s.connected());
    }
}
