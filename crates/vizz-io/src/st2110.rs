//! ST 2110 output: sends the master as an uncompressed SMPTE ST 2110-20
//! video stream on ordinary UDP sockets, to a multicast group or a unicast
//! address, for broadcast gear that takes no NDI.
//!
//! ## What goes out
//!
//! YCbCr 4:2:2 at 10 bits, BT.709 SDR in the narrow range: what broadcast
//! receivers take without being asked. The master's eight-bit values are
//! taken as R'G'B' as they stand and converted; alpha is dropped. An SDP
//! file describes the stream, and is written when the output starts: a
//! receiver needs it to join.
//!
//! Frames go out on the SMPTE Epoch, as ST 2110-10 asks: frame n at
//! n ÷ rate seconds of TAI, stamped with that instant, its packets paced
//! across the frame as an ST 2110-21 wide sender's. Time is the system
//! clock's. With the clock kept to PTP (`phc2sys` on Linux), name the
//! grandmaster and the stream lines up with every other source on the
//! network; without, the SDP file names this machine's own clock
//! (`localmac`), and the stream is steady but lined up with nobody's.
//!
//! The pacing is a thread sleeping and then spinning to each packet's
//! time, from st2110-media: it keeps one core busy, holds a wide sender's
//! limits at HD on a quiet machine, and bursts on a busy one.
//!
//! ## Why this does not stall the render thread
//!
//! Like NDI, it needs pixels on the CPU, and three threads share the work:
//!
//! - Render thread: [`ReadbackRing::capture`] encodes the GPU→CPU copy,
//!   and every copy that has finished goes into a one-picture mailbox,
//!   the newest replacing the one before. Nothing waits.
//! - Packing thread: wakes just before the send thread will want its next
//!   frame, takes the newest picture and packs it into pixel groups:
//!   about 6.5 ms for 1080 lines on one core.
//! - Send thread: sends each frame's packets at their times. It asks for a
//!   frame as the last packet of the one before goes, and gets the newest
//!   one packed, or the one it sent before when there is nothing newer.
//!
//! So the stream keeps its own rate, whatever the renderer's: ST 2110
//! wants a frame every period. Pictures from a renderer running faster
//! are left out; a renderer running slower, or held while the licence is
//! checked, has its last picture repeated. The stream freezes rather than
//! breaks up, as it would through a frame synchroniser.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use st2110_media::describe::{Clock, Description, Leg, Media};
use st2110_media::format::VideoFormat;
use st2110_media::net::{self, TAI_UTC_2017, Transmitter};
use st2110_media::pixels::{Converter, Order};
use st2110_media::send::{FrameSource, VideoSender, default_sender_type};
use st2110_ptp::PtpTime;
use st2110_ptp::epoch::{self, Signal};

use crate::FrameSender;
use crate::readback::{MappedFrame, ReadbackRing};

/// Readback slots: one the GPU copies into, one mapping, one waiting in
/// the mailbox and one being packed.
const DEPTH: usize = 4;
/// The RTP payload type: the first dynamic one, which video usually takes.
const PAYLOAD_TYPE: u8 = 96;
/// Multicast time to live, as `st2110 send` uses: enough for a routed plant.
const TTL: u8 = 32;
/// AF41, which AES67 and most ST 2110 plants mark media with.
const DSCP: u8 = 34;
const NANOS: i128 = 1_000_000_000;
const MS: i128 = 1_000_000;

/// What an ST 2110 output sends, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct St2110Options {
    /// Where the stream goes: a multicast group or a unicast address, and
    /// a port. Two for the legs of an ST 2022-7 pair.
    pub destinations: Vec<SocketAddrV4>,
    /// The address of the network interface to send from: one for every
    /// leg, or one each. The routing table's choice when empty.
    pub interfaces: Vec<Ipv4Addr>,
    /// Frames a second: `50`, `59.94` or `60000/1001`.
    pub rate: String,
    /// The reference clock the SDP file names: `traceable`,
    /// `<grandmaster>:<domain>` or `localmac=<MAC address>`. This
    /// machine's MAC address, `localmac`, when `None`.
    pub clock: Option<String>,
    /// The session name in the SDP file, which receivers list.
    pub name: String,
    /// Where to write the SDP file.
    pub sdp: Option<PathBuf>,
}

