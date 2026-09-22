//! A picture of a look, small enough to sit on the button that fires it.
//!
//! A preset is a list of numbers. Read out loud that list is meaningless,
//! and a name only means something to the person who typed it — three
//! months later "warehouse 2am" is a guess. So each look keeps a picture
//! of itself: what was on the master output at the moment it was saved,
//! shrunk to something a row of buttons can carry.
//!
//! These are a cache, not part of the preset. They live in their own
//! directory beside the presets rather than next to each JSON file, so
//! nothing that lists presets has to learn to skip them, and a built-in —
//! which has no file at all — can still have a picture. Deleting the lot
//! costs nothing but the next look at each preset.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

/// The widest a thumbnail is stored. Enough to read a composition at the
/// size a button draws it, and small enough that a library of two hundred
/// is a few megabytes rather than a few hundred.
pub const MAX_W: u32 = 128;
/// The tallest. A 16:9 master lands at 128x72; a square one at 72x72 —
/// see [`fit`], which never stretches.
pub const MAX_H: u32 = 72;

/// A decoded thumbnail: tight RGBA, top row first.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, RGBA, opaque.
    pub rgba: Vec<u8>,
}

/// Where thumbnails live: inside the show's preset directory, so copying
/// a show copies its pictures, and clearing them is one folder.
pub fn dir() -> PathBuf {
    crate::preset::preset_dir().join("thumbs")
}

pub fn path_for(name: &str) -> PathBuf {
    dir().join(format!("{}.png", crate::library::sanitize(name)))
}

pub fn exists(name: &str) -> bool {
    path_for(name).exists()
}

