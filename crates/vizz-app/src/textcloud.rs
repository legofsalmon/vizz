//! Words and pictures as point clouds.
//!
//! Both produce plain `Vec<Point>` for [`ParticleScene::set_cloud`], which
//! normalizes and resamples — so nothing in the render crate knows or
//! cares that a cloud started life as a typed word or a dropped JPEG.
//! The text path reuses the fonts egui already ships (they are in the
//! build either way), so a word costs no new dependency and no file.
//!
//! [`ParticleScene::set_cloud`]: vizz_render::particles::ParticleScene::set_cloud

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};
use vizz_render::pointcloud::Point;

/// Glyph height in raster pixels. Big enough that the coverage grid gives
/// tens of thousands of candidate cells for a short word; the subsampler
/// below takes it down to the slot budget.
const SCALE: f32 = 256.0;

/// Points to aim for. Under the slot's 65536 on purpose: the resampler
/// jitters repeats to fill, and undershooting keeps the letterforms
/// crisper than clipping overshoot would.
const TARGET: usize = 60_000;

/// A rasterized word, plus what it took to make it — the caller decides
/// whether "half the glyphs were tofu" is worth a warning.
pub struct TextCloud {
    pub points: Vec<Point>,
    /// Glyphs the font had no outline for.
    pub missing: usize,
    /// Glyphs attempted (whitespace excluded).
    pub glyphs: usize,
}

/// Rasterize a line of text into a point cloud.
///
/// Deterministic: the same string yields the same cloud, so a restored
/// session shows exactly what was on screen when it was saved.
pub fn rasterize(text: &str) -> TextCloud {
    // The panel's own body font. Shipped bytes — failing to parse them
    // would be a build defect, not a runtime condition.
    let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT)
        .expect("the shipped Ubuntu-Light parses");
    let scaled = font.as_scaled(PxScale::from(SCALE));

    let mut cells: Vec<[f32; 2]> = Vec::new();
    let (mut missing, mut glyphs) = (0usize, 0usize);
    let mut pen = 0.0f32;
    let mut prev = None;
    for c in text.chars() {
        if c.is_whitespace() {
            pen += scaled.h_advance(scaled.glyph_id(' '));
            prev = None;
            continue;
        }
        glyphs += 1;
        let id = scaled.glyph_id(c);
        if id.0 == 0 {
            missing += 1;
        }
        if let Some(p) = prev {
            pen += scaled.kern(p, id);
        }
        let glyph = id.with_scale_and_position(SCALE, ab_glyph::point(pen, SCALE));
        pen += scaled.h_advance(id);
        prev = Some(id);
        let Some(outlined) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outlined.px_bounds();
        outlined.draw(|x, y, coverage| {
            if coverage > 0.5 {
                cells.push([bounds.min.x + x as f32, bounds.min.y + y as f32]);
            }
        });
    }

    // Every covered cell down to the budget, evenly rather than randomly:
    // a strided take keeps letterform density uniform, and stays
    // deterministic for the restore path.
    let stride = cells.len() / TARGET + 1;
    let points = cells
        .iter()
        .step_by(stride)
        .map(|[x, y]| {
            // Raster y grows downward; the world's grows up. The z is a
            // shallow deterministic relief so depth-of-field and gravity
            // have something to hold — dead flat reads as a decal.
            let jitter = (x * 12.9898 + y * 78.233).sin() * SCALE * 0.03;
            Point::new(*x, -y, jitter)
        })
        .collect();
    TextCloud { points, missing, glyphs }
}

