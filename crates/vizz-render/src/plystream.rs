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


/// Which wire format a frame is in.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Wire {
    /// Whole PLY files, back to back: the ASCII magic, a header
    /// declaring `element vertex N`, then the body.
    Ply,
    /// A count and then fixed-size points, with no header: 4-byte
    /// little-endian count, then 15 bytes per point — 3 `f32` of
    /// position followed by 3 bytes of RGB.
    Packed,
}

/// Bytes per point in the packed format: 3 × `f32` + 3 × `u8`.
const PACKED_STRIDE: usize = 15;

/// A reader with the format-sniffing bytes put back in front of it.
///
/// The three bytes have to be consumed to be recognised, and the parser
/// that follows needs to see them, so they are handed back as the head
/// of the stream rather than remembered as a special case.
type Sniffed<R> = std::io::Chain<Cursor<Vec<u8>>, R>;

/// Decide the format for a whole connection, with a blocking read.
///
/// The per-frame sniff cannot wait for a slow first byte — `fill_buf`
/// will not top up a buffer that already holds something, so asking
/// again just returns the same two bytes forever. This reads the three
/// magic bytes properly, and hands them back with the reader so nothing
/// is lost: the format cannot change mid-connection, so once is enough.
fn sniff_connection<R: BufRead>(
    mut reader: R,
    stop: &dyn Fn() -> bool,
) -> Result<Option<(Wire, Sniffed<R>)>> {
    let mut head = [0u8; 3];
    let mut filled = 0;
    while filled < head.len() {
        match reader.read(&mut head[filled..]) {
            // Nothing at all is the sender closing between frames, which
            // is normal shutdown; a partial magic is a truncated stream.
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => bail!("stream ended inside the first {filled} bytes"),
            Ok(n) => filled += n,
            Err(e) if is_timeout(&e) => {
                if stop() {
                    return Ok(None);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let wire = if &head == b"ply" { Wire::Ply } else { Wire::Packed };
    Ok(Some((wire, std::io::Read::chain(Cursor::new(head.to_vec()), reader))))
}

/// Read one packed frame: a little-endian `u32` count, then that many
/// 15-byte points.
///
/// Little-endian is stated rather than assumed from the host: both
/// platforms vizz builds for are little-endian today, so a `from_ne`
/// would pass every test here and silently corrupt every coordinate the
/// day it ran anywhere else.
fn read_packed_patient(
    reader: &mut impl BufRead,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Vec<Point>>> {
    // The count is read byte-aware rather than with `read_exact_patient`,
    // which cannot tell "the sender closed cleanly between frames" from
    // "the sender vanished mid-frame". Between frames is normal shutdown.
    let mut head = [0u8; 4];
    let mut filled = 0;
    while filled < head.len() {
        match reader.read(&mut head[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => bail!("packed stream ended inside a frame header"),
            Ok(n) => filled += n,
            Err(e) if is_timeout(&e) => {
                if stop() {
                    return Ok(None);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let count = u32::from_le_bytes(head) as usize;
    // The count comes off the wire and sizes an allocation, so it is
    // bounded the same way the PLY path bounds its vertex count — before
    // a single byte of body has arrived.
    if count > MAX_VERTICES {
        bail!("packed frame claims {count} points, over the {MAX_VERTICES} limit");
    }
    let len = count * PACKED_STRIDE;
    if len > MAX_FRAME_BYTES {
        bail!("packed frame body over the {} MiB limit", MAX_FRAME_BYTES >> 20);
    }
    let mut body = vec![0u8; len];
    if !read_exact_patient(reader, &mut body, stop)? {
        return Ok(None);
    }
    let points = body
        .chunks_exact(PACKED_STRIDE)
        .map(|c| Point {
            pos: [
                f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                f32::from_le_bytes([c[8], c[9], c[10], c[11]]),
            ],
            // A live stream carries position and colour only — the
            // packed frame format has no room for a normal and a sender
            // computing one per frame would be spending its budget in the
            // wrong place. Estimation fills these in downstream.
            normal: [0.0; 3],
            color: [c[12], c[13], c[14]],
        })
        .collect();
    Ok(Some(points))
}

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
    read_frame_patient(reader, &|| false)
}

/// [`read_frame`], riding out read timeouts without losing its place.
///
/// A read timeout used to be handled a layer up, by restarting
/// `read_frame` from scratch — which is fine between frames and fatal in
/// the middle of one: the bytes already consumed were gone, so the next
/// attempt parsed a header out of the middle of a body and every frame
/// after that was garbage. A stall over the socket timeout mid-frame
/// permanently desynced the stream. Here a timeout keeps its position
/// and simply waits more, consulting `stop`; a stop while waiting is a
/// clean end of stream, not an error.
pub fn read_frame_patient(
    reader: &mut impl BufRead,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Vec<Point>>> {
    // Which of the two wire formats is on this socket, decided per frame
    // from the bytes themselves rather than from a setting.
    //
    // Not every app that streams point clouds streams *PLY*. LOTA sends
    // a packed frame — a count, then fixed-size points — with no header
    // at all, which is the sensible thing to send and unreadable to a
    // parser looking for the ASCII magic. Sniffing means one input works
    // with both, and nobody has to know which their app speaks in order
    // to choose correctly from a menu.
    read_ply_patient(reader, stop)
}

/// Read one PLY frame, the format already decided.
fn read_ply_patient(
    reader: &mut impl BufRead,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Vec<Point>>> {
    let Some(header) = read_header(reader, stop)? else {
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
            if !read_exact_patient(reader, &mut frame[start..], stop)? {
                return Ok(None);
            }
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
                let Some(read) = read_line_patient(reader, &mut line, stop)? else {
                    return Ok(None);
                };
                if read == 0 {
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

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Fill `buf` exactly, treating a read timeout as "wait more" rather
/// than an error — the position is tracked here, so nothing is lost.
/// Returns `false` if `stop` turned true while waiting.
fn read_exact_patient(
    reader: &mut impl BufRead,
    buf: &mut [u8],
    stop: &dyn Fn() -> bool,
) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => bail!("PLY stream ended mid-frame"),
            Ok(n) => filled += n,
            Err(e) if is_timeout(&e) => {
                if stop() {
                    return Ok(false);
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(true)
}

/// `read_line` that rides out timeouts. Built on `fill_buf`/`consume`,
/// which a timeout leaves untouched — `BufRead::read_line`'s contract
/// makes no promise about partially-read bytes on error, and losing
/// them is exactly the desync this file exists to avoid.
/// `Ok(Some(n))` appended `n` bytes (0 = end of stream); `Ok(None)`
/// means `stop` turned true while waiting.
fn read_line_patient(
    reader: &mut impl BufRead,
    line: &mut String,
    stop: &dyn Fn() -> bool,
) -> Result<Option<usize>> {
    let mut bytes = Vec::new();
    loop {
        let (take, done) = match reader.fill_buf() {
            Ok([]) => (0, true),
            Ok(buf) => match buf.iter().position(|b| *b == b'\n') {
                Some(pos) => {
                    bytes.extend_from_slice(&buf[..=pos]);
                    (pos + 1, true)
                }
                None => {
                    bytes.extend_from_slice(buf);
                    (buf.len(), false)
                }
            },
            Err(e) if is_timeout(&e) => {
                if stop() {
                    return Ok(None);
                }
                (0, false)
            }
            Err(e) => return Err(e.into()),
        };
        reader.consume(take);
        if done {
            break;
        }
        // A line longer than any header has business being: bail rather
        // than buffering an unbounded stream of not-PLY. Same wording as
        // the header cap — both mean the same thing to the user.
        if bytes.len() > MAX_HEADER {
            bail!("no end_header within {MAX_HEADER} bytes — is this a PLY stream?");
        }
    }
    let n = bytes.len();
    line.push_str(&String::from_utf8_lossy(&bytes));
    Ok(Some(n))
}

/// Read up to and including `end_header`. `None` at a clean end of
/// stream, or when `stop` turned true while waiting.
fn read_header(reader: &mut impl BufRead, stop: &dyn Fn() -> bool) -> Result<Option<Vec<u8>>> {
    let mut header = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let Some(read) = read_line_patient(reader, &mut line, stop)? else {
            return Ok(None);
        };
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
    ///
    /// The lock is held for as long as `f` runs, so this is for cheap
    /// questions — how many points arrived, is there anything there. For
    /// the upload, which is neither cheap nor quick, use
    /// [`Self::take_latest`].
    pub fn with_latest<R>(&self, f: impl FnOnce(&[Point]) -> R) -> Option<R> {
        self.slot.points.try_lock().ok().map(|p| f(&p))
    }

    /// Swap the newest cloud into `buffer`, if the slot is free.
    ///
    /// Returns whether anything was taken. The lock is held for a pointer
    /// swap and nothing else — the caller then owns the points outright
    /// and can spend as long as it likes on them without the reader
    /// thread waiting.
    ///
    /// That matters because the reader publishes with `try_lock` and
    /// drops the frame when it cannot get in. Doing the whole upload
    /// inside `with_latest` means the slot is held for the duration, so
    /// every frame arriving during it is dropped: at 180 ms of work and
    /// 30 fps of input that was five frames thrown away for each one
    /// drawn. The upload is far cheaper now, but the shape was wrong
    /// either way — the next expensive thing on this path would have
    /// quietly done the same.
    ///
    /// `buffer` goes into the slot in exchange, so the two sides pass
    /// allocations back and forth rather than allocating per frame.
    pub fn take_latest(&self, buffer: &mut Vec<Point>) -> bool {
        let Ok(mut held) = self.slot.points.try_lock() else {
            return false;
        };
        std::mem::swap(&mut *held, buffer);
        true
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

/// Taking a frame must not block the reader for longer than a swap.
#[cfg(test)]
mod handover_tests {
    use super::*;

    #[test]
    fn taking_a_frame_swaps_the_buffers_rather_than_copying() {
        let slot = std::sync::Arc::new(Slot::default());
        publish(&slot, vec![Point::new(1.0, 2.0, 3.0); 4]);
        let live = LiveCloud {
            slot: std::sync::Arc::clone(&slot),
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            thread: None,
            label: "test".into(),
        };

        let mut mine = Vec::with_capacity(64);
        mine.push(Point::new(9.0, 9.0, 9.0));
        assert!(live.take_latest(&mut mine));
        assert_eq!(mine.len(), 4, "the frame did not come across");
        assert_eq!(mine[0].pos, [1.0, 2.0, 3.0]);

        // And the slot got my buffer, so the reader has somewhere to
        // write without allocating.
        let left = slot.points.lock().unwrap();
        assert_eq!(left.len(), 1, "the slot did not take the buffer in exchange");
    }

    /// The point of the exercise: a reader can publish while the caller
    /// is still working on the frame it took.
    #[test]
    fn the_reader_is_not_blocked_while_a_frame_is_being_used() {
        let slot = std::sync::Arc::new(Slot::default());
        publish(&slot, vec![Point::new(1.0, 0.0, 0.0); 2]);
        let live = LiveCloud {
            slot: std::sync::Arc::clone(&slot),
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            thread: None,
            label: "test".into(),
        };

        let mut mine = Vec::new();
        assert!(live.take_latest(&mut mine));
        // Still holding `mine` — this is where the upload happens — and
        // the reader gets in anyway. Under the old shape the slot was
        // locked for all of it and this frame was dropped.
        publish(&slot, vec![Point::new(2.0, 0.0, 0.0); 3]);
        assert_eq!(
            slot.dropped.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "the reader was blocked by a frame already handed over"
        );
        assert_eq!(mine.len(), 2, "the frame in hand was disturbed");
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
    // Dialling out there is only ever the one sender, so nothing can
    // relieve it.
    pump(stream, slot, stop, &|| false)
}

/// The most recent sender waiting on the listener, if any.
///
/// The backlog is drained rather than taken one connection at a time: if
/// several senders queued up while another was being read, the newest is
/// the one wanted, and serving the rest in turn would hand each of them
/// the stream for a moment while already stale. Dropping them closes
/// them, so a sender that lost the race is told rather than left hanging.
fn newest_waiting(
    listener: &std::net::TcpListener,
) -> Result<Option<std::net::TcpStream>> {
    let mut newest = None;
    loop {
        match listener.accept() {
            Ok((stream, peer)) => {
                log::info!("live cloud: {peer} connected");
                newest = Some(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(newest),
            Err(e) => return Err(e.into()),
        }
    }
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

    // The newest sender is the one being read, and the listener goes on
    // accepting for as long as it is bound.
    //
    // Taking the first sender and holding it until the connection ended
    // was the whole failure mode of a socket that goes silent without
    // closing — a phone that slept, an app sent to the background. Such a
    // socket is open, quiet, and from in here identical to a live sender
    // sitting between frames, so it cannot be timed out without also
    // cutting off a slow scan. It kept the one reading slot for the life
    // of the app: every later connection, the sender restarting included,
    // sat unaccepted in the backlog delivering nothing, and the only fix
    // from outside was to restart vizz. Letting the newest connection
    // take over needs no timeout and no guess about what silence means.
    let mut current = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Some(next) = newest_waiting(&listener)? {
            // Assigning drops whatever was here, which closes it.
            current = Some(next);
        }
        let Some(stream) = current.take() else {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        };
        // Back to blocking for the transfer itself; `pump` sets its own
        // read timeout, which is what lets it notice `stop`.
        stream.set_nonblocking(false)?;
        // A sender that arrives mid-transfer is parked here by the
        // closure below and picked up on the next turn of the loop.
        let waiting = std::cell::RefCell::new(None);
        let relieved = || match newest_waiting(&listener) {
            Ok(Some(next)) => {
                log::info!("live cloud: a newer sender takes over");
                *waiting.borrow_mut() = Some(next);
                true
            }
            Ok(None) => false,
            // Reported properly by the `?` at the top of the loop; here
            // it only decides whether to give up the stream in hand.
            Err(e) => {
                log::warn!("live cloud: accepting while reading: {e:#}");
                false
            }
        };
        let outcome = pump(stream, slot, stop, &relieved);
        slot.connected.store(false, Ordering::Relaxed);
        current = waiting.into_inner();
        // One sender's broken frame is not a reason to give up the port.
        // The listener outlives any single connection — another sender
        // may be waiting on it already — where returning the error meant
        // rebinding, and a rebind is the moment a sender cannot connect.
        if let Err(e) = outcome {
            log::warn!("live cloud: {e:#}");
        }
    }
}

fn pump(
    stream: std::net::TcpStream,
    slot: &std::sync::Arc<Slot>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    relieved: &dyn Fn() -> bool,
) -> Result<()> {
    use std::sync::atomic::Ordering;
    // Without a timeout, a sender that goes quiet without closing leaves
    // this thread blocked in read forever and the app unable to shut down.
    stream.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
    let reader = std::io::BufReader::new(stream);
    // Two reasons to give up the stream: the app is quitting, or a newer
    // sender has arrived and this one is no longer the live one. Both are
    // asked only where a read has already timed out or a frame has just
    // ended, so the stream is never dropped mid-frame with bytes eaten.
    let done = || stop.load(Ordering::Relaxed) || relieved();
    // Settle the format before the first frame, not per frame: a slow
    // first packet cannot be sniffed by peeking, and the answer cannot
    // change while the connection lasts.
    let Some((wire, mut reader)) = sniff_connection(reader, &done)? else {
        return Ok(());
    };
    log::info!("live cloud: reading {wire:?} frames");
    slot.connected.store(true, Ordering::Relaxed);
    loop {
        if done() {
            return Ok(());
        }
        // Timeouts are ridden out *inside* the read, holding position.
        // They used to be caught here by restarting `read_frame`, which
        // between frames was an idle sender and mid-frame threw away the
        // bytes already consumed — the next attempt parsed a header out
        // of the middle of a body, and the stream never recovered.
        let got = match wire {
            Wire::Ply => read_ply_patient(&mut reader, &done),
            Wire::Packed => read_packed_patient(&mut reader, &done),
        };
        match got {
            Ok(Some(points)) => publish(slot, points),
            // Clean close: the sender is done (or we are), not broken.
            Ok(None) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
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

    /// LOTA's wire format, read off a socket.
    ///
    /// This is the format that made the feature not work: a 4-byte
    /// little-endian count then 15 bytes a point, with no PLY header
    /// anywhere. The reader used to scan those count bytes for the ASCII
    /// magic and give up.
    #[test]
    fn a_packed_frame_reads_back_exactly() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&2u32.to_le_bytes());
        for (pos, color) in [
            ([1.5f32, -2.25, 3.0], [255u8, 0, 128]),
            ([0.0, 0.5, -1.0], [10, 20, 30]),
        ] {
            for v in pos {
                wire.extend_from_slice(&v.to_le_bytes());
            }
            wire.extend_from_slice(&color);
        }
        // Exactly the size the format claims, or one side has drifted.
        assert_eq!(wire.len(), 4 + 2 * 15);

        let (wire_kind, mut reader) = connection(wire);
        assert_eq!(wire_kind, Wire::Packed, "LOTA's format was not recognised");
        let points = read_packed_patient(&mut reader, &|| false).unwrap().expect("a frame");
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].pos, [1.5, -2.25, 3.0]);
        assert_eq!(points[0].color, [255, 0, 128]);
        assert_eq!(points[1].pos, [0.0, 0.5, -1.0]);
        assert_eq!(points[1].color, [10, 20, 30]);
    }

    /// Back-to-back packed frames stay in step. Framing is the whole
    /// problem with a stream: one byte of drift and every frame after it
    /// is noise.
    #[test]
    fn packed_frames_do_not_drift() {
        let mut wire = Vec::new();
        for n in 1..=3u32 {
            wire.extend_from_slice(&n.to_le_bytes());
            for i in 0..n {
                for v in [i as f32, 0.0, 0.0] {
                    wire.extend_from_slice(&v.to_le_bytes());
                }
                wire.extend_from_slice(&[1, 2, 3]);
            }
        }
        let (_, mut reader) = connection(wire);
        for n in 1..=3 {
            let f = read_packed_patient(&mut reader, &|| false).unwrap().expect("a frame");
            assert_eq!(f.len(), n, "frame {n} came back the wrong length");
            assert_eq!(f[n - 1].pos[0], (n - 1) as f32);
        }
        // And then a clean end, not an error.
        assert!(read_packed_patient(&mut reader, &|| false).unwrap().is_none());
    }

    /// The format is settled once per connection, from its first bytes,
    /// and a PLY sender is still read as PLY.
    ///
    /// Deciding per frame instead looks tidier and is wrong: a PLY
    /// sender may pad with blank lines between clouds, so a frame can
    /// begin "\n\np" — which is not the magic, and would send every
    /// padded frame down the packed path to be read as noise.
    #[test]
    fn the_format_is_decided_once_per_connection() {
        let ply = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n1 2 3\n";
        let (kind, mut reader) = connection(ply.to_vec());
        assert_eq!(kind, Wire::Ply);
        let f = read_ply_patient(&mut reader, &|| false).unwrap().expect("a frame");
        assert_eq!(f[0].pos, [1.0, 2.0, 3.0]);

        let mut packed = Vec::new();
        packed.extend_from_slice(&1u32.to_le_bytes());
        packed.extend_from_slice(&7.5f32.to_le_bytes());
        packed.extend_from_slice(&0f32.to_le_bytes());
        packed.extend_from_slice(&0f32.to_le_bytes());
        packed.extend_from_slice(&[9, 9, 9]);
        let (kind, mut reader) = connection(packed);
        assert_eq!(kind, Wire::Packed);
        let f = read_packed_patient(&mut reader, &|| false).unwrap().expect("a frame");
        assert_eq!(f[0].pos[0], 7.5);
        assert_eq!(f[0].color, [9, 9, 9]);
    }

    /// What `pump` does to a fresh connection, without a socket.
    fn connection(
        bytes: Vec<u8>,
    ) -> (Wire, Sniffed<std::io::BufReader<Cursor<Vec<u8>>>>) {
        let reader = std::io::BufReader::new(Cursor::new(bytes));
        sniff_connection(reader, &|| false).unwrap().expect("a connection")
    }

    /// A count off the wire sizes an allocation, so it is bounded before
    /// any body arrives — a hostile or corrupt 4 billion must be an
    /// error, not four billion points of memory.
    #[test]
    fn an_absurd_packed_count_is_refused_not_allocated() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&u32::MAX.to_le_bytes());
        let (_, mut reader) = connection(wire);
        let err = read_packed_patient(&mut reader, &|| false).expect_err("should refuse");
        assert!(
            format!("{err:#}").contains("limit"),
            "refused for the wrong reason: {err:#}"
        );
    }
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

    /// A sender that stalls mid-frame — long enough for the socket
    /// timeout to fire — must not desync the stream. The timeout used
    /// to be caught a layer up by restarting the whole frame read,
    /// which discarded the bytes already consumed; the next attempt
    /// parsed a header out of the middle of a body, and every frame
    /// after the stall was garbage.
    #[test]
    fn a_stall_mid_frame_does_not_desync_the_stream() {
        struct Stutter {
            data: Vec<u8>,
            pos: usize,
            calls: usize,
        }
        impl std::io::Read for Stutter {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                // Every other call times out — mid-header, mid-body,
                // everywhere — and data arrives five bytes at a time.
                self.calls += 1;
                if self.calls % 2 == 1 {
                    return Err(std::io::ErrorKind::WouldBlock.into());
                }
                if self.pos >= self.data.len() {
                    return Ok(0);
                }
                let n = buf.len().min(5).min(self.data.len() - self.pos);
                buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
                self.pos += n;
                Ok(n)
            }
        }
        let mut data = binary_frame(2);
        data.extend(binary_frame(3));
        let mut r = std::io::BufReader::new(Stutter { data, pos: 0, calls: 0 });
        let stop = || false;
        assert_eq!(read_frame_patient(&mut r, &stop).unwrap().unwrap().len(), 2);
        assert_eq!(read_frame_patient(&mut r, &stop).unwrap().unwrap().len(), 3);
        assert!(read_frame_patient(&mut r, &stop).unwrap().is_none());
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

    /// A sender that connects and then says nothing must not lock out the
    /// next one.
    ///
    /// This is the shape of a phone that slept or an app sent to the
    /// background: the socket stays open and silent, which from the
    /// reading end is indistinguishable from a live sender between
    /// frames. Taking the first sender and keeping it meant that socket
    /// held the only reading slot for the life of the app — the sender
    /// restarting simply queued up behind itself and delivered nothing.
    #[test]
    fn a_newer_sender_takes_over_from_a_silent_one() {
        use std::io::Write;
        use std::time::{Duration, Instant};

        // `Listen` binds inside the thread, so the port has to be known
        // before it starts rather than read back off the listener.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap().to_string();
        drop(probe);

        let live = LiveCloud::start(Source::Listen(addr.clone())).unwrap();
        // Let the listener bind before anyone knocks.
        std::thread::sleep(Duration::from_millis(200));

        // The dead sender: connected, never says a word, never closes.
        let _silent = std::net::TcpStream::connect(&addr).unwrap();
        // Long enough for the 50 ms accept poll to have taken it, so the
        // reader is genuinely holding this connection and not merely
        // finding it in the backlog alongside the next one.
        std::thread::sleep(Duration::from_millis(300));

        let mut live_sender = std::net::TcpStream::connect(&addr).unwrap();
        live_sender.write_all(&binary_frame(7)).unwrap();
        live_sender.flush().unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        while live.revision() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            live.revision() >= 1,
            "the silent sender kept the slot; nothing from the live one arrived"
        );
        assert_eq!(
            live.with_latest(|p| p.len()),
            Some(7),
            "the frame that arrived was not the live sender's"
        );
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