pub struct St2110Sender {
    name: String,
    width: u32,
    height: u32,
    ring: ReadbackRing,
    shared: Arc<Shared>,
    packer: Option<JoinHandle<()>>,
    sender: Option<JoinHandle<()>>,
    sdp: String,
}

/// A picture read back from the GPU, waiting to be packed.
struct Picture {
    frame: MappedFrame,
    order: Order,
}

/// The newest packed frame, and whether the send thread has had it yet.
struct Packed {
    frame: Vec<u8>,
    fresh: bool,
}

/// What the render, packing and send threads share.
struct Shared {
    mailbox: Mutex<Option<Picture>>,
    packed: Mutex<Packed>,
    stop: AtomicBool,
    /// Why sending stopped, when it stopped by itself.
    failure: Mutex<Option<String>>,
    /// Frames sent again because nothing newer was packed in time.
    repeated: AtomicU64,
}

impl St2110Sender {
    /// Start sending a `width`x`height` master as `options` say, and write
    /// the SDP file. Fails, without sending, when the stream cannot be
    /// sent: a size ST 2110-20 has no pixel groups for, a rate it does
    /// not know, a destination no socket can reach.
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        options: &St2110Options,
    ) -> Result<Self> {
        let Stream { description, transmitter, sender, converter, sdp } =
            Stream::open(width, height, options)?;
        let ring = ReadbackRing::new(device, width, height, DEPTH)?;

        // Black until the first picture is packed: one row, repeated.
        let format = sender.format().clone();
        let mut row = vec![0; format.row_bytes()];
        converter.pack_row_in(&vec![0; width as usize * 4], Order::BGRA, &mut row);
        let black = row.repeat(height as usize);

        let shared = Arc::new(Shared {
            mailbox: Mutex::new(None),
            packed: Mutex::new(Packed { frame: black.clone(), fresh: false }),
            stop: AtomicBool::new(false),
            failure: Mutex::new(None),
            repeated: AtomicU64::new(0),
        });
        let timing = Timing::new(&sender);
        let packer = {
            let shared = Arc::clone(&shared);
            let back = vec![0; format.frame_bytes()];
            thread::Builder::new()
                .name("vizz-st2110-pack".into())
                .spawn(move || pack_loop(&shared, &converter, timing, back))?
        };
        let send = {
            let shared = Arc::clone(&shared);
            let packer = packer.thread().clone();
            let name = options.name.clone();
            thread::Builder::new().name("vizz-st2110-send".into()).spawn(move || {
                send_loop(&shared, sender, transmitter, black, &packer, &name);
            })
        };
        let send = match send {
            Ok(send) => send,
            Err(e) => {
                shared.stop.store(true, Ordering::Relaxed);
                packer.thread().unpark();
                let _ = packer.join();
                return Err(e.into());
            }
        };

        let legs: Vec<String> = description.legs.iter().map(|l| l.to_string()).collect();
        log::info!(
            "ST 2110 '{}': {format}, {:.2} Gb/s to {}; SDP file {}",
            options.name,
            format.bitrate() / 1e9,
            legs.join(" and "),
            options.sdp.as_ref().map_or("not written".into(), |p| p.display().to_string()),
        );
        log::debug!("ST 2110 '{}' SDP:\n{sdp}", options.name);
        Ok(Self {
            name: options.name.clone(),
            width,
            height,
            ring,
            shared,
            packer: Some(packer),
            sender: Some(send),
            sdp,
        })
    }

    /// The SDP file that describes the stream.
    pub fn sdp(&self) -> &str {
        &self.sdp
    }

    /// Frames dropped by the readback ring (GPU behind), and frames sent
    /// again because no newer picture was packed in time (renderer
    /// slower than the stream, or held).
    pub fn dropped(&self) -> (u64, u64) {
        (self.ring.dropped(), self.shared.repeated.load(Ordering::Relaxed))
    }
}

