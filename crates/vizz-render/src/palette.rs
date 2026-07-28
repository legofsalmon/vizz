//! The palette bank: colour ramps the shader reads by index.
//!
//! Palettes used to be four cosine gradients written directly into the
//! shader — `a + b*cos(TAU*(c*t + d))`, four coefficients each. That is a
//! lovely way to *describe* a ramp and a hopeless way to let someone bring
//! their own, because the coefficients are not a colour picker and nobody
//! exports them. Every palette anyone actually has is a list of colours.
//!
//! So the ramps live in a texture instead. One row per palette, 256 texels
//! across, and the shader reads it by index. The built-ins are baked by
//! evaluating exactly the same cosine formula on the CPU, so the shipped
//! looks are unchanged; a loaded palette is resampled into the same row
//! shape and is then indistinguishable from a built-in as far as the
//! renderer is concerned. Crossfading between palettes keeps working
//! because it is still just a mix between two rows.
//!
//! Read with `textureLoad` and interpolated by hand rather than with a
//! sampler. The lookup happens in the *vertex* stage, where filtered
//! sampling is the restricted path and a plain load is not — and it saves
//! a bind-group entry.

use anyhow::{Context as _, Result, bail};

use crate::GpuContext;

/// Rows in the bank. Four built-ins and room for a set of your own; the
/// texture is allocated once and indexed by row, so this cannot grow at
/// runtime without rebuilding every bind group mid-frame.
pub const PALETTES: usize = 16;

/// Texels across one ramp. Enough that a 256-step gradient is exact and
/// anything smoother is imperceptible once it is scattered over particles.
pub const LUT_W: usize = 256;

/// The first row a loaded palette may occupy.
///
/// Rows 1..=4 are the shipped gradients and rows are what `/color/palette`
/// indexes, so they are fixed forever: a preset saved with palette 3 must
/// still be "ice" in every future build, and a saved patch is the one
/// thing that cannot be migrated after the fact.
pub const FIRST_USER: usize = 5;

pub struct Palettes {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// A name per row, for the UI. Empty means the row is unused.
    pub names: Vec<String>,
}

