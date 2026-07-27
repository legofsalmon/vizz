//! Reading point clouds from files.
//!
//! Supports PLY (ASCII and binary little-endian) and plain XYZ/CSV, which
//! between them covers what photogrammetry tools, LiDAR exports and
//! scanners actually emit.
//!
//! Parsing is separated from GPU upload so it can be tested against
//! handwritten fixtures — malformed headers and truncated bodies are the
//! normal condition for files that came off someone else's scanner, and
//! they must produce an error rather than a panic or a silent empty cloud.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context as _, Result, bail};

/// One point: position plus optional colour, packed the way the GPU wants
/// it. Colour defaults to white so an uncoloured cloud takes the palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub pos: [f32; 3],
    pub color: [u8; 3],
}

impl Point {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { pos: [x, y, z], color: [255, 255, 255] }
    }
}

/// Refuse rather than exhaust memory on a file that claims a billion
/// points. Ten million is far past anything renderable at 60 fps.
const MAX_POINTS: usize = 10_000_000;

pub fn load(path: &Path) -> Result<Vec<Point>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let points = match ext.as_str() {
        "ply" => read_ply(&mut reader),
        "xyz" | "csv" | "txt" | "pts" => read_xyz(&mut reader),
        other => bail!("unsupported point cloud format {other:?} (want .ply, .xyz, .csv or .pts)"),
    }
    .with_context(|| format!("reading {}", path.display()))?;

    if points.is_empty() {
        bail!("{} contained no points", path.display());
    }
    Ok(points)
}

/// Whitespace- or comma-separated `x y z [r g b]` per line, `#` comments.
///
/// Deliberately forgiving about the extra columns: exports routinely carry
/// intensity, normals or classification after the coordinates, and
/// rejecting the file over a column nobody asked for is not useful.
pub fn read_xyz(reader: &mut impl BufRead) -> Result<Vec<Point>> {
    let mut out = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("reading line {}", i + 1))?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect();
        if fields.len() < 3 {
            continue; // Header row or stray text; skip rather than fail.
        }
        let (Ok(x), Ok(y), Ok(z)) = (
            fields[0].parse::<f32>(),
            fields[1].parse::<f32>(),
            fields[2].parse::<f32>(),
        ) else {
            // A non-numeric first row is a column header, which is common
            // in CSV exports.
            continue;
        };
        if !(x.is_finite() && y.is_finite() && z.is_finite()) {
            continue;
        }
        let mut p = Point::new(x, y, z);
        if fields.len() >= 6
            && let (Ok(r), Ok(g), Ok(b)) = (
                fields[3].parse::<f32>(),
                fields[4].parse::<f32>(),
                fields[5].parse::<f32>(),
            )
        {
            // Colour is written either 0..1 or 0..255 depending on the
            // tool; guess from the range rather than picking one and being
            // wrong half the time.
            let scale = if r <= 1.0 && g <= 1.0 && b <= 1.0 { 255.0 } else { 1.0 };
            p.color = [
                (r * scale).clamp(0.0, 255.0) as u8,
                (g * scale).clamp(0.0, 255.0) as u8,
                (b * scale).clamp(0.0, 255.0) as u8,
            ];
        }
        out.push(p);
        if out.len() > MAX_POINTS {
            bail!("more than {MAX_POINTS} points");
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PlyFormat {
    Ascii,
    BinaryLe,
}

#[derive(Debug, Clone, Copy)]
struct PlyProp {
    /// Size in bytes for binary reads.
    size: usize,
    kind: PlyKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PlyKind {
    Float,
    Int,
    Uint,
}

fn ply_type(name: &str) -> Option<PlyProp> {
    let (size, kind) = match name {
        "char" | "int8" => (1, PlyKind::Int),
        "uchar" | "uint8" => (1, PlyKind::Uint),
        "short" | "int16" => (2, PlyKind::Int),
        "ushort" | "uint16" => (2, PlyKind::Uint),
        "int" | "int32" => (4, PlyKind::Int),
        "uint" | "uint32" => (4, PlyKind::Uint),
        "float" | "float32" => (4, PlyKind::Float),
        "double" | "float64" => (8, PlyKind::Float),
        _ => return None,
    };
    Some(PlyProp { size, kind })
}

pub fn read_ply(reader: &mut impl BufRead) -> Result<Vec<Point>> {
    let mut magic = String::new();
    reader.read_line(&mut magic)?;
    if magic.trim() != "ply" {
        bail!("not a PLY file (missing 'ply' magic)");
    }

    let mut format = None;
    let mut count = 0usize;
    // Properties of the *vertex* element, in file order — the offsets
    // matter for binary reads.
    let mut props: Vec<(String, PlyProp)> = Vec::new();
    let mut in_vertex = false;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            bail!("PLY header ended without end_header");
        }
        let line = line.trim();
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["end_header"] => break,
            ["format", f, ..] => {
                format = Some(match *f {
                    "ascii" => PlyFormat::Ascii,
                    "binary_little_endian" => PlyFormat::BinaryLe,
                    // Big-endian PLY exists but is vanishingly rare; say so
                    // rather than reading it wrong.
                    other => bail!("unsupported PLY format {other:?}"),
                });
            }
            ["element", name, n] => {
                in_vertex = *name == "vertex";
                if in_vertex {
                    count = n.parse().context("vertex count")?;
                    if count > MAX_POINTS {
                        bail!("PLY declares {count} points, more than {MAX_POINTS}");
                    }
                }
            }
            ["property", "list", ..] => {
                // Face lists and similar. Only vertex properties are read,
                // and a list inside the vertex element would break the
                // fixed stride, so refuse rather than misparse.
                if in_vertex {
                    bail!("list properties inside the vertex element are not supported");
                }
            }
            ["property", ty, name] if in_vertex => {
                let prop = ply_type(ty).with_context(|| format!("unknown PLY type {ty:?}"))?;
                props.push(((*name).to_string(), prop));
            }
            _ => {}
        }
    }

    let format = format.context("PLY header declared no format")?;
    let index = |n: &str| props.iter().position(|(p, _)| p == n);
    let (Some(xi), Some(yi), Some(zi)) = (index("x"), index("y"), index("z")) else {
        bail!("PLY vertex element has no x/y/z properties");
    };
    let color_idx = ["red", "green", "blue"].map(index);

    match format {
        PlyFormat::Ascii => read_ply_ascii(reader, count, xi, yi, zi, color_idx),
        PlyFormat::BinaryLe => read_ply_binary(reader, count, &props, xi, yi, zi, color_idx),
    }
}

