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
    /// Which way the surface faces here, or all-zero for "not known".
    ///
    /// Zero rather than a default direction, and the distinction is
    /// load-bearing: the shader stands every directional term down where
    /// the normal is zero, because a made-up normal lights a wall the
    /// wrong way round and that is worse than not lighting it at all.
    pub normal: [f32; 3],
    pub color: [u8; 3],
}

impl Point {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { pos: [x, y, z], normal: [0.0; 3], color: [255, 255, 255] }
    }

    /// Whether this point knows which way it faces.
    pub fn oriented(&self) -> bool {
        let n = self.normal;
        n[0] * n[0] + n[1] * n[1] + n[2] * n[2] > 0.25
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
    // Normals, where the export bothered. The property table already
    // carried them — the header parser keeps every vertex property by
    // name because the binary offsets depend on all of them — so this is
    // only a matter of asking.
    let normal_idx = ["nx", "ny", "nz"].map(index);

    match format {
        PlyFormat::Ascii => read_ply_ascii(reader, count, xi, yi, zi, color_idx, normal_idx),
        PlyFormat::BinaryLe => {
            read_ply_binary(reader, count, &props, xi, yi, zi, color_idx, normal_idx)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn read_ply_ascii(
    reader: &mut impl BufRead,
    count: usize,
    xi: usize,
    yi: usize,
    zi: usize,
    color_idx: [Option<usize>; 3],
    normal_idx: [Option<usize>; 3],
) -> Result<Vec<Point>> {
    let mut out = Vec::with_capacity(count.min(1 << 20));
    for line in reader.lines() {
        // The declared count is where the vertex element *ends*, not a
        // hint. Reading to end-of-file swept whatever follows into the
        // cloud — a scan with a face element after its vertices had every
        // face row "3 0 1 2" parsed as a point at (3,0,1), which corrupts
        // the cloud with a diagonal smear towards the origin.
        if out.len() >= count {
            break;
        }
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
        if let [Some(nx), Some(ny), Some(nz)] = normal_idx
            && f.len() > nx.max(ny).max(nz)
            && let (Ok(a), Ok(b), Ok(c)) = (
                f[nx].parse::<f32>(),
                f[ny].parse::<f32>(),
                f[nz].parse::<f32>(),
            )
        {
            p.normal = unit([a, b, c]);
        }
        out.push(p);
        if out.len() == count {
            break;
        }
    }
    Ok(out)
}

/// A unit vector, or all-zero where there is nothing to normalise.
///
/// Zero-length in gives zero-length out rather than a NaN: an export that
/// wrote `0 0 0` for a normal it could not compute is a real file, and it
/// should read as "unknown" rather than poison every lighting term the
/// point touches.
fn unit(v: [f32; 3]) -> [f32; 3] {
    let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if !len2.is_finite() || len2 < 1e-12 {
        return [0.0; 3];
    }
    let inv = len2.sqrt().recip();
    [v[0] * inv, v[1] * inv, v[2] * inv]
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
    normal_idx: [Option<usize>; 3],
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
        if let [Some(nx), Some(ny), Some(nz)] = normal_idx {
            p.normal = unit([get(nx), get(ny), get(nz)]);
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
        for (v, c) in p.pos.iter_mut().zip(centre) {
            *v = (*v - c) * scale;
        }
    }
}


/// Normalisation that holds still while the subject moves.
///
/// [`normalize`] is right for a file: measure the cloud, centre it, scale
/// it to fit, once. Run the same thing on every frame of a live stream
/// and the result swims. A LiDAR frame's bounding box is never twice the
/// same — one stray point at the back of the room changes the extent, so
/// the *whole* cloud rescales; the subject leans and the centre moves, so
/// the whole cloud slides. Nothing about the geometry jittered, only the
/// frame it was measured against.
///
/// So the fit is measured robustly and then eased into:
///
/// - Centre and spread come from the mean and standard deviation rather
///   than from min and max. A single outlier moves a mean by a
///   thousandth and pins a min/max entirely, which is the difference
///   between a stable cloud and a twitching one.
/// - The transform follows the measurement through a low-pass, so a real
///   move arrives over about half a second and per-frame noise does not
///   arrive at all.
#[derive(Debug, Clone)]
pub struct StreamFit {
    centre: [f32; 3],
    scale: f32,
    /// Nothing measured yet: the first frame is adopted outright rather
    /// than eased into from an arbitrary starting point, which would
    /// otherwise fly in from the origin at every stream start.
    started: bool,
    /// Per-frame follow rate. 0.08 at 30 fps is roughly a half-second
    /// settle — slow enough to reject frame noise, quick enough that
    /// stepping sideways does not leave the cloud behind.
    rate: f32,
}

impl Default for StreamFit {
    fn default() -> Self {
        Self { centre: [0.0; 3], scale: 1.0, started: false, rate: 0.08 }
    }
}

impl StreamFit {
    /// Centre and scale `points` in place, easing the fit toward this
    /// frame's measurement.
    pub fn apply(&mut self, points: &mut [Point]) {
        if points.is_empty() {
            return;
        }
        let n = points.len() as f32;
        let mut mean = [0.0f32; 3];
        for p in points.iter() {
            for (m, v) in mean.iter_mut().zip(p.pos) {
                *m += v;
            }
        }
        for m in &mut mean {
            *m /= n;
        }
        // Spread as the largest per-axis standard deviation. Two of them
        // covers the bulk of a cloud; the tail lands outside the unit box
        // and is none the worse for it, which is the trade that keeps a
        // far wall from shrinking the subject to nothing.
        let mut var = [0.0f32; 3];
        for p in points.iter() {
            for ((v, c), x) in var.iter_mut().zip(mean).zip(p.pos) {
                let d = x - c;
                *v += d * d;
            }
        }
        let spread = (0..3).fold(0.0f32, |m, i| m.max((var[i] / n).sqrt()));
        let want = if spread > 1e-9 { 1.0 / (2.0 * spread) } else { 1.0 };

        if !self.started {
            self.started = true;
            self.centre = mean;
            self.scale = want;
        } else {
            let k = self.rate;
            for (c, m) in self.centre.iter_mut().zip(mean) {
                *c += (m - *c) * k;
            }
            self.scale += (want - self.scale) * k;
        }

        for p in points.iter_mut() {
            for (v, c) in p.pos.iter_mut().zip(self.centre) {
                *v = (*v - c) * self.scale;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live cloud must not swim when one point moves.
    ///
    /// This is the jitter: `normalize` measures min/max per frame, so a
    /// single stray return at the back of the room changes the extent
    /// and rescales everything, and a subject leaning moves the centre
    /// and slides everything. The geometry was steady; the frame it was
    /// measured against was not.
    #[test]
    fn a_stream_fit_holds_still_when_an_outlier_appears() {
        let body = || {
            (0..200)
                .map(|i| Point {
                    pos: [(i % 10) as f32 * 0.1, (i / 10) as f32 * 0.1, 0.0],
                    normal: [0.0; 3],
                    color: [255, 255, 255],
                })
                .collect::<Vec<_>>()
        };

        // Per-frame min/max normalisation: the same body, plus one far
        // point on the second frame, comes back at a different size.
        let mut a = body();
        normalize(&mut a);
        let mut b = body();
        b.push(Point { pos: [50.0, 0.0, 0.0], normal: [0.0; 3], color: [255, 255, 255] });
        normalize(&mut b);
        let moved = (a[0].pos[0] - b[0].pos[0]).abs();
        assert!(
            moved > 0.5,
            "the test's own premise is wrong — min/max normalisation did not move ({moved})"
        );

        // The streaming fit barely notices.
        let mut fit = StreamFit::default();
        let mut a = body();
        fit.apply(&mut a);
        let mut b = body();
        b.push(Point { pos: [50.0, 0.0, 0.0], normal: [0.0; 3], color: [255, 255, 255] });
        fit.apply(&mut b);
        let moved = (a[0].pos[0] - b[0].pos[0]).abs();
        assert!(
            moved < 0.15,
            "one outlier moved the whole cloud by {moved} — this is the jitter"
        );
    }

    /// It must still follow the subject, or a performer who takes a step
    /// leaves the frame and never comes back.
    #[test]
    fn a_stream_fit_follows_a_real_move() {
        let at = |x: f32| {
            (0..200)
                .map(|i| Point {
                    pos: [x + (i % 10) as f32 * 0.1, (i / 10) as f32 * 0.1, 0.0],
                    normal: [0.0; 3],
                    color: [255, 255, 255],
                })
                .collect::<Vec<_>>()
        };
        let mut fit = StreamFit::default();
        let mut first = at(0.0);
        fit.apply(&mut first);
        let centred = first[0].pos[0];

        // Hold a moved subject for two seconds of frames. Two rather
        // than one because the follow is deliberately slow — a big jump
        // settles over about a second and a half, and buying that back
        // would mean letting per-frame noise through, which is the whole
        // thing being fixed.
        let mut last = Vec::new();
        for _ in 0..60 {
            last = at(5.0);
            fit.apply(&mut last);
        }
        assert!(
            (last[0].pos[0] - centred).abs() < 0.2,
            "the fit never caught up with a subject that moved: {} vs {centred}",
            last[0].pos[0]
        );
    }

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

    /// The declared vertex count is where the vertices end. Reading to
    /// end-of-file swept the face element into the cloud: every face row
    /// "3 0 1 2" parsed as a point at (3,0,1), smearing a diagonal of
    /// phantom points towards the origin through any scan with faces.
    #[test]
    fn face_rows_after_the_vertices_are_not_points() {
        let dir = std::env::temp_dir().join(format!("vizz-plyface-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("faces.ply");
        std::fs::write(
            &path,
            "ply\nformat ascii 1.0\nelement vertex 2\n\
             property float x\nproperty float y\nproperty float z\n\
             element face 2\nproperty list uchar int vertex_indices\n\
             end_header\n\
             1.0 2.0 3.0\n\
             4.0 5.0 6.0\n\
             3 0 1 2\n\
             3 1 2 0\n",
        )
        .unwrap();
        let points = load(&path).unwrap();
        assert_eq!(points.len(), 2, "face rows leaked into the cloud");
        assert_eq!(points[1].pos, [4.0, 5.0, 6.0]);
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

/// Work out which way each point's surface faces, from its neighbours.
///
/// Most scans are exported without normals — the format has a slot for
/// them and half the tools leave it empty — so without this, directional
/// light would work on a minority of files and whether relighting did
/// anything would be decided by whoever pressed export.
///
/// The method is the standard one: take each point's nearest neighbours,
/// fit a plane through them, and the plane's normal is the surface's. A
/// plane fit is the eigenvector of the neighbourhood's covariance with
/// the smallest eigenvalue — the direction in which the neighbours vary
/// least, which for anything locally flat is straight off the surface.
///
/// # Orientation is left ambiguous, on purpose
///
/// A plane fit cannot tell `n` from `-n`, and resolving it properly means
/// propagating a consistent choice across the whole cloud — which is a
/// minimum spanning tree over a graph of a million nodes, and still gets
/// it backwards for a scan of a room, where the surfaces you want lit
/// face inwards. The shader flips the normal towards the eye instead, so
/// every surface is two-sided. There are no shadows here for that to be
/// inconsistent with, and a point cloud lit from the wrong side is black
/// for no reason a performer could diagnose.
///
/// Points already carrying a normal — from a file that did write them —
/// are left alone.
pub fn estimate_normals(points: &mut [Point]) {
    const NEIGHBOURS: usize = 12;
    if points.len() < NEIGHBOURS {
        return;
    }
    let grid = Grid::build(points);
    // Read from a copy: filling in a normal must not change what the next
    // point's neighbourhood looks like, and more practically the borrow
    // checker is right that reading and writing the same slice here is a
    // question worth answering explicitly.
    let positions: Vec<[f32; 3]> = points.iter().map(|p| p.pos).collect();
    let mut near: Vec<(f32, usize)> = Vec::with_capacity(64);
    for i in 0..points.len() {
        if points[i].oriented() {
            continue;
        }
        near.clear();
        grid.around(positions[i], |j| {
            if j != i {
                near.push((dist2(positions[i], positions[j]), j));
            }
        });
        if near.len() < 3 {
            continue;
        }
        // Partial rather than a full sort: only the nearest handful are
        // read, and a neighbourhood can easily be a few hundred points in
        // a dense scan.
        let k = NEIGHBOURS.min(near.len());
        near.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0));
        points[i].normal = plane_normal(&positions, &near[..k], positions[i]);
    }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// The direction a neighbourhood varies *least* in: the eigenvector of
/// its covariance with the smallest eigenvalue.
fn plane_normal(pos: &[[f32; 3]], near: &[(f32, usize)], _centre: [f32; 3]) -> [f32; 3] {
    let n = near.len() as f32;
    let mut mean = [0.0f32; 3];
    for (_, j) in near {
        for a in 0..3 {
            mean[a] += pos[*j][a];
        }
    }
    for m in &mut mean {
        *m /= n;
    }
    // Upper triangle of the covariance; it is symmetric.
    let (mut xx, mut xy, mut xz, mut yy, mut yz, mut zz) = (0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0);
    for (_, j) in near {
        let d = [
            pos[*j][0] - mean[0],
            pos[*j][1] - mean[1],
            pos[*j][2] - mean[2],
        ];
        xx += d[0] * d[0];
        xy += d[0] * d[1];
        xz += d[0] * d[2];
        yy += d[1] * d[1];
        yz += d[1] * d[2];
        zz += d[2] * d[2];
    }
    smallest_eigenvector([xx, xy, xz, yy, yz, zz])
}

/// Eigenvector of the smallest eigenvalue of a symmetric 3×3, given as
/// `[xx, xy, xz, yy, yz, zz]`.
///
/// Closed form rather than an iteration: the eigenvalues of a symmetric
/// 3×3 have an exact expression, and an iterative solver would need a
/// convergence criterion tuned against clouds nobody has yet.
fn smallest_eigenvector(m: [f32; 6]) -> [f32; 3] {
    let [xx, xy, xz, yy, yz, zz] = m;
    let off = xy * xy + xz * xz + yz * yz;
    if off < 1e-20 {
        // Already diagonal: the answer is whichever axis varies least.
        let (mut best, mut which) = (xx, 0);
        if yy < best {
            best = yy;
            which = 1;
        }
        if zz < best {
            which = 2;
        }
        let mut v = [0.0; 3];
        v[which] = 1.0;
        return v;
    }
    let q = (xx + yy + zz) / 3.0;
    let p2 = (xx - q) * (xx - q) + (yy - q) * (yy - q) + (zz - q) * (zz - q) + 2.0 * off;
    let p = (p2 / 6.0).max(1e-30).sqrt();
    // B = (A − qI)/p, whose determinant gives the angle.
    let b = [
        (xx - q) / p,
        xy / p,
        xz / p,
        (yy - q) / p,
        yz / p,
        (zz - q) / p,
    ];
    let det = b[0] * (b[3] * b[5] - b[4] * b[4]) - b[1] * (b[1] * b[5] - b[4] * b[2])
        + b[2] * (b[1] * b[4] - b[3] * b[2]);
    let r = (det / 2.0).clamp(-1.0, 1.0);
    let phi = r.acos() / 3.0;
    // The smallest of the three, which is the one a plane fit wants.
    let small = q + 2.0 * p * (phi + std::f32::consts::TAU / 3.0).cos();
    // Null space of (A − λI): the cross product of two of its rows. Which
    // two matters — rows can be parallel, and their cross product is then
    // zero — so all three pairs are tried and the longest wins.
    let a = [
        [xx - small, xy, xz],
        [xy, yy - small, yz],
        [xz, yz, zz - small],
    ];
    let mut best = [0.0f32; 3];
    let mut best_len = 0.0f32;
    for (i, j) in [(0, 1), (0, 2), (1, 2)] {
        let c = cross(a[i], a[j]);
        let l = c[0] * c[0] + c[1] * c[1] + c[2] * c[2];
        if l > best_len {
            best_len = l;
            best = c;
        }
    }
    unit(best)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A uniform bucket grid over the cloud, for neighbour lookups.
///
/// Uniform rather than a kd-tree because the clouds this runs on have
/// already been normalised into a unit box and sampled down to what the
/// GPU slot holds, so the density is bounded and a grid is both faster to
/// build and simpler to be sure of.
struct Grid {
    cell: f32,
    lo: [f32; 3],
    dim: [i32; 3],
    /// Point indices, bucketed; `start[c]..start[c + 1]` is cell `c`.
    start: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    fn build(points: &[Point]) -> Self {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in points {
            for a in 0..3 {
                lo[a] = lo[a].min(p.pos[a]);
                hi[a] = hi[a].max(p.pos[a]);
            }
        }
        // Cell size, from the shape the cloud actually has.
        //
        // The obvious version divides the bounding box by the cube root
        // of the count, which assumes the points fill a volume. Almost
        // nothing here does. A depth sensor sends a 2.5D sheet and a scan
        // of a wall is a plane, and for those the box is mostly empty:
        // sizing for a volume gives cells far too large in the two axes
        // that matter, points pile into a fraction of them, and the
        // 27-cell scan walks hundreds of candidates per point instead of
        // a handful. Measured on a live sensor frame that was 851
        // candidates against an intended 108, and three times the
        // running time — while a test on a uniform random cube, which is
        // the one shape this assumption holds for, reported it fine.
        //
        // So count the axes the cloud actually extends along and take the
        // root that matches: a cube gets a cube root, a sheet a square
        // root, a line a straight division.
        let mut extents = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        for e in &mut extents {
            if !e.is_finite() || *e < 0.0 {
                *e = 0.0;
            }
        }
        let widest = extents.iter().copied().fold(0.0f32, f32::max).max(1e-6);
        // An axis is "real" if the cloud has meaningful thickness along
        // it. A twentieth of the widest is well below anything that reads
        // as depth and well above the noise on a flat scan.
        let real: Vec<f32> = extents
            .iter()
            .copied()
            .filter(|e| *e > widest * 0.05)
            .collect();
        let dims = real.len().max(1);
        let measure: f32 = real.iter().copied().fold(1.0, |a, b| a * b).max(1e-12);
        // A handful of points per cell, so the 27-cell scan around a
        // point covers a few hundred candidates rather than the cloud.
        let per_cell = 4.0;
        let cells_wanted = (points.len() as f32 / per_cell).max(1.0);
        let cell = (measure / cells_wanted).powf(1.0 / dims as f32).max(1e-6);
        let dim = std::array::from_fn(|a| (((hi[a] - lo[a]) / cell).ceil() as i32 + 1).max(1));
        let cells = (dim[0] as usize) * (dim[1] as usize) * (dim[2] as usize);
        // Counting sort into buckets: two passes, no per-cell Vec.
        let mut counts = vec![0u32; cells + 1];
        let index = |lo: [f32; 3], dim: [i32; 3], cell: f32, p: [f32; 3]| -> usize {
            let c: [i32; 3] = std::array::from_fn(|a| {
                (((p[a] - lo[a]) / cell) as i32).clamp(0, dim[a] - 1)
            });
            (c[2] as usize * dim[1] as usize + c[1] as usize) * dim[0] as usize + c[0] as usize
        };
        for p in points {
            counts[index(lo, dim, cell, p.pos) + 1] += 1;
        }
        for i in 0..cells {
            counts[i + 1] += counts[i];
        }
        let start = counts.clone();
        let mut cursor = counts;
        let mut items = vec![0u32; points.len()];
        for (i, p) in points.iter().enumerate() {
            let c = index(lo, dim, cell, p.pos);
            items[cursor[c] as usize] = i as u32;
            cursor[c] += 1;
        }
        Self { cell, lo, dim, start, items }
    }

    /// Every point in the 27 cells around `p`.
    fn around(&self, p: [f32; 3], mut f: impl FnMut(usize)) {
        let c: [i32; 3] = std::array::from_fn(|a| {
            (((p[a] - self.lo[a]) / self.cell) as i32).clamp(0, self.dim[a] - 1)
        });
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let n = [c[0] + dx, c[1] + dy, c[2] + dz];
                    if (0..3).any(|a| n[a] < 0 || n[a] >= self.dim[a]) {
                        continue;
                    }
                    let idx = (n[2] as usize * self.dim[1] as usize + n[1] as usize)
                        * self.dim[0] as usize
                        + n[0] as usize;
                    for &i in &self.items[self.start[idx] as usize..self.start[idx + 1] as usize] {
                        f(i as usize);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod normal_tests {
    use super::*;

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    /// A flat sheet must come out facing off the sheet.
    ///
    /// Compared by absolute dot product, because a plane fit genuinely
    /// cannot tell `n` from `-n` and the shader is written knowing that.
    /// Asserting a sign here would be asserting something the method does
    /// not provide, and it would pass or fail on the arbitrary order the
    /// eigen solver happens to pick.
    #[test]
    fn a_flat_sheet_faces_off_the_sheet() {
        let mut pts = Vec::new();
        for i in 0..40 {
            for j in 0..40 {
                pts.push(Point::new(i as f32 * 0.05, 0.0, j as f32 * 0.05));
            }
        }
        estimate_normals(&mut pts);
        let mut checked = 0;
        for p in &pts {
            // Skip the rim, where the neighbourhood is one-sided.
            if p.pos[0] < 0.2 || p.pos[0] > 1.7 || p.pos[2] < 0.2 || p.pos[2] > 1.7 {
                continue;
            }
            assert!(p.oriented(), "no normal at {:?}", p.pos);
            assert!(
                dot(p.normal, [0.0, 1.0, 0.0]).abs() > 0.97,
                "{:?} does not face off the sheet: {:?}",
                p.pos,
                p.normal
            );
            checked += 1;
        }
        assert!(checked > 500, "only checked {checked} interior points");
    }

    /// And a sphere faces outward from its own centre — the case a sheet
    /// cannot distinguish, since on a sheet every correct answer is the
    /// same vector.
    #[test]
    fn a_sphere_faces_along_its_own_radius() {
        let mut pts = Vec::new();
        // Fibonacci sphere: even coverage without clustering at the poles,
        // which is where a naive lat/long grid would make the
        // neighbourhood anisotropic and the fit wrong for a reason that
        // has nothing to do with the fit.
        let n = 2000;
        let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
        for i in 0..n {
            let y = 1.0 - (i as f32 / (n - 1) as f32) * 2.0;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let th = golden * i as f32;
            pts.push(Point::new(th.cos() * r, y, th.sin() * r));
        }
        estimate_normals(&mut pts);
        let mut worst = 1.0f32;
        for p in &pts {
            assert!(p.oriented(), "no normal at {:?}", p.pos);
            // The radius *is* the normal on a sphere.
            worst = worst.min(dot(p.normal, unit(p.pos)).abs());
        }
        assert!(worst > 0.9, "worst point faces {worst} off its own radius");
    }

    /// Normals a file already carried are left exactly as they were: an
    /// estimate is a fallback, not an improvement on what the scanner
    /// measured.
    #[test]
    fn a_normal_from_the_file_is_not_overwritten() {
        let mut pts: Vec<Point> = (0..500)
            .map(|i| {
                let mut p = Point::new(i as f32 * 0.01, 0.0, 0.0);
                p.normal = [0.0, 0.0, 1.0];
                p
            })
            .collect();
        estimate_normals(&mut pts);
        assert!(pts.iter().all(|p| p.normal == [0.0, 0.0, 1.0]));
    }

    /// A zero normal in the file reads as "unknown" rather than poisoning
    /// every lighting term the point touches.
    #[test]
    fn a_zero_normal_is_not_a_direction() {
        assert_eq!(unit([0.0, 0.0, 0.0]), [0.0; 3]);
        assert_eq!(unit([f32::NAN, 1.0, 0.0]), [0.0; 3]);
        let p = Point::new(0.0, 0.0, 0.0);
        assert!(!p.oriented(), "a fresh point claims to know which way it faces");
    }

    /// A sheet is the real case, and the one a cube-shaped test hides.
    ///
    /// The first version of this suite measured a uniform random cube,
    /// which is the one shape a volume-filling grid assumption holds for.
    /// A depth sensor sends a 2.5D sheet and a scan of a wall is a plane;
    /// on those the bounding box is mostly empty, points pile into a
    /// fraction of the cells, and the neighbour scan walks hundreds of
    /// candidates per point. It measured three times slower than the cube
    /// while the cube test reported everything fine.
    #[test]
    fn a_sheet_is_not_dramatically_slower_than_a_cube() {
        let n = crate::attractor::POINTS;
        let mut seed = 24680u32;
        let mut rng = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16_777_216.0 - 0.5
        };
        let mut cube: Vec<Point> = (0..n).map(|_| Point::new(rng(), rng(), rng())).collect();
        // A sheet: full extent in two axes, sensor-noise thickness in the
        // third. This is what a frame off a depth camera looks like.
        let mut sheet: Vec<Point> =
            (0..n).map(|_| Point::new(rng(), rng(), rng() * 0.01)).collect();

        let t = std::time::Instant::now();
        estimate_normals(&mut cube);
        let cube_took = t.elapsed();
        let t = std::time::Instant::now();
        estimate_normals(&mut sheet);
        let sheet_took = t.elapsed();

        eprintln!("cube {cube_took:?}, sheet {sheet_took:?}");
        assert!(
            sheet_took < cube_took * 3,
            "a sheet took {sheet_took:?} against a cube's {cube_took:?} — the grid is \
             sizing cells for a volume the points do not fill"
        );
    }

    /// The estimate runs on what the GPU slot actually holds, so this is
    /// the size that matters. It happens at load, beside a file read and
    /// a parse that already cost more than this — but a method that took
    /// ten seconds would need a thread, and the only way to know which is
    /// to measure.
    #[test]
    fn estimating_a_full_slot_is_quick() {
        let n = crate::attractor::POINTS;
        let mut pts = Vec::with_capacity(n);
        let mut seed = 12345u32;
        let mut rng = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16777216.0 - 0.5
        };
        for _ in 0..n {
            pts.push(Point::new(rng(), rng(), rng()));
        }
        let t = std::time::Instant::now();
        estimate_normals(&mut pts);
        let took = t.elapsed();
        // Generous, because CI runners are slow and this is a guard
        // against an accidental quadratic rather than a benchmark.
        assert!(
            took.as_secs_f32() < 5.0,
            "estimating {n} normals took {took:?}, which needs a thread rather than a comment"
        );
        eprintln!("estimate_normals: {n} points in {took:?}");
    }
}