/// The stream before any thread starts: format, sockets and SDP file.
/// Needs no GPU, so everything that can go wrong with the options is found
/// here.
struct Stream {
    description: Description,
    transmitter: Transmitter,
    sender: VideoSender,
    converter: Converter,
    sdp: String,
}

impl Stream {
    fn open(width: u32, height: u32, options: &St2110Options) -> Result<Self> {
        let legs = options.destinations.len();
        if !(1..=2).contains(&legs) {
            bail!("ST 2110 takes one destination, or two for an ST 2022-7 pair, not {legs}");
        }
        if options.interfaces.len() > legs {
            bail!(
                "ST 2110 takes an interface for each destination at most, not {}",
                options.interfaces.len()
            );
        }
        let format = video_format(width, height, &options.rate)?;
        let interfaces: Vec<Option<Ipv4Addr>> = (0..legs)
            .map(|i| options.interfaces.get(i).or(options.interfaces.last()).copied())
            .collect();
        let mut description = Description {
            name: options.name.clone(),
            media: Media::Video(format.clone()),
            payload_type: PAYLOAD_TYPE,
            legs: options
                .destinations
                .iter()
                .map(|&destination| Leg { destination, source: None })
                .collect(),
            clock: None,
            ttl: TTL,
        };
        let transmitter = Transmitter::new(&description.legs, &interfaces, TTL, DSCP, TAI_UTC_2017)
            .context("cannot open a socket to send ST 2110 from")?;
        for (leg, &source) in description.legs.iter_mut().zip(transmitter.sources()) {
            leg.source = (!source.is_unspecified()).then_some(source);
        }
        description.clock = Some(match &options.clock {
            Some(text) => Clock::parse(text).map_err(|e| anyhow!("ST 2110 clock: {e}"))?,
            None => Clock::LocalMac(local_mac(transmitter.sources()[0]).ok_or_else(|| {
                anyhow!(
                    "no MAC address found to name this machine's clock in the ST 2110 SDP file: \
                     name the reference clock instead, as traceable, <grandmaster>:<domain> or \
                     localmac=<MAC address>"
                )
            })?),
        });
        description.check().map_err(anyhow::Error::msg)?;
        let sender = VideoSender::new(&description, random() as u32, random() as u32)
            .map_err(anyhow::Error::msg)?;
        let converter = Converter::new(&format).map_err(anyhow::Error::msg)?;
        let session = u64::try_from(net::tai_now(0) / NANOS).unwrap_or(0);
        let sdp = description.sdp(session);
        if let Some(path) = &options.sdp {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                fs::create_dir_all(dir)
                    .with_context(|| format!("cannot make {}", dir.display()))?;
            }
            fs::write(path, &sdp)
                .with_context(|| format!("cannot write the ST 2110 SDP file {}", path.display()))?;
        }
        Ok(Self { description, transmitter, sender, converter, sdp })
    }
}

/// The format a `width`x`height` master goes out as at `rate`: YCbCr 4:2:2
/// at 10 bits, and a wide sender where ST 2110-21 defines one.
fn video_format(width: u32, height: u32, rate: &str) -> Result<VideoFormat> {
    let rate = rate.trim();
    // The rate alone first, on a size that is surely fine, so that a
    // mistyped rate is not reported as a bad format name.
    if VideoFormat::from_name(&format!("1920x1080p{rate}")).is_err() {
        bail!("{rate} is not a frame rate ST 2110 can send, such as 50, 59.94 or 60000/1001");
    }
    let mut format = VideoFormat::from_name(&format!("{width}x{height}p{rate}"))
        .map_err(|e| anyhow!("ST 2110: {e}"))?;
    format.sender_type = default_sender_type(&format).map_err(anyhow::Error::msg)?;
    Ok(format)
}

/// Where octets sit in a pixel of a texture this output can read back.
fn order_of(format: wgpu::TextureFormat) -> Result<Order> {
    use wgpu::TextureFormat as F;
    match format {
        F::Bgra8Unorm | F::Bgra8UnormSrgb => Ok(Order::BGRA),
        F::Rgba8Unorm | F::Rgba8UnormSrgb => Ok(Order::RGBA),
        other => bail!("ST 2110 output takes an eight-bit BGRA or RGBA master, not {other:?}"),
    }
}

