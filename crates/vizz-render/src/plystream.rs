//! Live point clouds: a stream of PLY frames rather than a file.
//!
//! An app that streams PLY just concatenates whole files onto a socket, so
//! the only real problem is **framing** — knowing where one cloud ends and
//! the next begins. There is no length prefix and no delimiter.
//!
//! There does not need to be one. A PLY header declares `element vertex N`
//! and the properties each vertex carries, which is exactly enough to
//! compute the body's size: `N * stride` for binary, `N` lines for ASCII.
//! The header is its own frame length, so this reads frames off a stream
//! with no wrapper protocol and works with anything that writes ordinary
//! PLY files back to back.
//!
//! Why not hand the socket straight to [`crate::pointcloud::read_ply`]:
//! its ASCII path iterates `reader.lines()`, which is right for a file and
//! wrong for a stream — the first frame would swallow every frame after
//! it. Binary would have framed correctly, which is worse, because it
//! would have worked in testing and failed on whichever app sends ASCII.
//!
//! So the header is scanned here to find the body's extent, the exact
//! bytes of one frame are lifted out, and the existing parser runs over
//! those. The parser is untouched and files keep working as before.

use std::io::{BufRead, Cursor};

use anyhow::{Result, bail};

use crate::pointcloud::Point;

/// Largest header we will read before deciding this is not PLY.
///
/// A real header is a few hundred bytes. Without a cap, a stream of
/// anything else — an HTTP error page, a wrong port — is read forever.
const MAX_HEADER: usize = 64 * 1024;

/// Largest vertex count accepted from a stream.
///
/// The count comes off the wire, and `N * stride` is used to size a read.
/// A corrupt or hostile header claiming four billion vertices would
/// otherwise be an instant allocation of everything the machine has.
const MAX_VERTICES: usize = 20_000_000;

/// Most bytes one frame body may claim. The vertex cap alone still let a
/// header ask for MAX_VERTICES at a 64-byte stride — a 1.28 GB
/// allocation, sized entirely by an untrusted peer before a single byte
/// of body had arrived. A quarter gigabyte covers any real scan at any
/// real stride and is survivable if a hostile peer asks for all of it.
const MAX_FRAME_BYTES: usize = 256 << 20;

/// What the header says about the body that follows it.
#[derive(Debug, PartialEq)]
struct FrameShape {
    vertices: usize,
    /// Bytes per vertex for a binary body; `None` for ASCII.
    stride: Option<usize>,
}

/// Read one PLY frame from a stream, leaving it positioned at the next.
///
/// Returns `Ok(None)` at a clean end of stream — the sender closing
/// between frames is normal shutdown, not an error.
pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<Point>>> {
    let Some(header) = read_header(reader)? else {
        return Ok(None);
    };
    let shape = parse_shape(&header)?;

    // Header and body are concatenated back together so the existing
    // parser sees exactly what it would see in a file.
    let mut frame = header;
    match shape.stride {
        Some(stride) => {
            let len = shape
                .vertices
                .checked_mul(stride)
                .filter(|n| *n <= MAX_FRAME_BYTES)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "PLY frame body over the {} MiB limit",
                        MAX_FRAME_BYTES >> 20
                    )
                })?;
            let start = frame.len();
            frame.resize(start + len, 0);
            reader.read_exact(&mut frame[start..])?;
        }
        None => {
            // ASCII: exactly `vertices` non-blank lines. Blank lines are
            // skipped rather than counted, matching the file parser, so a
            // sender that pads between frames does not desynchronise the
            // stream by one cloud and never recover.
            let mut seen = 0;
            let mut line = String::new();
            while seen < shape.vertices {
                line.clear();
                if reader.read_line(&mut line)? == 0 {
                    bail!(
                        "PLY stream ended after {seen} of {} vertices",
                        shape.vertices
                    );
                }
                if !line.trim().is_empty() {
                    seen += 1;
                }
                frame.extend_from_slice(line.as_bytes());
            }
        }
    }

    let points = crate::pointcloud::read_ply(&mut Cursor::new(frame))?;
    Ok(Some(points))
}