/// Sample a decoded image into a point cloud: x/y from position, colour
/// from the pixel, and a shallow luminance relief in z — enough that the
/// picture has a surface, not so much that it reads as noise.
pub fn image_points(image: &image::RgbaImage) -> Vec<Point> {
    // Down to the slot budget while keeping aspect: scale so the pixel
    // count fits, then let the resampler fill the remainder.
    let (w, h) = (image.width().max(1), image.height().max(1));
    let scale = (TARGET as f64 / (w as f64 * h as f64)).sqrt().min(1.0);
    let (tw, th) = (
        ((w as f64 * scale) as u32).max(1),
        ((h as f64 * scale) as u32).max(1),
    );
    let small = if (tw, th) == (w, h) {
        image.clone()
    } else {
        image::imageops::thumbnail(image, tw, th)
    };

    let mut points = Vec::with_capacity((tw * th) as usize);
    for (x, y, px) in small.enumerate_pixels() {
        let [r, g, b, a] = px.0;
        // Fully transparent pixels are not part of the picture — a
        // logo's alpha silhouette is the shape the cloud should take.
        if a < 8 {
            continue;
        }
        let lum = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
        let depth = (lum - 0.5) * 0.3 * tw as f32;
        points.push(Point {
            pos: [x as f32, -(y as f32), depth],
            // A word or a picture is a flat sheet standing in space, so
            // it faces the way a sheet faces. Left for the estimator
            // rather than asserted here: the relief pushes each point
            // along z by its own brightness, so the surface is not
            // actually flat and a hardcoded normal would be a lie about
            // exactly the part that is interesting.
            normal: [0.0; 3],
            color: [r, g, b],
        });
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word yields a real cloud: thousands of points, inside a sane
    /// box, and byte-identical between calls so a restored session shows
    /// exactly what was saved.
    #[test]
    fn a_word_rasterizes_deterministically() {
        let a = rasterize("VIZZ");
        assert!(a.points.len() > 1000, "only {} points", a.points.len());
        assert!(a.points.len() <= 65536, "over the slot budget");
        assert_eq!(a.missing, 0, "the shipped font is missing latin glyphs?");
        for p in &a.points {
            assert!(p.pos[0].abs() < SCALE * 8.0 && p.pos[1].abs() < SCALE * 2.0);
        }
        let b = rasterize("VIZZ");
        assert_eq!(a.points, b.points, "the same word made two different clouds");
    }

    /// Letterforms are actually letterforms: a wide glyph covers more
    /// ground than a thin one, and an empty string is an empty cloud.
    #[test]
    fn glyph_shapes_survive_into_the_cloud() {
        let width = |t: &str| {
            let c = rasterize(t);
            let xs: Vec<f32> = c.points.iter().map(|p| p.pos[0]).collect();
            xs.iter().fold(f32::MIN, |a, b| a.max(*b)) - xs.iter().fold(f32::MAX, |a, b| a.min(*b))
        };
        assert!(width("W") > width("I") * 2.0, "W is not wider than I");
        assert!(rasterize("").points.is_empty());
        assert!(rasterize("   ").points.is_empty(), "whitespace made ink");
    }

    /// Tofu is counted, not hidden: the caller warns when the font could
    /// not draw most of what was typed.
    #[test]
    fn missing_glyphs_are_counted() {
        let c = rasterize("héllo");
        assert_eq!(c.glyphs, 5);
        assert_eq!(c.missing, 0, "Ubuntu-Light covers latin-1");
    }

    /// Image sampling keeps colour and shape, skips transparent pixels,
    /// and lands under the slot budget whatever the input size.
    #[test]
    fn an_image_samples_to_coloured_points() {
        let mut img = image::RgbaImage::new(4, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        img.put_pixel(2, 0, image::Rgba([0, 0, 255, 255]));
        // The rest stay transparent and must not appear.
        let pts = image_points(&img);
        assert_eq!(pts.len(), 3, "transparent pixels leaked in");
        assert_eq!(pts[0].color, [255, 0, 0]);
        assert_eq!(pts[1].color, [0, 255, 0]);

        // A large image downsamples under the budget.
        let big = image::RgbaImage::from_pixel(4000, 3000, image::Rgba([9, 9, 9, 255]));
        let pts = image_points(&big);
        assert!(pts.len() <= 65536, "{} points from a 12 MP image", pts.len());
        assert!(pts.len() > 30_000, "downsample overshot: {}", pts.len());
    }
}

#[cfg(test)]
mod plot_tests {
    /// Not an assertion — writes the raster to a PNG when asked, so the
    /// letterforms can be checked by eye: `VIZZ_DUMP_TEXTCLOUD=/tmp/x.png
    /// cargo test -p vizz-app plot_the_word -- --ignored`.
    #[test]
    #[ignore = "by-eye tool, run with VIZZ_DUMP_TEXTCLOUD set"]
    fn plot_the_word() {
        let Ok(path) = std::env::var("VIZZ_DUMP_TEXTCLOUD") else { return };
        let cloud = super::rasterize("VIZZ");
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in &cloud.points {
            for i in 0..2 {
                lo[i] = lo[i].min(p.pos[i]);
                hi[i] = hi[i].max(p.pos[i]);
            }
        }
        let (w, h) = (900u32, 320u32);
        let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([12, 14, 18, 255]));
        for p in &cloud.points {
            let x = ((p.pos[0] - lo[0]) / (hi[0] - lo[0]) * (w - 20) as f32) as u32 + 10;
            let y = ((p.pos[1] - lo[1]) / (hi[1] - lo[1]) * (h - 20) as f32) as u32 + 10;
            img.put_pixel(x.min(w - 1), h - 1 - y.min(h - 1), image::Rgba([235, 240, 245, 255]));
        }
        img.save(path).unwrap();
    }
}