/// A number that differs from one start to the next, for the SSRC and the
/// first sequence number: RFC 3550 asks for random ones, so that a
/// receiver can tell a restarted stream from the old one.
fn random() -> u64 {
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut hasher = std::hash::RandomState::new().build_hasher();
    hasher.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.finish()
}

/// A network interface, as far as naming this machine's clock goes.
#[derive(Clone)]
struct Interface {
    name: String,
    mac: [u8; 6],
    addresses: Vec<IpAddr>,
}

/// The MAC address of the interface `source` belongs to, or failing that
/// of the first interface by name that has one, written as `localmac=`
/// wants it.
fn local_mac(source: Ipv4Addr) -> Option<String> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let interfaces = networks
        .iter()
        .map(|(name, data)| Interface {
            name: name.clone(),
            mac: data.mac_address().0,
            addresses: data.ip_networks().iter().map(|n| n.addr).collect(),
        })
        .collect();
    pick_mac(interfaces, source)
}

fn pick_mac(mut interfaces: Vec<Interface>, source: Ipv4Addr) -> Option<String> {
    // Loopback has none. By name, so that the SDP file names the same
    // clock from one start to the next.
    interfaces.retain(|i| i.mac != [0; 6]);
    interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    let chosen = interfaces
        .iter()
        .find(|i| i.addresses.contains(&IpAddr::V4(source)))
        .or(interfaces.first())?;
    let m = chosen.mac;
    Some(format!("{:02X}-{:02X}-{:02X}-{:02X}-{:02X}-{:02X}", m[0], m[1], m[2], m[3], m[4], m[5]))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A thread that panicked holding it has already stopped the output.
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// When the send thread will ask for each frame, which is when the newest
/// picture should have just been packed.
///
/// It asks for frame n once it has sent the last packet of frame n − 1,
/// which the ST 2110-21 schedule puts a fixed time after that frame's
/// alignment point.
#[derive(Clone, Copy, Debug)]
struct Timing {
    signal: Signal,
    /// From a frame's alignment point to the ask for the next, in ns.
    ask: i128,
    /// A frame, in ns.
    period: i128,
}

impl Timing {
    fn new(sender: &VideoSender) -> Self {
        let rate = sender.format().rate;
        Self {
            signal: Signal::Video(rate),
            ask: sender.schedule().send_offset(sender.packets() - 1),
            period: (1e9 / rate.to_f64()).round() as i128,
        }
    }

    /// The first time after `now` that is `lead` before an ask.
    fn wake(&self, now: i128, lead: i128) -> i128 {
        match PtpTime::from_nanos(now + lead - self.ask)
            .and_then(|t| epoch::next_alignment(t, self.signal))
        {
            Some((_, start)) => start.nanos() + self.ask - lead,
            // A clock outside PTP's range: nothing lines up anyway.
            None => now + self.period,
        }
    }

    /// How long before an ask to start packing: half as long again as the
    /// slowest pack lately, and a millisecond to wake, within a frame.
    fn lead(&self, slowest: i128) -> i128 {
        (slowest * 3 / 2 + MS).clamp(2 * MS, (self.period - MS).max(2 * MS))
    }
}

/// Packs the newest picture just before each ask, so that what goes out
/// is as new as it can be and pictures nobody will send are not packed.
fn pack_loop(shared: &Shared, converter: &Converter, timing: Timing, mut back: Vec<u8>) {
    // A guess until the first pack is timed.
    let mut slowest = timing.period / 4;
    loop {
        let lead = timing.lead(slowest);
        let wake = timing.wake(net::tai_now(TAI_UTC_2017), lead);
        // Parked rather than asleep, so that stopping wakes it at once.
        loop {
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            let left = wake - net::tai_now(TAI_UTC_2017);
            if left <= 0 {
                break;
            }
            thread::park_timeout(Duration::from_nanos(left as u64));
        }
        let Some(picture) = lock(&shared.mailbox).take() else {
            continue;
        };
        let started = Instant::now();
        let packed = picture.frame.with_bytes(|bytes| {
            converter.pack_frame(bytes, picture.frame.stride as usize, picture.order, &mut back);
        });
        // Gives the slot back to the renderer.
        drop(picture);
        if let Err(e) = packed {
            log::error!("ST 2110: could not read a frame back: {e:#}");
            continue;
        }
        slowest = (started.elapsed().as_nanos() as i128).max(slowest - slowest / 16);
        let mut latest = lock(&shared.packed);
        std::mem::swap(&mut latest.frame, &mut back);
        latest.fresh = true;
    }
}

/// Gives the send thread the newest packed frame, or the one it sent
/// before when nothing newer is ready.
struct Latest<'a> {
    shared: &'a Shared,
    current: Vec<u8>,
}