/// Write `thumb` as the picture of `name`.
///
/// Through a temporary file and a rename, like every other write in the
/// library: a half-written PNG is a decode error on every future draw,
/// and the point of a cache is that it cannot make things worse.
pub fn save(name: &str, thumb: &Thumb) -> Result<()> {
    if thumb.width == 0 || thumb.height == 0 {
        bail!("a thumbnail needs a non-zero size, got {}x{}", thumb.width, thumb.height);
    }
    let dir = dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = path_for(name);
    let tmp = crate::library::tmp_path(&path);
    {
        let file = std::fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        let out = std::io::BufWriter::new(file);
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(out),
            &thumb.rgba,
            thumb.width,
            thumb.height,
            image::ExtendedColorType::Rgba8,
        )
        .with_context(|| format!("encoding {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// The picture of `name`, or `None` when there is not one.
///
/// A missing picture and an unreadable one are the same answer on
/// purpose: the caller draws the fallback either way, and a corrupt cache
/// entry must not be an error a performer has to deal with mid-set. The
/// unreadable case is logged so it is still discoverable.
pub fn read(name: &str) -> Option<Thumb> {
    let path = path_for(name);
    let bytes = std::fs::read(&path).ok()?;
    match image::load_from_memory_with_format(&bytes, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            Some(Thumb {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            })
        }
        Err(e) => {
            log::warn!("thumbnail {} could not be read: {e}", path.display());
            None
        }
    }
}

/// Forget the picture of `name`. Silent when there is not one.
pub fn remove(name: &str) {
    let path = path_for(name);
    if path.exists() && let Err(e) = std::fs::remove_file(&path) {
        log::warn!("could not remove {}: {e}", path.display());
    }
}

/// The stored size for a master of `width` x `height`.
///
/// Fitted inside [`MAX_W`] x [`MAX_H`] rather than forced to them: a
/// performer can set a square or a tall master, and a thumbnail that
/// stretched it would show a composition nobody is sending. Never
/// upscales — a master smaller than the box is stored as it is.
pub fn fit(width: u32, height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let scale = (MAX_W as f32 / width as f32)
        .min(MAX_H as f32 / height as f32)
        .min(1.0);
    (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    )
}

/// Shrink a BGRA readback of the master into a thumbnail.
///
/// `stride` is bytes per row *including* the copy-alignment padding, as
/// the readback ring hands it over; the trailing bytes of each row are
/// junk and are never read.
///
/// Every source pixel inside an output pixel's footprint is averaged, so
/// a field of single-pixel particles survives the shrink as a haze rather
/// than as whichever pixels a nearest-neighbour pick happened to land on
/// — at 1/15 scale that difference is the whole picture.
///
/// The average is taken on the stored bytes, which are sRGB. That is not
/// the physically correct way to resample, and it is the right one here:
/// a linear average of one bright particle in two hundred dark pixels is
/// black, and the look being described is made of bright particles in the
/// dark.
///
/// Returns `None` if the buffer is smaller than `stride * height` says it
/// should be, rather than reading past it.
pub fn from_bgra(bytes: &[u8], width: u32, height: u32, stride: u32) -> Option<Thumb> {
    let (tw, th) = fit(width, height);
    if tw == 0 || th == 0 {
        return None;
    }
    if (stride as usize) < (width as usize) * 4 {
        return None;
    }
    if bytes.len() < (stride as usize) * (height as usize) {
        return None;
    }
    let mut rgba = vec![0u8; (tw as usize) * (th as usize) * 4];
    for y in 0..th {
        let y0 = (y as u64 * height as u64 / th as u64) as u32;
        let y1 = ((y as u64 + 1) * height as u64 / th as u64).max(y0 as u64 + 1) as u32;
        let y1 = y1.min(height);
        for x in 0..tw {
            let x0 = (x as u64 * width as u64 / tw as u64) as u32;
            let x1 = ((x as u64 + 1) * width as u64 / tw as u64).max(x0 as u64 + 1) as u32;
            let x1 = x1.min(width);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for sy in y0..y1 {
                let row = (sy as usize) * (stride as usize);
                for sx in x0..x1 {
                    let i = row + (sx as usize) * 4;
                    b += bytes[i] as u32;
                    g += bytes[i + 1] as u32;
                    r += bytes[i + 2] as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = ((y as usize) * (tw as usize) + x as usize) * 4;
            rgba[o] = (r / n) as u8;
            rgba[o + 1] = (g / n) as u8;
            rgba[o + 2] = (b / n) as u8;
            // Opaque. The master's alpha is whatever the last pass left
            // there, and a thumbnail is a picture rather than a key —
            // inheriting it drew half the library as a faint smear.
            rgba[o + 3] = 255;
        }
    }
    Some(Thumb { width: tw, height: th, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BGRA buffer of `width` x `height` with alignment padding, whose
    /// left half is one colour and right half another.
    fn split(width: u32, height: u32, stride: u32, left: [u8; 3], right: [u8; 3]) -> Vec<u8> {
        let mut bytes = vec![0u8; (stride * height) as usize];
        for y in 0..height {
            for x in 0..width {
                let c = if x < width / 2 { left } else { right };
                let i = (y * stride + x * 4) as usize;
                // BGRA, and a deliberately transparent alpha so the
                // opaque-output claim is actually being tested.
                bytes[i..i + 4].copy_from_slice(&[c[2], c[1], c[0], 0]);
            }
        }
        bytes
    }

    /// The reason the box filter exists.
    ///
    /// A 1080p master shrinks by fifteen, so every output pixel stands
    /// for 225 source ones — and a particle field is mostly dark with
    /// single bright pixels in it. Nearest-neighbour keeps such a pixel
    /// only if the sample happens to land on it, which is 224 times out
    /// of 225 a picture of an empty room. The average always keeps a
    /// trace. Probed by replacing the average with the top-left sample of
    /// each cell, which lit 0 of 100 particles instead of 100.
    #[test]
    fn single_bright_pixels_survive_a_fifteen_fold_shrink() {
        let (w, h, stride) = (1920u32, 1080u32, 1920 * 4);
        let mut bytes = vec![0u8; (stride * h) as usize];
        // A hundred particles, spread so no two share an output pixel and
        // none sits on a cell corner.
        for n in 0..100u32 {
            let (x, y) = (7 + n * 19, 5 + n * 10);
            let i = ((y * stride) + x * 4) as usize;
            bytes[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        let thumb = from_bgra(&bytes, w, h, stride).expect("a thumbnail");
        let lit = thumb.rgba.chunks_exact(4).filter(|p| p[0] > 0).count();
        assert_eq!(lit, 100, "{lit} of 100 particles made it into the picture");
    }

    /// Channels are not swapped on the way through. BGRA in, RGBA out.
    #[test]
    fn a_red_frame_reads_back_as_red() {
        let (w, h, stride) = (64u32, 36u32, 64 * 4);
        let bytes = split(w, h, stride, [255, 0, 0], [255, 0, 0]);
        let thumb = from_bgra(&bytes, w, h, stride).expect("a thumbnail");
        assert_eq!(&thumb.rgba[..4], &[255, 0, 0, 255], "red came back as something else");
    }

    /// The left half stays on the left. A stride bug or a transposed loop
    /// would still produce a plausible-looking picture, so the halves are
    /// different colours and both ends are checked.
    #[test]
    fn the_halves_do_not_swap() {
        let (w, h, stride) = (96u32, 54u32, 96 * 4 + 64);
        let bytes = split(w, h, stride, [255, 0, 0], [0, 0, 255]);
        let thumb = from_bgra(&bytes, w, h, stride).expect("a thumbnail");
        let px = |x: u32, y: u32| {
            let o = ((y * thumb.width + x) * 4) as usize;
            [thumb.rgba[o], thumb.rgba[o + 1], thumb.rgba[o + 2]]
        };
        assert_eq!(px(0, 0), [255, 0, 0], "the left edge is not the left colour");
        assert_eq!(
            px(thumb.width - 1, thumb.height - 1),
            [0, 0, 255],
            "the right edge is not the right colour"
        );
    }

    /// A master's shape is kept. A 1:1 master squeezed into 16:9 would
    /// show a composition nobody is sending.
    #[test]
    fn a_square_master_stays_square() {
        assert_eq!(fit(1080, 1080), (72, 72));
        assert_eq!(fit(1920, 1080), (128, 72));
        assert_eq!(fit(1080, 1920), (41, 72));
        // Already smaller than the box: stored as it is, not blown up.
        assert_eq!(fit(64, 36), (64, 36));
    }

    /// A short buffer is refused rather than read past.
    #[test]
    fn a_short_buffer_is_refused() {
        let stride = 64 * 4;
        let bytes = vec![0u8; (stride * 20) as usize];
        assert!(from_bgra(&bytes, 64, 36, stride).is_none());
        assert!(from_bgra(&bytes, 0, 36, stride).is_none());
    }

    /// The master's alpha does not come along. See the comment on the
    /// write: inheriting it drew the library as a smear.
    #[test]
    fn a_thumbnail_is_opaque() {
        let (w, h, stride) = (64u32, 36u32, 64 * 4);
        let bytes = split(w, h, stride, [10, 20, 30], [40, 50, 60]);
        let thumb = from_bgra(&bytes, w, h, stride).expect("a thumbnail");
        assert!(thumb.rgba.chunks_exact(4).all(|p| p[3] == 255));
    }

    /// Round trip through the disk, in a scoped show directory.
    #[test]
    fn a_saved_picture_reads_back_the_same() {
        let (_guard, _tmp) = crate::test_env::scoped("thumb-round-trip");
        let (w, h, stride) = (64u32, 36u32, 64 * 4);
        let bytes = split(w, h, stride, [200, 30, 30], [30, 30, 200]);
        let thumb = from_bgra(&bytes, w, h, stride).expect("a thumbnail");
        assert!(!exists("night bus"));
        save("night bus", &thumb).expect("saving");
        assert!(exists("night bus"));
        let back = read("night bus").expect("reading back");
        assert_eq!(back, thumb, "the picture changed on the way through PNG");
        remove("night bus");
        assert!(!exists("night bus"));
        assert!(read("night bus").is_none());
    }

    /// A picture is found under the same name a preset is saved under,
    /// however the name is punctuated. Both go through `sanitize`, and if
    /// they ever stopped agreeing every thumbnail would be invisible with
    /// nothing failing.
    #[test]
    fn a_picture_is_filed_under_the_preset_s_own_name() {
        let (_guard, _tmp) = crate::test_env::scoped("thumb-naming");
        let name = "warehouse 2am / take 3";
        let preset_stem = crate::library::sanitize(name);
        let thumb_stem = path_for(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(String::from)
            .expect("a stem");
        assert_eq!(thumb_stem, preset_stem);
    }

    /// Junk in the cache is a missing picture, not a failure.
    #[test]
    fn an_unreadable_picture_reads_as_no_picture() {
        let (_guard, _tmp) = crate::test_env::scoped("thumb-corrupt");
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(path_for("half written"), b"not a png").unwrap();
        assert!(exists("half written"), "the file is there");
        assert!(read("half written").is_none(), "and it did not decode");
    }
}