fn read_ply_ascii(
    reader: &mut impl BufRead,
    count: usize,
    xi: usize,
    yi: usize,
    zi: usize,
    color_idx: [Option<usize>; 3],
) -> Result<Vec<Point>> {
    let mut out = Vec::with_capacity(count.min(1 << 20));
    for line in reader.lines() {
        let line = line?;
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() {
            continue;
        }
        let needed = xi.max(yi).max(zi);
        if f.len() <= needed {
            continue;
        }
        let (Ok(x), Ok(y), Ok(z)) =
            (f[xi].parse::<f32>(), f[yi].parse::<f32>(), f[zi].parse::<f32>())
        else {
            continue;
        };
        let mut p = Point::new(x, y, z);
        if let [Some(r), Some(g), Some(b)] = color_idx
            && f.len() > r.max(g).max(b)
        {
            p.color = [
                f[r].parse::<f32>().unwrap_or(255.0).clamp(0.0, 255.0) as u8,
                f[g].parse::<f32>().unwrap_or(255.0).clamp(0.0, 255.0) as u8,
                f[b].parse::<f32>().unwrap_or(255.0).clamp(0.0, 255.0) as u8,
            ];
        }
        out.push(p);
        if out.len() == count {
            break;
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn read_ply_binary(
    reader: &mut impl Read,
    count: usize,
    props: &[(String, PlyProp)],
    xi: usize,
    yi: usize,
    zi: usize,
    color_idx: [Option<usize>; 3],
) -> Result<Vec<Point>> {
    let stride: usize = props.iter().map(|(_, p)| p.size).sum();
    if stride == 0 {
        bail!("PLY vertex element has zero-width rows");
    }
    // Byte offset of each property within a row, computed once.
    let mut offsets = Vec::with_capacity(props.len());
    let mut at = 0usize;
    for (_, p) in props {
        offsets.push(at);
        at += p.size;
    }

    let mut row = vec![0u8; stride];
    let mut out = Vec::with_capacity(count.min(1 << 20));
    for i in 0..count {
        // A truncated file is common — an interrupted export, a partial
        // download — so stop cleanly with what was read rather than
        // failing the whole load.
        match reader.read_exact(&mut row) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                log::warn!("PLY truncated after {i} of {count} points; using what was read");
                break;
            }
            Err(e) => return Err(e).context("reading PLY body"),
        }
        let get = |idx: usize| -> f32 {
            let (_, p) = &props[idx];
            let o = offsets[idx];
            let b = &row[o..o + p.size];
            match (p.kind, p.size) {
                (PlyKind::Float, 4) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                (PlyKind::Float, 8) => {
                    f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32
                }
                (PlyKind::Uint, 1) => b[0] as f32,
                (PlyKind::Int, 1) => (b[0] as i8) as f32,
                (PlyKind::Uint, 2) => u16::from_le_bytes([b[0], b[1]]) as f32,
                (PlyKind::Int, 2) => i16::from_le_bytes([b[0], b[1]]) as f32,
                (PlyKind::Uint, 4) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                (PlyKind::Int, 4) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                _ => 0.0,
            }
        };
        let (x, y, z) = (get(xi), get(yi), get(zi));
        if !(x.is_finite() && y.is_finite() && z.is_finite()) {
            continue;
        }
        let mut p = Point::new(x, y, z);
        if let [Some(r), Some(g), Some(b)] = color_idx {
            p.color = [
                get(r).clamp(0.0, 255.0) as u8,
                get(g).clamp(0.0, 255.0) as u8,
                get(b).clamp(0.0, 255.0) as u8,
            ];
        }
        out.push(p);
    }
    Ok(out)
}

/// Centre on the bounding box and scale the widest axis to fit [-1, 1],
/// so an imported cloud sits where the procedural shapes do and
/// `/particles/spread` means the same thing for both.
///
/// Uniform scale, not per-axis: fitting each axis independently would
/// stretch a scan into something that is no longer the thing scanned.
pub fn normalize(points: &mut [Point]) {
    if points.is_empty() {
        return;
    }
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in points.iter() {
        for i in 0..3 {
            lo[i] = lo[i].min(p.pos[i]);
            hi[i] = hi[i].max(p.pos[i]);
        }
    }
    let centre = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ];
    let extent = (0..3).fold(0.0f32, |m, i| m.max(hi[i] - lo[i]));
    let scale = if extent > 1e-9 { 2.0 / extent } else { 1.0 };
    for p in points.iter_mut() {
        for i in 0..3 {
            p.pos[i] = (p.pos[i] - centre[i]) * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ply_ascii() -> &'static str {
        "ply\n\
         format ascii 1.0\n\
         comment made by a scanner\n\
         element vertex 3\n\
         property float x\n\
         property float y\n\
         property float z\n\
         property uchar red\n\
         property uchar green\n\
         property uchar blue\n\
         element face 0\n\
         property list uchar int vertex_indices\n\
         end_header\n\
         1.0 2.0 3.0 255 0 0\n\
         -1.0 0.0 1.0 0 255 0\n\
         0.5 0.5 0.5 0 0 255\n"
    }

    #[test]
    fn reads_ascii_ply_with_colour() {
        let mut r = std::io::Cursor::new(ply_ascii());
        let pts = read_ply(&mut r).expect("parse failed");
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[0].pos, [1.0, 2.0, 3.0]);
        assert_eq!(pts[0].color, [255, 0, 0]);
        assert_eq!(pts[2].pos, [0.5, 0.5, 0.5]);
        assert_eq!(pts[2].color, [0, 0, 255]);
    }

    /// A face element with a list property must not derail the vertex
    /// read — almost every mesh-exported PLY has one.
    #[test]
    fn a_trailing_face_element_is_ignored() {
        let mut r = std::io::Cursor::new(ply_ascii());
        assert_eq!(read_ply(&mut r).unwrap().len(), 3);
    }

    /// Binary PLY carries properties we do not want in a fixed-width row,
    /// so x/y/z have to be found by offset rather than assumed to be first.
    #[test]
    fn reads_binary_ply_with_interleaved_properties() {
        let mut buf = Vec::new();
        buf.extend_from_slice(
            b"ply\nformat binary_little_endian 1.0\n\
              element vertex 2\n\
              property float nx\n\
              property float x\n\
              property float y\n\
              property float z\n\
              property uchar red\n\
              property uchar green\n\
              property uchar blue\n\
              end_header\n",
        );
        for (nx, x, y, z, c) in [
            (9.0f32, 1.0f32, 2.0f32, 3.0f32, [10u8, 20, 30]),
            (9.0, -4.0, -5.0, -6.0, [40, 50, 60]),
        ] {
            for v in [nx, x, y, z] {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf.extend_from_slice(&c);
        }

        let mut r = std::io::Cursor::new(buf);
        let pts = read_ply(&mut r).expect("parse failed");
        assert_eq!(pts.len(), 2);
        // The leading normal must not be mistaken for the position.
        assert_eq!(pts[0].pos, [1.0, 2.0, 3.0]);
        assert_eq!(pts[0].color, [10, 20, 30]);
        assert_eq!(pts[1].pos, [-4.0, -5.0, -6.0]);
    }

    /// A partial export is a normal thing to be handed. Take what is there
    /// rather than failing the whole load.
    #[test]
    fn a_truncated_binary_body_keeps_what_was_read() {
        let mut buf = Vec::new();
        buf.extend_from_slice(
            b"ply\nformat binary_little_endian 1.0\n\
              element vertex 4\n\
              property float x\nproperty float y\nproperty float z\n\
              end_header\n",
        );
        for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        // Declares 4 points, supplies 2.
        let mut r = std::io::Cursor::new(buf);
        let pts = read_ply(&mut r).expect("should not fail on truncation");
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[1].pos, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn malformed_headers_are_errors_not_panics() {
        for bad in [
            "not a ply at all\n",
            "ply\nformat ascii 1.0\nelement vertex 1\nproperty float q\nend_header\n0\n",
            "ply\nformat binary_big_endian 1.0\nend_header\n",
            "ply\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n",
            "ply\nformat ascii 1.0\n",
        ] {
            let mut r = std::io::Cursor::new(bad);
            assert!(read_ply(&mut r).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn reads_xyz_with_comments_headers_and_extra_columns() {
        let text = "# exported by something\n\
                    x,y,z,intensity\n\
                    1.0, 2.0, 3.0, 0.4\n\
                    \n\
                    -1.0 0.0 1.0\n\
                    2 3 4 0.1 0.2 0.9\n";
        let mut r = std::io::Cursor::new(text);
        let pts = read_xyz(&mut r).unwrap();
        assert_eq!(pts.len(), 3, "{pts:?}");
        assert_eq!(pts[0].pos, [1.0, 2.0, 3.0]);
        assert_eq!(pts[1].pos, [-1.0, 0.0, 1.0]);
        // 0..1 colours are scaled up; 0..255 are taken as-is.
        assert_eq!(pts[2].color, [25, 51, 229]);
    }

    #[test]
    fn non_finite_coordinates_are_dropped() {
        let mut r = std::io::Cursor::new("1 2 3\nnan 1 2\n1 inf 2\n4 5 6\n");
        let pts = read_xyz(&mut r).unwrap();
        assert_eq!(pts.len(), 2, "{pts:?}");
    }

    /// Normalisation must centre and fit without distorting: an imported
    /// scan stretched per-axis is no longer the thing that was scanned.
    #[test]
    fn normalize_centres_and_scales_uniformly() {
        let mut pts = vec![
            Point::new(100.0, 0.0, 0.0),
            Point::new(140.0, 10.0, 5.0),
            Point::new(120.0, 5.0, 2.5),
        ];
        normalize(&mut pts);

        let xs: Vec<f32> = pts.iter().map(|p| p.pos[0]).collect();
        assert!((xs[0] + 1.0).abs() < 1e-5, "widest axis should span -1..1: {xs:?}");
        assert!((xs[1] - 1.0).abs() < 1e-5, "{xs:?}");
        // The narrow axes keep their proportion rather than being stretched.
        let y_extent = pts.iter().map(|p| p.pos[1]).fold(f32::MIN, f32::max)
            - pts.iter().map(|p| p.pos[1]).fold(f32::MAX, f32::min);
        assert!((y_extent - 0.5).abs() < 1e-5, "aspect distorted: y extent {y_extent}");
    }

    #[test]
    fn normalize_survives_degenerate_input() {
        let mut empty: Vec<Point> = Vec::new();
        normalize(&mut empty);
        // All points identical: no division by zero, no NaN.
        let mut same = vec![Point::new(5.0, 5.0, 5.0); 3];
        normalize(&mut same);
        assert!(same.iter().all(|p| p.pos.iter().all(|v| v.is_finite())), "{same:?}");
    }
}