impl FrameSource for Latest<'_> {
    fn frame(&mut self, _n: u64) -> Option<&[u8]> {
        if self.shared.stop.load(Ordering::Relaxed) {
            return None;
        }
        let mut packed = lock(&self.shared.packed);
        if packed.fresh {
            std::mem::swap(&mut packed.frame, &mut self.current);
            packed.fresh = false;
        } else {
            self.shared.repeated.fetch_add(1, Ordering::Relaxed);
        }
        drop(packed);
        Some(&self.current)
    }
}

/// Sends until stopped, or until a socket fails; then says why, and stops
/// the packing thread too.
fn send_loop(
    shared: &Shared,
    mut sender: VideoSender,
    mut transmitter: Transmitter,
    black: Vec<u8>,
    packer: &Thread,
    name: &str,
) {
    log::info!("ST 2110 send thread for '{name}' started");
    // A moment's grace for receivers reading the SDP file, as `st2110 send`
    // gives.
    let start = net::tai_now(TAI_UTC_2017) + NANOS / 10;
    let mut source = Latest { shared, current: black };
    match sender.run(&mut transmitter, start, i128::MAX, &mut source) {
        Ok(sent) => {
            let t = transmitter.counts();
            log::info!(
                "ST 2110 send thread for '{name}' stopped: {} frames sent, {} of them again; \
                 {} left out late; {} packets late, {} refused",
                sent.frames,
                shared.repeated.load(Ordering::Relaxed),
                sent.skipped,
                t.late,
                t.refused,
            );
        }
        Err(e) => {
            log::error!("ST 2110 '{name}': sending stopped: {e}");
            *lock(&shared.failure) = Some(e.to_string());
            shared.stop.store(true, Ordering::Relaxed);
            packer.unpark();
        }
    }
}

impl FrameSender for St2110Sender {
    fn name(&self) -> &str {
        &self.name
    }

    fn publish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Result<()> {
        let ended = |t: &Option<JoinHandle<()>>| t.as_ref().is_none_or(|t| t.is_finished());
        if ended(&self.packer) || ended(&self.sender) {
            let why = lock(&self.shared.failure).clone();
            bail!("ST 2110 output stopped: {}", why.as_deref().unwrap_or("a thread ended"));
        }
        let order = order_of(texture.format())?;
        let size = texture.size();
        if (size.width, size.height) != (self.width, self.height) {
            bail!(
                "the master is {}x{} and the ST 2110 stream {}x{}",
                size.width,
                size.height,
                self.width,
                self.height
            );
        }

        // 1. Enqueue this frame's GPU→CPU copy; a full ring drops it here.
        self.ring.capture(device, queue, texture);

        // 2. Everything finished goes to the mailbox, oldest first, so the
        //    newest is what stays there.
        while let Some(frame) = self.ring.take_ready() {
            let older = lock(&self.shared.mailbox).replace(Picture { frame, order });
            // Unmapped here, outside the lock.
            drop(older);
        }
        Ok(())
    }
}

impl Drop for St2110Sender {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.packer.take() {
            t.thread().unpark();
            let _ = t.join();
        }
        // It stops when it next asks for a frame: within a frame period.
        if let Some(t) = self.sender.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;
    use std::sync::mpsc;

    use st2110_media::video::{Depacketiser, FrameInfo};

    const W: u32 = 64;
    const H: u32 = 32;