/// Read up to and including `end_header`. `None` at a clean end of stream.
fn read_header(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>> {
    let mut header = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            // Nothing at all: the sender closed between frames.
            if header.is_empty() {
                return Ok(None);
            }
            bail!("PLY stream ended mid-header");
        }
        // Skip whitespace between frames rather than folding it into the
        // header: a sender that pads between clouds would otherwise
        // produce a header starting with a blank line, which the parser
        // rejects as "missing 'ply' magic" — every frame after the first.
        if header.is_empty() && line.trim().is_empty() {
            continue;
        }
        header.extend_from_slice(line.as_bytes());
        if header.len() > MAX_HEADER {
            bail!(
                "no end_header within {MAX_HEADER} bytes — is this a PLY stream?"
            );
        }
        if line.trim() == "end_header" {
            return Ok(Some(header));
        }
    }
}

/// Bytes occupied by a PLY scalar type.
fn type_size(name: &str) -> Option<usize> {
    Some(match name {
        "char" | "uchar" | "int8" | "uint8" => 1,
        "short" | "ushort" | "int16" | "uint16" => 2,
        "int" | "uint" | "int32" | "uint32" | "float" | "float32" => 4,
        "double" | "float64" | "int64" | "uint64" => 8,
        _ => return None,
    })
}

/// Work out the body's extent from the header text.
fn parse_shape(header: &[u8]) -> Result<FrameShape> {
    let text = String::from_utf8_lossy(header);
    let mut binary = None;
    let mut vertices = None;
    let mut stride = 0usize;
    // Properties only count toward the stride while we are inside the
    // vertex element — a header with faces after it would otherwise
    // inflate the row size and desynchronise every frame after the first.
    let mut in_vertex = false;

    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["format", kind, ..] => {
                binary = Some(match *kind {
                    "ascii" => false,
                    "binary_little_endian" => true,
                    other => bail!("unsupported PLY format for streaming: {other}"),
                });
            }
            ["element", name, count] => {
                in_vertex = *name == "vertex";
                if in_vertex {
                    let n: usize = count.parse().map_err(|_| {
                        anyhow::anyhow!("PLY vertex count is not a number: {count}")
                    })?;
                    if n > MAX_VERTICES {
                        bail!("PLY frame claims {n} vertices, over the {MAX_VERTICES} limit");
                    }
                    vertices = Some(n);
                }
            }
            ["property", "list", ..] if in_vertex => {
                // A list inside the vertex element has no fixed width, so
                // the body cannot be framed by arithmetic at all.
                bail!("PLY vertex element has a list property; cannot frame a stream");
            }
            ["property", ty, _name] if in_vertex => {
                stride += type_size(ty)
                    .ok_or_else(|| anyhow::anyhow!("unknown PLY property type: {ty}"))?;
            }
            _ => {}
        }
    }

    let Some(vertices) = vertices else {
        bail!("PLY header declares no vertex element");
    };
    match binary {
        Some(true) if stride == 0 => bail!("PLY vertex element has zero-width rows"),
        Some(true) => Ok(FrameShape { vertices, stride: Some(stride) }),
        Some(false) => Ok(FrameShape { vertices, stride: None }),
        None => bail!("PLY header has no format line"),
    }
}

/// Where a live cloud comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// Connect out to a streaming app: `host:port`.
    Connect(String),
    /// Listen and take the first sender that connects. For an app that
    /// pushes rather than serves, and for the case where vizz starts first.
    Listen(String),
    /// Re-read a file whenever it changes. Works with any app that rewrites
    /// a `.ply` in place, including over a shared folder, and needs no
    /// network at all.
    Watch(std::path::PathBuf),
}

impl std::str::FromStr for Source {
    type Err = anyhow::Error;