impl Palettes {
    pub fn new(ctx: &GpuContext) -> Self {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("palette-bank"),
            size: wgpu::Extent3d {
                width: LUT_W as u32,
                height: PALETTES as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Unorm rather than sRGB: the shader works in linear light and
            // does its own tone-mapping, so a gamma-encoded fetch here
            // would brighten every palette by roughly a stop.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut names = vec![String::new(); PALETTES];

        // Row 0 is never read — palette 0 is the original HSV colouring and
        // stays procedural, so `/particles/hue` keeps meaning what it did.
        names[0] = "hsv".into();
        for (i, (name, coeffs)) in BUILTINS.iter().enumerate() {
            let row = i + 1;
            names[row] = (*name).into();
            write_row(ctx, &texture, row, &bake_cosine(coeffs));
        }
        Self {
            texture,
            view,
            names,
        }
    }

    /// Replace one row with a ramp built from colour stops.
    pub fn load_slot(&mut self, ctx: &GpuContext, row: usize, stops: &[[f32; 3]], name: &str) {
        if row >= PALETTES || stops.is_empty() {
            return;
        }
        write_row(ctx, &self.texture, row, &resample(stops));
        self.names[row] = name.to_string();
    }

    /// Highest row that has actually been written.
    ///
    /// The parameter's range covers the whole bank, but rows past the
    /// built-ins are empty until a palette is dropped — and an empty row
    /// reads back as zeroed texels, so sweeping into one faded the field
    /// to black and held it there. The shader clamps to this instead, so
    /// the control saturates on the last real palette.
    pub fn occupied(&self) -> usize {
        self.names
            .iter()
            .rposition(|n| !n.is_empty())
            .unwrap_or(0)
    }

    /// The next free user row, wrapping once they are all taken. Wrapping
    /// rather than refusing: a full bank should keep accepting palettes,
    /// and the oldest is the one you are least likely to still want.
    pub fn next_user_row(&self, loaded: usize) -> usize {
        FIRST_USER + loaded % (PALETTES - FIRST_USER)
    }
}

/// Inigo Quilez cosine gradients: `color = a + b*cos(TAU*(c*t + d))`.
/// The exact coefficients the shader used, so the shipped palettes are the
/// same colours they have always been.
type Coeffs = [[f32; 3]; 4];
const BUILTINS: &[(&str, Coeffs)] = &[
    (
        "warm",
        [
            [0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [1.0, 1.0, 1.0],
            [0.0, 0.33, 0.67],
        ],
    ),
    (
        "ember",
        [
            [0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [1.0, 1.0, 1.0],
            [0.0, 0.10, 0.20],
        ],
    ),
    (
        "ice",
        [
            [0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [1.0, 1.0, 0.5],
            [0.8, 0.90, 0.30],
        ],
    ),
    (
        "neon",
        [
            [0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [2.0, 1.0, 0.0],
            [0.5, 0.20, 0.25],
        ],
    ),
];

fn bake_cosine(c: &Coeffs) -> Vec<u8> {
    const TAU: f32 = std::f32::consts::TAU;
    let mut out = Vec::with_capacity(LUT_W * 4);
    for i in 0..LUT_W {
        let t = i as f32 / LUT_W as f32;
        // Indexed rather than iterated: the channel indexes several
        // parallel arrays at once, and zipping them reads worse.
        #[allow(clippy::needless_range_loop)]
        for ch in 0..3 {
            let v = c[0][ch] + c[1][ch] * (TAU * (c[2][ch] * t + c[3][ch])).cos();
            out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        out.push(255);
    }
    out
}

/// Spread arbitrary stops evenly across the ramp and interpolate between
/// them.
///
/// Evenly rather than at authored positions because the formats people
/// actually export — a row of swatches from Coolors, a Lospec hex list —
/// carry no positions at all. A five-colour palette becomes five equal
/// bands blended together, which is what those sites show you.
fn resample(stops: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(LUT_W * 4);
    let n = stops.len();
    for i in 0..LUT_W {
        let t = i as f32 / (LUT_W - 1) as f32;
        // Wrap rather than clamp at the end: the drive value wraps, so a
        // ramp that does not loop shows a hard seam once per revolution.
        let scaled = t * n as f32;
        let a = (scaled as usize) % n;
        let b = (a + 1) % n;
        let f = scaled - scaled.floor();
        // Indexed rather than iterated: the channel indexes several
        // parallel arrays at once, and zipping them reads worse.
        #[allow(clippy::needless_range_loop)]
        for ch in 0..3 {
            let v = stops[a][ch] + (stops[b][ch] - stops[a][ch]) * f;
            out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        out.push(255);
    }
    out
}

fn write_row(ctx: &GpuContext, texture: &wgpu::Texture, row: usize, data: &[u8]) {
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: row as u32,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some((LUT_W * 4) as u32),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: LUT_W as u32,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

/// Read a palette file into colour stops.
///
/// Handles the formats palette sites actually export, because a palette
/// format nobody exports is a palette format nobody uses:
///
/// - a list of hex colours, one per line or comma-separated, with or
///   without `#` — what Coolors, Lospec and every "copy palette" button
///   produce;
/// - GIMP `.gpl`, which is what Inkscape, Krita and Aseprite export and
///   what most palette archives are stored as.
///
/// Anything unparseable on a line is skipped rather than failing the file:
/// these formats are full of comments, names and stray blank lines, and
/// refusing a palette because line nine had a word on it would be
/// unhelpful.
pub fn parse(path: &std::path::Path) -> Result<(Vec<[f32; 3]>, String)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let mut stops = Vec::new();
    let mut name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("palette")
        .to_string();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // A `.gpl` names itself, which beats the filename.
        if let Some(rest) = line.strip_prefix("Name:") {
            let rest = rest.trim();
            if !rest.is_empty() {
                name = rest.to_string();
            }
            continue;
        }
        if line.eq_ignore_ascii_case("GIMP Palette") || line.starts_with("Columns:") {
            continue;
        }
        // Comment lines need no special case: `#` is also the hex prefix,
        // and anything that is not a colour fails both parsers below and
        // is skipped. Trying to detect comments by their leading `#` is
        // what breaks a comma-separated row of hex codes, which begins
        // with exactly the same character.
        for token in line.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Some(c) = parse_hex(token) {
                stops.push(c);
            } else if let Some(c) = parse_triplet(token) {
                stops.push(c);
            }
        }
    }

    if stops.is_empty() {
        bail!(
            "{} has no colours I could read — expected hex codes or a GIMP palette",
            path.display()
        );
    }
    // More than this and the ramp is finer than the texture, so the extra
    // stops are averaged away anyway.
    stops.truncate(LUT_W);
    Ok((stops, name))
}

fn parse_hex(token: &str) -> Option<[f32; 3]> {
    let t = token.trim_start_matches('#');
    // Byte length is only a char count for ASCII, and the slicing below
    // indexes bytes. A palette file with a smart quote, an em dash or a
    // BOM — all of which these formats are full of — would otherwise
    // panic on a char boundary and take the app down, and because
    // palettes reload at startup it would then do it on every launch.
    if !t.is_ascii() {
        return None;
    }
    let (r, g, b) = match t.len() {
        6 => (
            u8::from_str_radix(&t[0..2], 16).ok()?,
            u8::from_str_radix(&t[2..4], 16).ok()?,
            u8::from_str_radix(&t[4..6], 16).ok()?,
        ),
        // Short form, where each digit is doubled: fff is white.
        3 => {
            let d = |i: usize| u8::from_str_radix(&t[i..i + 1], 16).ok().map(|v| v * 17);
            (d(0)?, d(1)?, d(2)?)
        }
        _ => return None,
    };
    Some(srgb_to_linear([r, g, b]))
}

/// `R G B` on one line, which is how `.gpl` stores its entries — often
/// followed by a name, which is ignored.
fn parse_triplet(token: &str) -> Option<[f32; 3]> {
    let mut it = token.split_whitespace();
    let r: u16 = it.next()?.parse().ok()?;
    let g: u16 = it.next()?.parse().ok()?;
    let b: u16 = it.next()?.parse().ok()?;
    if r > 255 || g > 255 || b > 255 {
        return None;
    }
    Some(srgb_to_linear([r as u8, g as u8, b as u8]))
}

/// Palette files are authored in sRGB — they came out of a colour picker —
/// and the renderer works in linear light. Skipping this makes every
/// loaded palette noticeably paler than the same colours look in the tool
/// they were chosen in, which reads as the import being subtly broken.
fn srgb_to_linear(c: [u8; 3]) -> [f32; 3] {
    let f = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(c[0]), f(c[1]), f(c[2])]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write(name: &str, body: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("vizz-pal-{}-{name}", std::process::id()));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p
    }

    /// The formats palette sites actually export. A hex list is what every
    /// "copy" button produces; `.gpl` is what the desktop tools write.
    #[test]
    fn reads_the_formats_people_actually_have() {
        let p = write("coolors.hex", "#264653\n2a9d8f\nE9C46A\n#f4a261\n#E76F51\n");
        let (stops, name) = parse(&p).unwrap();
        assert_eq!(stops.len(), 5, "{stops:?}");
        // A bare hex list names nothing, so the filename is the name.
        assert!(name.ends_with("coolors"), "got {name}");

        let p = write(
            "krita.gpl",
            "GIMP Palette\nName: Warehouse\nColumns: 4\n#\n\
             38 70 83 Dark blue\n42 157 143 Teal\n233 196 106 Sand\n",
        );
        let (stops, name) = parse(&p).unwrap();
        assert_eq!(stops.len(), 3, "{stops:?}");
        assert_eq!(name, "Warehouse", "the palette's own name beats the filename");

        // Comma-separated on one line, the other thing "copy" gives you.
        let p = write("row.txt", "#264653, #2a9d8f, #e9c46a");
        assert_eq!(parse(&p).unwrap().0.len(), 3);
    }

    /// A file with nothing readable in it must say so, not load a black
    /// palette that looks like the feature silently failing.
    #[test]
    fn a_file_with_no_colours_is_an_error() {
        let p = write("notes.txt", "just some words\nand more words\n");
        assert!(parse(&p).is_err());
    }

    /// Colours are authored in sRGB and the renderer works in linear
    /// light. Getting this wrong makes every imported palette pale
    /// compared to the tool it was picked in, which reads as a broken
    /// import rather than a colour-space slip.
    #[test]
    fn colours_are_converted_out_of_srgb() {
        let p = write("mid.hex", "#808080\n");
        let c = parse(&p).unwrap().0[0];
        // Mid grey in sRGB is about 0.216 linear, not 0.5.
        assert!(
            (c[0] - 0.216).abs() < 0.01,
            "sRGB 0x80 became {} rather than ~0.216",
            c[0]
        );
        // And the endpoints still land exactly where they should.
        let p = write("ends.hex", "#000000\n#ffffff\n");
        let s = parse(&p).unwrap().0;
        assert_eq!(s[0], [0.0, 0.0, 0.0]);
        assert!((s[1][0] - 1.0).abs() < 1e-6);
    }

    /// The ramp has to loop. The drive value wraps, so a palette whose two
    /// ends do not meet shows a hard seam once per revolution — most
    /// visible on exactly the slow gradients palettes are chosen for.
    #[test]
    fn the_ramp_wraps_rather_than_ending() {
        let data = resample(&[[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        let first = &data[0..3];
        let last = &data[(LUT_W - 1) * 4..(LUT_W - 1) * 4 + 3];
        // The last texel is nearly back at the first, not stranded at the
        // far end of the ramp.
        let gap: i32 = first
            .iter()
            .zip(last)
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .sum();
        assert!(gap < 40, "ramp does not loop: {first:?} vs {last:?}");
    }

    /// Baking must not change the shipped palettes. They are what every
    /// existing preset and every built-in look was tuned against.
    #[test]
    fn the_builtins_bake_to_their_own_formula() {
        for (name, coeffs) in BUILTINS {
            let baked = bake_cosine(coeffs);
            assert_eq!(baked.len(), LUT_W * 4, "{name}");
            for i in [0usize, 64, 191] {
                let t = i as f32 / LUT_W as f32;
                let expect = coeffs[0][0]
                    + coeffs[1][0] * (std::f32::consts::TAU * (coeffs[2][0] * t + coeffs[3][0])).cos();
                let got = baked[i * 4] as f32 / 255.0;
                assert!(
                    (got - expect.clamp(0.0, 1.0)).abs() < 0.01,
                    "{name} at {t}: baked {got}, formula {expect}"
                );
            }
        }
    }
}