    fn options(destination: SocketAddrV4) -> St2110Options {
        St2110Options {
            destinations: vec![destination],
            interfaces: Vec::new(),
            rate: "50".into(),
            clock: Some("traceable".into()),
            name: "vizz test".into(),
            sdp: None,
        }
    }

    /// Headless GPU (lavapipe in CI), or `None` without one — unless CI
    /// set `VIZZ_REQUIRE_GPU=1`, as `readback`'s tests explain.
    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let found = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .ok()
        .and_then(|adapter| {
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("st2110-test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))
            .ok()
        });
        if found.is_none() {
            assert!(
                std::env::var_os("VIZZ_REQUIRE_GPU").is_none(),
                "VIZZ_REQUIRE_GPU is set but no GPU adapter was found"
            );
            eprintln!("no GPU adapter available; skipping GPU test");
        }
        found
    }

    fn master(device: &wgpu::Device, width: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("st2110-test-master"),
            size: wgpu::Extent3d { width, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// B', G', R' and alpha, different in every pixel, so that a row out of
    /// place or octets in the wrong order show.
    fn picture(seed: u8) -> Vec<u8> {
        (0..H as u8)
            .flat_map(|y| {
                (0..W as u8).flat_map(move |x| {
                    [
                        x.wrapping_mul(4) ^ seed,
                        y.wrapping_mul(8).wrapping_add(seed),
                        x.wrapping_add(y).wrapping_mul(3),
                        255,
                    ]
                })
            })
            .collect()
    }

    fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, bgra: &[u8]) {
        queue.write_texture(
            texture.as_image_copy(),
            bgra,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: None,
            },
            texture.size(),
        );
    }

    /// The frame of pixel groups a BGRA picture should arrive as, packed the
    /// crate's reference way, from R'G'B'.
    fn packed(format: &VideoFormat, bgra: &[u8]) -> Vec<u8> {
        let rgb: Vec<u8> =
            bgra.as_chunks::<4>().0.iter().flat_map(|&[b, g, r, _]| [r, g, b]).collect();
        st2110_media::pixels::from_rgb(format, &rgb).unwrap()
    }

    /// Receives the stream and passes on every whole frame, stamped with when
    /// its first packet arrived.
    fn receive(
        socket: UdpSocket,
        format: VideoFormat,
        stop: Arc<AtomicBool>,
        frames: mpsc::Sender<(FrameInfo, Vec<u8>)>,
    ) {
        socket.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut depacketiser = Depacketiser::new(&format).unwrap();
        let mut buffer = [0; 2048];
        while !stop.load(Ordering::Relaxed) {
            let Ok(n) = socket.recv(&mut buffer) else {
                continue;
            };
            depacketiser.push(net::tai_now(TAI_UTC_2017), &buffer[..n], |info, frame| {
                if info.whole {
                    let _ = frames.send((*info, frame.to_vec()));
                }
            });
        }
    }