    /// `tcp://host:port`, `listen://host:port`, or a filesystem path.
    ///
    /// A bare `host:port` is treated as connect, since that is what a
    /// person types when they mean "the stream is over there".
    fn from_str(s: &str) -> Result<Self> {
        if let Some(rest) = s.strip_prefix("tcp://") {
            return Ok(Source::Connect(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("listen://") {
            return Ok(Source::Listen(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("file://") {
            return Ok(Source::Watch(rest.into()));
        }
        // `host:port` with a numeric port, and nothing that looks like a
        // path. A Windows path such as C:\clouds\a.ply also contains a
        // colon, which is why the port has to parse as a number.
        if let Some((host, port)) = s.rsplit_once(':')
            && !host.is_empty()
            && port.parse::<u16>().is_ok()
        {
            return Ok(Source::Connect(s.to_string()));
        }
        Ok(Source::Watch(s.into()))
    }
}

/// The latest cloud received, and a revision so the renderer can tell a
/// new frame from a repeat without comparing points.
#[derive(Default)]
struct Slot {
    points: std::sync::Mutex<Vec<Point>>,
    revision: std::sync::atomic::AtomicU64,
    connected: std::sync::atomic::AtomicBool,
    dropped: std::sync::atomic::AtomicU64,
}

/// A running live point-cloud input.
///
/// Same shape as the NDI receiver, and for the same reasons: one slot
/// rather than a queue, because the newest cloud is the only one worth
/// drawing; `try_lock` on the render side, because waiting on the network
/// to finish a copy is the stall this design exists to avoid; and
/// reconnection as the normal path, because a streaming app restarting
/// mid-set should not need vizz restarted with it.
pub struct LiveCloud {
    slot: std::sync::Arc<Slot>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    label: String,
}

impl LiveCloud {
    pub fn start(source: Source) -> Result<Self> {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        let slot = Arc::new(Slot::default());
        let stop = Arc::new(AtomicBool::new(false));
        let label = format!("{source:?}");
        let (s, st) = (Arc::clone(&slot), Arc::clone(&stop));
        let thread = std::thread::Builder::new()
            .name("ply-stream".into())
            .spawn(move || source_loop(source, &s, &st))?;
        Ok(Self { slot, stop, thread: Some(thread), label })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn connected(&self) -> bool {
        self.slot.connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn revision(&self) -> u64 {
        self.slot.revision.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Frames the reader could not hand over because the renderer held the
    /// slot. Dropping is correct; the count is worth showing.
    pub fn dropped(&self) -> u64 {
        self.slot.dropped.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Run `f` over the newest cloud, if the slot is free.
    pub fn with_latest<R>(&self, f: impl FnOnce(&[Point]) -> R) -> Option<R> {
        self.slot.points.try_lock().ok().map(|p| f(&p))
    }
}

impl Drop for LiveCloud {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn publish(slot: &std::sync::Arc<Slot>, points: Vec<Point>) {
    use std::sync::atomic::Ordering;
    let Ok(mut held) = slot.points.try_lock() else {
        slot.dropped.fetch_add(1, Ordering::Relaxed);
        return;
    };
    *held = points;
    slot.revision.fetch_add(1, Ordering::Release);
}

fn source_loop(
    source: Source,
    slot: &std::sync::Arc<Slot>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;
    while !stop.load(Ordering::Relaxed) {
        let result = match &source {
            Source::Connect(addr) => stream_from_connect(addr, slot, stop),
            Source::Listen(addr) => stream_from_listen(addr, slot, stop),
            Source::Watch(path) => watch_file(path, slot, stop),
        };
        slot.connected.store(false, Ordering::Relaxed);
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if let Err(e) = result {
            log::warn!("live cloud: {e:#} — retrying");
        }
        // A tight reconnect loop against a dead port is a busy-wait; a
        // second is short enough that restarting the sender feels instant.
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn stream_from_connect(
    addr: &str,
    slot: &std::sync::Arc<Slot>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    let stream = std::net::TcpStream::connect(addr)?;
    log::info!("live cloud: connected to {addr}");
    pump(stream, slot, stop)
}

fn stream_from_listen(
    addr: &str,
    slot: &std::sync::Arc<Slot>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    use std::sync::atomic::Ordering;
    let listener = std::net::TcpListener::bind(addr)?;
    log::info!("live cloud: listening on {addr}");
    // Non-blocking, polled against `stop`.
    //
    // A blocking `accept` was the intended behaviour for a source that has
    // not started yet, and it made quitting hang forever: `Drop` joins
    // this thread unconditionally, and the flag is only observed inside
    // `pump`. Any moment with no sender attached — before the first
    // connection or after any disconnect, since the caller re-enters here
    // — left the process alive after the window closed, still holding the
    // port so the next launch could not bind it.
    listener.set_nonblocking(true)?;
    let stream = loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                log::info!("live cloud: {peer} connected");
                break stream;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(e.into()),
        }
    };
    // Back to blocking for the transfer itself; `pump` sets its own read
    // timeout, which is what lets it notice `stop`.
    stream.set_nonblocking(false)?;
    pump(stream, slot, stop)
}

fn pump(
    stream: std::net::TcpStream,
    slot: &std::sync::Arc<Slot>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    use std::sync::atomic::Ordering;
    // Without a timeout, a sender that goes quiet without closing leaves
    // this thread blocked in read forever and the app unable to shut down.
    stream.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
    let mut reader = std::io::BufReader::new(stream);
    slot.connected.store(true, Ordering::Relaxed);
    while !stop.load(Ordering::Relaxed) {
        match read_frame(&mut reader) {
            Ok(Some(points)) => publish(slot, points),
            // Clean close: the sender is done, not broken.
            Ok(None) => return Ok(()),
            Err(e) => {
                // A read timeout is an idle sender, not a failure.
                if let Some(io) = e.downcast_ref::<std::io::Error>()
                    && matches!(
                        io.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    )
                {
                    continue;
                }
                return Err(e);
            }
        }
    }
    Ok(())
}

fn watch_file(
    path: &std::path::Path,
    slot: &std::sync::Arc<Slot>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    use std::sync::atomic::Ordering;
    let mut last = None;
    slot.connected.store(true, Ordering::Relaxed);
    while !stop.load(Ordering::Relaxed) {
        // Polled rather than watched with inotify: one stat per tick is
        // nothing, it works identically on every platform, and it works
        // over network shares where change notifications do not.
        let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if stamp.is_some() && stamp != last {
            last = stamp;
            match crate::pointcloud::load(path) {
                Ok(points) => publish(slot, points),
                // A partially written file is the normal case for a
                // writer that does not write atomically: skip this
                // revision and take the next one.
                Err(e) => log::debug!("live cloud: {} not readable yet: {e:#}", path.display()),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(33));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_frame(n: u32) -> Vec<u8> {
        let mut v = format!(
            "ply\nformat binary_little_endian 1.0\nelement vertex {n}\n\
             property float x\nproperty float y\nproperty float z\nend_header\n"
        )
        .into_bytes();
        for i in 0..n {
            for c in [i as f32, i as f32 + 0.5, i as f32 + 0.25] {
                v.extend_from_slice(&c.to_le_bytes());
            }
        }
        v
    }

    fn ascii_frame(n: u32) -> Vec<u8> {
        let mut s = format!(
            "ply\nformat ascii 1.0\nelement vertex {n}\n\
             property float x\nproperty float y\nproperty float z\nend_header\n"
        );
        for i in 0..n {
            s.push_str(&format!("{i} {i}.5 {i}.25\n"));
        }
        s.into_bytes()
    }

    /// The whole point: consecutive frames on one stream must come back
    /// as separate clouds. Reading one frame has to leave the reader
    /// exactly at the start of the next, or every frame after the first is
    /// garbage.
    #[test]
    fn back_to_back_binary_frames_are_framed_separately() {
        let mut stream = binary_frame(3);
        stream.extend(binary_frame(5));
        stream.extend(binary_frame(2));
        let mut r = Cursor::new(stream);

        let counts: Vec<usize> = std::iter::from_fn(|| read_frame(&mut r).transpose())
            .map(|f| f.unwrap().len())
            .collect();
        assert_eq!(counts, vec![3, 5, 2]);
    }

    /// ASCII is the case the file parser cannot frame: it reads to EOF, so
    /// without this the first frame would swallow the entire stream.
    #[test]
    fn back_to_back_ascii_frames_are_framed_separately() {
        let mut stream = ascii_frame(4);
        stream.extend(ascii_frame(1));
        let mut r = Cursor::new(stream);

        let first = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(first.len(), 4, "ASCII frame ran past its vertex count");
        let second = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(second.len(), 1);
        assert!(read_frame(&mut r).unwrap().is_none(), "expected clean end of stream");
    }

    /// Values must survive framing, not just counts — a framing bug that
    /// lands one byte off would still produce the right number of points.
    #[test]
    fn framed_points_keep_their_values() {
        let mut r = Cursor::new(binary_frame(2));
        let pts = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(pts[1].pos, [1.0, 1.5, 1.25]);
    }

    /// A sender closing between frames is normal shutdown.
    #[test]
    fn a_clean_end_of_stream_is_not_an_error() {
        let mut r = Cursor::new(Vec::new());
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    /// A frame cut short must fail rather than return a partial cloud that
    /// looks fine and is silently missing its far half.
    #[test]
    fn a_truncated_frame_is_an_error() {
        let full = binary_frame(10);
        let mut r = Cursor::new(full[..full.len() - 40].to_vec());
        assert!(read_frame(&mut r).is_err(), "truncated frame was accepted");

        let ascii = ascii_frame(10);
        let cut = ascii.len() - 20;
        let mut r = Cursor::new(ascii[..cut].to_vec());
        assert!(read_frame(&mut r).is_err(), "truncated ASCII frame was accepted");
    }

    /// The vertex count is attacker-controlled: it comes off a socket and
    /// sizes an allocation. A header claiming billions must be refused
    /// rather than attempted.
    #[test]
    fn an_absurd_vertex_count_is_refused_before_allocating() {
        let header = b"ply\nformat binary_little_endian 1.0\nelement vertex 4000000000\n\
                       property float x\nproperty float y\nproperty float z\nend_header\n";
        let err = read_frame(&mut Cursor::new(header.to_vec())).unwrap_err().to_string();
        assert!(err.contains("limit"), "no size guard: {err}");
    }

    /// Pointing vizz at the wrong port must fail quickly, not read an
    /// endless non-PLY stream looking for a header that never comes.
    #[test]
    fn a_stream_that_is_not_ply_gives_up() {
        let junk = vec![b'x'; MAX_HEADER * 2];
        let err = read_frame(&mut Cursor::new(junk)).unwrap_err().to_string();
        assert!(err.contains("end_header"), "no header bound: {err}");
    }

    /// Properties belonging to a later element must not count toward the
    /// vertex stride. Getting this wrong reads too many bytes per frame,
    /// which desynchronises the stream permanently after frame one.
    #[test]
    fn properties_after_the_vertex_element_do_not_widen_the_stride() {
        let shape = parse_shape(
            b"ply\nformat binary_little_endian 1.0\n\
              element vertex 2\nproperty float x\nproperty float y\nproperty float z\n\
              element face 1\nproperty uchar r\nproperty uchar g\nend_header\n",
        )
        .unwrap();
        assert_eq!(shape, FrameShape { vertices: 2, stride: Some(12) });
    }

    /// Colour widens the row, and the stride has to follow or every frame
    /// after the first starts at the wrong offset.
    #[test]
    fn colour_properties_widen_the_stride() {
        let shape = parse_shape(
            b"ply\nformat binary_little_endian 1.0\nelement vertex 7\n\
              property float x\nproperty float y\nproperty float z\n\
              property uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n",
        )
        .unwrap();
        assert_eq!(shape, FrameShape { vertices: 7, stride: Some(15) });
    }

    /// A variable-width vertex row cannot be framed by arithmetic at all,
    /// so say so rather than silently reading the wrong number of bytes.
    #[test]
    fn a_list_property_in_the_vertex_element_is_refused() {
        let err = parse_shape(
            b"ply\nformat binary_little_endian 1.0\nelement vertex 2\n\
              property float x\nproperty list uchar int idx\nend_header\n",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("list"), "wrong error: {err}");
    }

    /// Big-endian is unsupported by the file parser too; streaming must
    /// refuse it at the header rather than produce scrambled coordinates.
    #[test]
    fn big_endian_is_refused_at_the_header() {
        let err = parse_shape(
            b"ply\nformat binary_big_endian 1.0\nelement vertex 1\n\
              property float x\nend_header\n",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("binary_big_endian"), "wrong error: {err}");
    }

    /// Blank lines between ASCII frames must not be counted as vertices,
    /// or the stream slips by a cloud and never recovers.
    #[test]
    fn blank_lines_do_not_desynchronise_an_ascii_stream() {
        let mut stream = ascii_frame(2);
        stream.extend_from_slice(b"\n\n");
        stream.extend(ascii_frame(3));
        let mut r = Cursor::new(stream);
        assert_eq!(read_frame(&mut r).unwrap().unwrap().len(), 2);
        assert_eq!(read_frame(&mut r).unwrap().unwrap().len(), 3);
    }

    /// The source string is what someone types, so the parse has to match
    /// intent. A bare `host:port` means connect; a Windows path also
    /// contains a colon, which is why the port must parse as a number.
    #[test]
    fn source_strings_parse_the_way_someone_would_type_them() {
        use std::str::FromStr;
        assert!(matches!(
            Source::from_str("tcp://192.168.1.9:9000").unwrap(),
            Source::Connect(a) if a == "192.168.1.9:9000"
        ));
        assert!(matches!(
            Source::from_str("127.0.0.1:9000").unwrap(),
            Source::Connect(a) if a == "127.0.0.1:9000"
        ));
        assert!(matches!(
            Source::from_str("listen://0.0.0.0:9000").unwrap(),
            Source::Listen(a) if a == "0.0.0.0:9000"
        ));
        assert!(matches!(Source::from_str("/tmp/live.ply").unwrap(), Source::Watch(_)));
        assert!(
            matches!(Source::from_str(r"C:\clouds\live.ply").unwrap(), Source::Watch(_)),
            "a Windows path was mistaken for host:port"
        );
        assert!(matches!(Source::from_str("live.ply").unwrap(), Source::Watch(_)));
    }

    /// End to end over a real socket: a sender writing whole PLY files
    /// back to back, and the newest cloud landing in the slot.
    ///
    /// The slot keeps only the newest frame — that is the whole design —
    /// so a sender that runs ahead is *allowed* to have a frame overwritten
    /// before anyone looks at it. Sending on a timer and sampling on
    /// another timer therefore tests the two timers, not the transport: a
    /// loaded runner starts the reader late, the first frame is gone by the
    /// first sample, and the test fails on behaviour that is correct. So
    /// the reader acknowledges each frame and the sender waits for that
    /// acknowledgement, keeping exactly one frame in flight. What is left
    /// under test is the part that matters: that whole PLY files framed
    /// back to back come out whole, in order, with the right vertex counts.
    #[test]
    fn frames_arrive_over_a_tcp_socket() {
        use std::io::Write;
        use std::time::{Duration, Instant};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (ack_tx, ack_rx) = std::sync::mpsc::channel::<()>();
        let sender = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            for n in [4u32, 9, 2] {
                s.write_all(&binary_frame(n)).unwrap();
                s.flush().unwrap();
                ack_rx
                    .recv_timeout(Duration::from_secs(10))
                    .expect("the reader never picked up a frame");
            }
            // Hold the connection open briefly so the reader is not racing
            // a close it would report as a clean end of stream.
            std::thread::sleep(Duration::from_millis(200));
        });

        let live = LiveCloud::start(Source::Connect(addr)).unwrap();
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while seen.len() < 3 && Instant::now() < deadline {
            if live.revision() as usize > seen.len()
                && let Some(n) = live.with_latest(|p| p.len())
            {
                seen.push(n);
                // Release the next frame only once this one is recorded.
                ack_tx.send(()).unwrap();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        sender.join().unwrap();
        assert!(live.revision() >= 3, "only {} frames arrived", live.revision());
        assert_eq!(seen, vec![4, 9, 2], "frames arrived wrong or out of order");
    }

    /// A file rewritten in place must be picked up. This is the transport
    /// that needs no network and works over a shared folder.
    #[test]
    fn a_rewritten_file_is_picked_up() {
        let dir = std::env::temp_dir().join(format!("vizz-plystream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("live.ply");
        std::fs::write(&path, ascii_frame(3)).unwrap();

        let live = LiveCloud::start(Source::Watch(path.clone())).unwrap();
        let mut first = 0;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if let Some(n) = live.with_latest(|p| p.len())
                && n > 0
            {
                first = n;
                break;
            }
        }
        assert_eq!(first, 3, "initial file not loaded");

        // Modification time has one-second granularity on some
        // filesystems, so make the change unambiguous.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&path, ascii_frame(6)).unwrap();
        let mut second = first;
        for _ in 0..150 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if let Some(n) = live.with_latest(|p| p.len())
                && n != first
            {
                second = n;
                break;
            }
        }
        drop(live);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(second, 6, "rewrite was not picked up");
    }
}