    /// What goes out is what was rendered: black before the first picture,
    /// then each picture as published, never an older one after a newer,
    /// each frame stamped with its alignment point on the SMPTE Epoch.
    #[test]
    fn the_stream_carries_the_masters_pictures_on_the_epoch() {
        let Some((device, queue)) = gpu() else { return };
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = socket.local_addr().unwrap().port();
        let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        let mut sender = St2110Sender::new(&device, W, H, &options(to)).unwrap();
        let (description, _) = Description::parse(sender.sdp()).unwrap();
        let Media::Video(format) = description.media else { panic!("{}", sender.sdp()) };

        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let receiver = {
            let (stop, format) = (Arc::clone(&stop), format.clone());
            thread::spawn(move || receive(socket, format, stop, tx))
        };

        let texture = master(&device, W, wgpu::TextureFormat::Bgra8UnormSrgb);
        let pictures = [vec![0; (W * H * 4) as usize], picture(0), picture(0x5A)];
        let wanted: Vec<Vec<u8>> = pictures.iter().map(|p| packed(&format, p)).collect();
        let mut seen: Vec<(FrameInfo, usize)> = Vec::new();
        for (which, bgra) in pictures.iter().enumerate() {
            // Black is what goes out before anything is published.
            if which > 0 {
                upload(&queue, &texture, bgra);
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            'arrival: loop {
                if which > 0 {
                    sender.publish(&device, &queue, &texture).unwrap();
                    let _ = device.poll(wgpu::PollType::Poll);
                }
                while let Ok((info, frame)) = rx.try_recv() {
                    let Some(kind) = wanted.iter().position(|w| *w == frame) else {
                        panic!("frame {} is none of the pictures", info.timestamp);
                    };
                    seen.push((info, kind));
                    if kind == which {
                        break 'arrival;
                    }
                }
                let kinds: Vec<usize> = seen.iter().map(|s| s.1).collect();
                assert!(Instant::now() < deadline, "picture {which} never came; saw {kinds:?}");
                thread::sleep(Duration::from_millis(5));
            }
        }

        let kinds: Vec<usize> = seen.iter().map(|s| s.1).collect();
        assert!(kinds.is_sorted(), "an older picture came after a newer one: {kinds:?}");
        for (info, _) in &seen {
            // The timestamp is the frame's alignment point, which its first
            // packet follows by less than a frame on a quiet machine: a
            // second's slack, for a busy runner, still tells it from a
            // stamp off by TAI − UTC.
            let arrival = PtpTime::from_nanos(info.first_arrival).unwrap();
            let offset = epoch::rtp_offset(info.timestamp, arrival, 90_000);
            assert!((-90_000..=0).contains(&offset), "stamped {offset} ticks from its arrival");
        }
        for pair in seen.windows(2) {
            // A frame is 1800 ticks of 90 kHz at 50 Hz.
            let step = pair[1].0.timestamp.wrapping_sub(pair[0].0.timestamp);
            assert!(step > 0 && step % 1800 == 0, "timestamps stepped by {step}");
        }

        let stopping = Instant::now();
        drop(sender);
        let took = stopping.elapsed();
        assert!(took < Duration::from_secs(1), "stopping took {took:?}");
        stop.store(true, Ordering::Relaxed);
        receiver.join().unwrap();
    }

    /// A master the stream was not made for is an error for the output, not
    /// a copy out of bounds or a float read as bytes.
    #[test]
    fn a_master_it_was_not_made_for_is_refused() {
        let Some((device, queue)) = gpu() else { return };
        let sink = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, sink.local_addr().unwrap().port());
        let mut sender = St2110Sender::new(&device, W, H, &options(to)).unwrap();

        let wider = master(&device, W * 2, wgpu::TextureFormat::Bgra8UnormSrgb);
        let e = sender.publish(&device, &queue, &wider).unwrap_err().to_string();
        assert!(e.contains("128x32") && e.contains("64x32"), "{e}");

        let float = master(&device, W, wgpu::TextureFormat::Rgba16Float);
        let e = sender.publish(&device, &queue, &float).unwrap_err().to_string();
        assert!(e.contains("Rgba16Float"), "{e}");
    }

    #[test]
    fn options_are_checked_before_anything_is_sent() {
        let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5004);
        let error = |width: u32, change: &dyn Fn(&mut St2110Options)| {
            let mut o = options(to);
            change(&mut o);
            match Stream::open(width, H, &o) {
                Ok(_) => panic!("{o:?} at {width}x{H} was taken"),
                Err(e) => format!("{e:#}"),
            }
        };
        let e = error(W, &|o| o.destinations = vec![to; 3]);
        assert!(e.contains("one destination, or two"), "{e}");
        let e = error(W, &|o| o.destinations.clear());
        assert!(e.contains("one destination, or two"), "{e}");
        let e = error(W, &|o| o.interfaces = vec![Ipv4Addr::LOCALHOST; 2]);
        assert!(e.contains("an interface for each destination"), "{e}");
        let e = error(W, &|o| o.rate = "fifty".into());
        assert!(e.contains("fifty is not a frame rate"), "{e}");
        let e = error(W - 1, &|_| {});
        assert!(e.contains("pixel groups"), "{e}");
        let e = error(W, &|o| o.clock = Some("tomorrow".into()));
        assert!(e.contains("ST 2110 clock"), "{e}");
        // Two legs to one place are no ST 2022-7 pair.
        let e = error(W, &|o| o.destinations = vec![to, to]);
        assert!(e.contains("ST 2022-7"), "{e}");
    }

    #[test]
    fn the_sdp_file_says_what_goes_out() {
        let dir = std::env::temp_dir().join(format!("vizz-st2110-{:x}", random()));
        let path = dir.join("show").join("st2110.sdp");
        let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5004);
        let o = St2110Options { rate: "59.94".into(), sdp: Some(path.clone()), ..options(to) };
        let stream = Stream::open(1280, 720, &o).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(written, stream.sdp);
        let (description, _) = Description::parse(&written).unwrap();
        let Media::Video(format) = &description.media else { panic!("{written}") };
        assert_eq!(format.to_string(), "1280x720p59.94 YCbCr-4:2:2 10-bit");
        assert_eq!(format.colorimetry, "BT709");
        assert_eq!(description.name, "vizz test");
        assert_eq!(description.legs[0].destination, to);
        assert_eq!(description.clock, Some(Clock::Traceable));
        assert!(written.contains("TP=2110TPW"), "not a wide sender:\n{written}");
    }

    #[test]
    fn names_the_clock_after_the_interface_it_sends_from() {
        let interface = |name: &str, mac: [u8; 6], address: &str| Interface {
            name: name.into(),
            mac,
            addresses: vec![address.parse().unwrap()],
        };
        let all = vec![
            interface("lo", [0; 6], "127.0.0.1"),
            interface("eth1", [0xCA, 0xFE, 0, 0, 0, 2], "10.0.0.2"),
            interface("eth0", [0xCA, 0xFE, 0, 0, 0, 1], "192.168.1.10"),
        ];
        let pick = |source: &str| pick_mac(all.clone(), source.parse().unwrap());
        assert_eq!(pick("10.0.0.2").as_deref(), Some("CA-FE-00-00-00-02"));
        // Loopback has no MAC address: the first interface by name that has.
        assert_eq!(pick("127.0.0.1").as_deref(), Some("CA-FE-00-00-00-01"));
        assert_eq!(pick_mac(all[..1].to_vec(), Ipv4Addr::LOCALHOST), None);
        // Written as the SDP file's localmac wants it.
        let mac = pick("10.0.0.2").unwrap();
        assert_eq!(Clock::parse(&format!("localmac={mac}")), Ok(Clock::LocalMac(mac)));
    }

    #[test]
    fn packs_a_lead_before_each_ask() {
        let rate = VideoFormat::from_name("1080p50").unwrap().rate;
        let timing = Timing { signal: Signal::Video(rate), ask: 21 * MS, period: 20 * MS };
        // Seven milliseconds into a frame.
        let now = 1_790_510_437 * NANOS + 7 * MS;
        let lead = 5 * MS;
        let wake = timing.wake(now, lead);
        assert!(wake > now && wake - now <= timing.period, "{wake} from {now}");
        // An alignment point plus the ask, less the lead.
        assert_eq!((wake + lead - timing.ask).rem_euclid(timing.period), 0);
        // And from there, the next ask, not the same one again.
        assert_eq!(timing.wake(wake, lead), wake + timing.period);

        // Leads stay inside a frame however slow packing is.
        assert_eq!(timing.lead(0), 2 * MS);
        assert_eq!(timing.lead(4 * MS), 7 * MS);
        assert_eq!(timing.lead(100 * MS), 19 * MS);
    }

    #[test]
    fn reads_eight_bit_bgra_and_rgba_masters() {
        use wgpu::TextureFormat as F;
        assert_eq!(order_of(F::Bgra8UnormSrgb).unwrap(), Order::BGRA);
        assert_eq!(order_of(F::Bgra8Unorm).unwrap(), Order::BGRA);
        assert_eq!(order_of(F::Rgba8UnormSrgb).unwrap(), Order::RGBA);
        let e = order_of(F::Rgba16Float).unwrap_err().to_string();
        assert!(e.contains("Rgba16Float"), "{e}");
    }
}
