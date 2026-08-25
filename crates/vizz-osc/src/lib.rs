//! OSC input: a background UDP listener that writes straight into the
//! [`ParamRegistry`].
//!
//! The listener thread never touches the renderer — it only stores atomic
//! target values, so a flood of OSC traffic cannot stall a frame. Malformed
//! packets and unknown addresses are logged and dropped; the show goes on.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use rosc::{OscPacket, OscType};
use vizz_params::ParamRegistry;

/// The address a Resolume column launch arrives on, as
/// `/composition/columns/<n>/connect`.
const RESOLUME_COLUMNS: &str = "/composition/columns/";
const RESOLUME_CONNECT: &str = "/connect";

/// And a Resolume deck change, as `/composition/decks/<n>/select`.
///
/// The two together are the whole sync: a deck is the song, a column is
/// the section, and both programs following both means one launch in Arena
/// moves the video, the lights and the field to the same place.
const RESOLUME_DECKS: &str = "/composition/decks/";
const RESOLUME_SELECT: &str = "/select";

/// Where a followed column lands. See [`ColumnSync`].
pub const COLUMN_FIRE: &str = "/column/fire";
/// Where a followed deck lands.
pub const DECK_SELECT: &str = "/deck/select";

/// Everything the listener needs to know to follow Resolume's columns.
///
/// Three atomics rather than a lock, for the reason the module header
/// gives: this thread must never be able to stall a frame, and a mutex
/// shared with the render thread is exactly how that happens. Nothing here
/// has two writers — the app stores `enabled` and `origin`, the listener
/// only reads them; the listener bumps `fires`, the app only reads it — so
/// there is no state a torn update could produce.
#[derive(Debug, Default)]
pub struct ColumnSync {
    /// Whether Resolume's column launches drive this app at all.
    ///
    /// Off unless asked for. The listener binds every interface by
    /// default, and following columns turns any packet from anyone on the
    /// venue's wifi into a scene change on both grids at once.
    pub enabled: AtomicBool,
    /// Which Resolume column the live deck's column 1 follows, 1-based.
    /// Mirrored here by the app on every deck change; see
    /// `vizz_mod::deck::Deck::origin`.
    pub origin: AtomicU32,
    /// How many columns have been accepted, ever.
    ///
    /// Firing is edge triggered on the slot number, which is right for a
    /// pad — pressing 5 twice in a row is one move — and wrong for a
    /// column. Relaunching the same Resolume column is a deliberate
    /// re-trigger and has to land, so the render thread watches this
    /// counter and re-arms the grids when it moves. A counter rather than
    /// a flag because the render thread must not have to clear anything:
    /// a flag it failed to clear before the next packet would swallow a
    /// launch, and it is sampled at frame rate against a source that is
    /// not.
    pub fires: AtomicU32,
}

impl ColumnSync {
    /// The vizz column a Resolume column maps to, 1-based, or `None` when
    /// this deck does not cover it.
    ///
    /// Out of range is silence rather than a clamp. Clamping would make
    /// every column past the end fire the last pad — a whole song's worth
    /// of launches all landing on scene 16 — which reads as the feature
    /// being broken rather than as the deck not covering that stretch.
    fn column_for(&self, resolume: u32, slots: u32) -> Option<u32> {
        let origin = self.origin.load(Ordering::Relaxed).max(1);
        let slot = resolume.checked_sub(origin)?.checked_add(1)?;
        (slot <= slots).then_some(slot)
    }
}

/// Handle to the OSC listener thread. Dropping it shuts the thread down.
pub struct OscServer {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    local_addr: SocketAddr,
}

impl OscServer {
    /// Bind a UDP socket and start the listener thread.
    pub fn spawn(
        registry: Arc<ParamRegistry>,
        columns: Arc<ColumnSync>,
        bind: impl ToSocketAddrs,
    ) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
        // Short timeout so the thread notices the stop flag promptly.
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        let local_addr = socket.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));

        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("vizz-osc".into())
            .spawn(move || {
                log::info!("OSC listening on {local_addr}");
                // The full UDP maximum, not rosc's 1536-byte "MTU". A
                // datagram larger than the buffer is silently truncated
                // by recv_from and then fails to decode, so a TouchOSC
                // page sending one bundle of a few dozen faders simply
                // never arrived. 64 KiB once on a worker thread's stack
                // is nothing; the messages it saves are real.
                let mut buf = [0u8; 65_507];
                while !thread_stop.load(Ordering::Relaxed) {
                    match socket.recv_from(&mut buf) {
                        Ok((n, _peer)) => match rosc::decoder::decode_udp(&buf[..n]) {
                            Ok((_, packet)) => apply_packet(&registry, &columns, packet),
                            Err(e) => log::warn!("dropped malformed OSC packet: {e}"),
                        },
                        Err(e)
                            if e.kind() == io::ErrorKind::WouldBlock
                                || e.kind() == io::ErrorKind::TimedOut => {}
                        Err(e) => {
                            // Transient socket errors must not kill control input.
                            log::error!("OSC socket error: {e}");
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
                log::info!("OSC listener stopped");
            })?;

        Ok(Self {
            stop,
            handle: Some(handle),
            local_addr,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl Drop for OscServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Route a decoded packet into the registry. Bundles recurse; each message's
/// first numeric argument becomes the parameter value.
fn apply_packet(registry: &ParamRegistry, columns: &ColumnSync, packet: OscPacket) {
    match packet {
        OscPacket::Message(msg) => {
            // Resolume first, because a launch is not a parameter write
            // and — more to the point — need not carry an argument at
            // all, so the numeric-argument guard below would drop it.
            if follow_column(registry, columns, &msg) || follow_deck(registry, columns, &msg) {
                return;
            }
            let Some(value) = msg.args.iter().find_map(as_f32) else {
                log::debug!("OSC message {} had no numeric argument", msg.addr);
                return;
            };
            // A driven parameter is the transition's, not the wire's:
            // `/cloud/morph` is swept by whichever scene change is in
            // flight, and an OSC write landing in the middle of that is
            // two things steering one value. Dropped with a note rather
            // than silently, so a script that still sends one can be
            // found.
            if let Some(id) = registry.id(&msg.addr)
                && registry.defs()[id.index()].driven
            {
                log::debug!("OSC message {} is driven by the app, not by hand", msg.addr);
                return;
            }
            registry.set_by_addr(&msg.addr, value);
        }
        OscPacket::Bundle(bundle) => {
            for inner in bundle.content {
                apply_packet(registry, columns, inner);
            }
        }
    }
}

/// Turn `/composition/decks/N/select` into a deck select, if this is one
/// and we are listening.
///
/// No origin arithmetic, unlike a column: a deck is a song and songs are
/// numbered the same on both sides. A deck past the end of the book is
/// clamped away by the parameter's own range and then ignored by the
/// engine, which is the right outcome — a composition with more decks
/// than this show has songs should not park the set on its last page.
///
/// The counter is not bumped. Reselecting the deck already showing is a
/// no-op here on purpose: a page turn is not a trigger, and re-running one
/// would put both fire controls back to rest under a performer who had
/// just pressed a pad.
fn follow_deck(registry: &ParamRegistry, columns: &ColumnSync, msg: &rosc::OscMessage) -> bool {
    let Some(rest) = msg.addr.strip_prefix(RESOLUME_DECKS) else {
        return false;
    };
    let Some(number) = rest.strip_suffix(RESOLUME_SELECT) else {
        return false;
    };
    let Ok(deck) = number.parse::<u32>() else {
        return false;
    };
    if !columns.enabled.load(Ordering::Relaxed) {
        return true;
    }
    if deck < 1 || msg.args.first().and_then(as_f32).is_some_and(|v| v < 1.0) {
        return true;
    }
    let Some(id) = registry.id(DECK_SELECT) else {
        log::debug!("{DECK_SELECT} is not a parameter here; deck {deck} ignored");
        return true;
    };
    let pages = registry.defs().get(id.index()).map_or(0.0, |d| d.max) as u32;
    if deck > pages {
        log::debug!("Resolume deck {deck} is past the end of this show");
        return true;
    }
    registry.set(id, deck as f32);
    true
}

/// Turn `/composition/columns/N/connect` into a column fire, if this is
/// one and we are listening for them.
///
/// Returns whether the message was Resolume's and has been dealt with, so
/// the caller does not also hand it to the registry — which would be
/// harmless today and a silent double-write the moment somebody registers
/// a parameter under a composition address.
///
/// The argument rule is Resolume's, not ours. Arena sends `connect 1` when
/// a column starts and `connect 0` when it is disconnected, and sends the
/// message with no argument at all in some configurations. So: no argument
/// means yes, and an argument means yes only when it is 1 or more. Reading
/// the *first* argument rather than the first numeric one matters here —
/// a message whose leading argument is a string is not something to go
/// hunting through for a number to act on.
fn follow_column(registry: &ParamRegistry, columns: &ColumnSync, msg: &rosc::OscMessage) -> bool {
    let Some(rest) = msg.addr.strip_prefix(RESOLUME_COLUMNS) else {
        return false;
    };
    let Some(number) = rest.strip_suffix(RESOLUME_CONNECT) else {
        return false;
    };
    let Ok(resolume) = number.parse::<u32>() else {
        return false;
    };
    // Claimed from here on: this is a column connect, whatever we decide
    // to do with it.
    if !columns.enabled.load(Ordering::Relaxed) {
        return true;
    }
    if resolume < 1 || msg.args.first().and_then(as_f32).is_some_and(|v| v < 1.0) {
        return true;
    }
    let Some(id) = registry.id(COLUMN_FIRE) else {
        // The app that owns this registry did not register the address.
        // Nothing to do, and nothing worth saying every packet.
        log::debug!("{COLUMN_FIRE} is not a parameter here; column {resolume} ignored");
        return true;
    };
    // The parameter's own ceiling is the number of columns there are —
    // taken from the registry rather than restated here, so this cannot
    // disagree with the grid about how many pads a row has.
    let slots = registry.defs().get(id.index()).map_or(0.0, |d| d.max) as u32;
    let Some(slot) = columns.column_for(resolume, slots) else {
        log::debug!("Resolume column {resolume} is outside this deck's columns");
        return true;
    };
    registry.set(id, slot as f32);
    // Released after the value, and read with Acquire on the other side,
    // so a render thread that sees the bump is guaranteed to see the
    // column it belongs to rather than the one before it.
    columns.fires.fetch_add(1, Ordering::Release);
    true
}

fn as_f32(arg: &OscType) -> Option<f32> {
    match arg {
        OscType::Float(f) => Some(*f),
        OscType::Double(d) => Some(*d as f32),
        OscType::Int(i) => Some(*i as f32),
        OscType::Long(l) => Some(*l as f32),
        OscType::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscBundle, OscMessage, OscTime};
    use std::time::Instant;
    use vizz_params::ParamDef;

    fn registry() -> Arc<ParamRegistry> {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/test/a", 0.0, 100.0, 0.0));
        b.add(ParamDef::new("/test/b", 0.0, 1.0, 0.5));
        // Sixteen columns, matching the grid the app actually has, so the
        // out-of-range tests below are testing the real boundary.
        b.add(ParamDef::new(COLUMN_FIRE, 0.0, 16.0, 0.0));
        // Twenty-four pages, matching the deck book's ceiling.
        b.add(ParamDef::new(DECK_SELECT, 0.0, 24.0, 0.0));
        // Driven by the app rather than the wire: the cloud morph.
        b.add(ParamDef::new("/cloud/morph", 0.0, 1.0, 0.0).driven());
        Arc::new(b.build())
    }

    /// A driven parameter is refused over OSC.
    ///
    /// `/cloud/morph` is swept by whichever scene transition is running.
    /// A script still sending it would be a second hand on the same
    /// value, and the visible result — a crossfade that stutters or
    /// lands short — looks like a bug in transitions rather than like
    /// two writers.
    #[test]
    fn the_wire_cannot_move_a_driven_parameter() {
        let reg = registry();
        let columns = ColumnSync::default();
        apply_packet(
            &reg,
            &columns,
            OscPacket::Message(OscMessage {
                addr: "/cloud/morph".into(),
                args: vec![OscType::Float(1.0)],
            }),
        );
        assert_eq!(
            reg.target(reg.id("/cloud/morph").unwrap()),
            0.0,
            "OSC moved a parameter the app drives"
        );
        // And an ordinary parameter in the same breath still lands, so
        // this is a rule about one flag and not a broken ingest.
        apply_packet(
            &reg,
            &columns,
            OscPacket::Message(OscMessage {
                addr: "/test/b".into(),
                args: vec![OscType::Float(1.0)],
            }),
        );
        assert_eq!(reg.target(reg.id("/test/b").unwrap()), 1.0);
    }

    fn select(deck: u32, args: Vec<OscType>) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: format!("/composition/decks/{deck}/select"),
            args,
        })
    }

    fn page(reg: &ParamRegistry) -> f32 {
        reg.target(reg.id(DECK_SELECT).unwrap())
    }

    /// A listener that is following columns from the first one.
    fn following() -> Arc<ColumnSync> {
        let columns = Arc::new(ColumnSync::default());
        columns.enabled.store(true, Ordering::Relaxed);
        columns.origin.store(1, Ordering::Relaxed);
        columns
    }

    fn connect(column: u32, args: Vec<OscType>) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: format!("/composition/columns/{column}/connect"),
            args,
        })
    }

    /// What the registry holds for a column, and how many have landed.
    fn state(reg: &ParamRegistry, columns: &ColumnSync) -> (f32, u32) {
        (
            reg.target(reg.id(COLUMN_FIRE).unwrap()),
            columns.fires.load(Ordering::Acquire),
        )
    }

    fn wait_for(reg: &ParamRegistry, addr: &str, expect: f32) -> bool {
        let id = reg.id(addr).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if (reg.target(id) - expect).abs() < 1e-6 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn end_to_end_udp_message_sets_param() {
        let reg = registry();
        let server = OscServer::spawn(Arc::clone(&reg), Arc::new(ColumnSync::default()), "127.0.0.1:0").unwrap();

        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let msg = OscPacket::Message(OscMessage {
            addr: "/test/a".into(),
            args: vec![OscType::Float(42.0)],
        });
        let bytes = rosc::encoder::encode(&msg).unwrap();
        client.send_to(&bytes, server.local_addr()).unwrap();

        assert!(wait_for(&reg, "/test/a", 42.0), "param never updated");
    }

    #[test]
    fn bundles_and_int_args_are_handled() {
        let reg = registry();
        let bundle = OscPacket::Bundle(OscBundle {
            timetag: OscTime { seconds: 0, fractional: 0 },
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/test/a".into(),
                    args: vec![OscType::Int(7)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/test/b".into(),
                    args: vec![OscType::Double(0.25)],
                }),
            ],
        });
        apply_packet(&reg, &ColumnSync::default(), bundle);
        assert_eq!(reg.target(reg.id("/test/a").unwrap()), 7.0);
        assert_eq!(reg.target(reg.id("/test/b").unwrap()), 0.25);
    }

    #[test]
    fn garbage_and_unknown_addresses_are_ignored() {
        let reg = registry();
        let server = OscServer::spawn(Arc::clone(&reg), Arc::new(ColumnSync::default()), "127.0.0.1:0").unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();

        // Garbage bytes, then an unknown address, then a valid message.
        client.send_to(b"not osc at all", server.local_addr()).unwrap();
        let unknown = rosc::encoder::encode(&OscPacket::Message(OscMessage {
            addr: "/mystery".into(),
            args: vec![OscType::Float(1.0)],
        }))
        .unwrap();
        client.send_to(&unknown, server.local_addr()).unwrap();
        let valid = rosc::encoder::encode(&OscPacket::Message(OscMessage {
            addr: "/test/a".into(),
            args: vec![OscType::Float(3.0)],
        }))
        .unwrap();
        client.send_to(&valid, server.local_addr()).unwrap();

        // The listener survived the garbage and still processes real traffic.
        assert!(wait_for(&reg, "/test/a", 3.0));
    }

    /// The whole point: launching a column in Resolume fires the matching
    /// column here.
    #[test]
    fn a_resolume_column_launch_fires_the_matching_column() {
        let (reg, columns) = (registry(), following());
        apply_packet(&reg, &columns, connect(3, vec![OscType::Int(1)]));
        assert_eq!(state(&reg, &columns), (3.0, 1));
    }

    /// Arena sends the connect with no argument at all in some
    /// configurations, and the numeric-argument guard that protects every
    /// other address would drop it — which is why the column handler runs
    /// first. This is the case that made mouse launches appear to do
    /// nothing while a MIDI launch of the same column worked.
    #[test]
    fn a_column_launch_with_no_argument_still_fires() {
        let (reg, columns) = (registry(), following());
        apply_packet(&reg, &columns, connect(2, vec![]));
        assert_eq!(state(&reg, &columns), (2.0, 1));
    }

    /// `connect 0` is Resolume saying a column was *dis*connected. Firing
    /// on it would make every column launch fire twice — once for the
    /// column arriving and once for the one it replaced leaving.
    #[test]
    fn disconnecting_a_column_fires_nothing() {
        let (reg, columns) = (registry(), following());
        apply_packet(&reg, &columns, connect(4, vec![OscType::Int(1)]));
        assert_eq!(state(&reg, &columns), (4.0, 1));
        apply_packet(&reg, &columns, connect(4, vec![OscType::Int(0)]));
        assert_eq!(
            state(&reg, &columns),
            (4.0, 1),
            "a disconnect was treated as a launch"
        );
    }

    /// Relaunching the same column has to land. Firing is edge triggered
    /// on the slot number, so the value alone cannot say a second launch
    /// happened — the counter is what carries it.
    #[test]
    fn relaunching_the_same_column_is_a_second_fire() {
        let (reg, columns) = (registry(), following());
        apply_packet(&reg, &columns, connect(5, vec![OscType::Int(1)]));
        apply_packet(&reg, &columns, connect(5, vec![OscType::Int(1)]));
        assert_eq!(
            state(&reg, &columns),
            (5.0, 2),
            "the second launch of the same column left no trace"
        );
    }

    /// A deck pointed at its own stretch of a long composition follows
    /// that stretch, and stays silent either side of it. Clamping instead
    /// would land every column past the end on the last pad, which reads
    /// as the feature being broken.
    #[test]
    fn a_deck_follows_its_own_stretch_of_columns_and_no_others() {
        let (reg, columns) = (registry(), following());
        columns.origin.store(17, Ordering::Relaxed);

        apply_packet(&reg, &columns, connect(17, vec![]));
        assert_eq!(state(&reg, &columns), (1.0, 1), "the deck's first column did not fire");
        apply_packet(&reg, &columns, connect(32, vec![]));
        assert_eq!(state(&reg, &columns), (16.0, 2), "the deck's last column did not fire");

        apply_packet(&reg, &columns, connect(16, vec![]));
        apply_packet(&reg, &columns, connect(33, vec![]));
        assert_eq!(
            state(&reg, &columns),
            (16.0, 2),
            "a column outside this deck's stretch fired anyway"
        );
    }

    /// Off means off. The listener binds every interface by default, so
    /// following columns hands the venue's wifi the scene transport —
    /// nobody should get that without asking.
    #[test]
    fn a_column_launch_does_nothing_while_following_is_off() {
        let (reg, columns) = (registry(), Arc::new(ColumnSync::default()));
        assert!(!columns.enabled.load(Ordering::Relaxed), "following defaulted to on");
        apply_packet(&reg, &columns, connect(3, vec![OscType::Int(1)]));
        assert_eq!(state(&reg, &columns), (0.0, 0));
    }

    /// A composition address is claimed by the column handler whether or
    /// not it fires, so it can never also fall through to the registry.
    /// Nothing registers a `/composition/*` parameter today; a silent
    /// double-write the day something does is exactly the kind of fault
    /// that gets blamed on Resolume.
    #[test]
    fn a_composition_address_never_reaches_the_registry() {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/composition/columns/1/connect", 0.0, 9.0, 0.0));
        b.add(ParamDef::new(COLUMN_FIRE, 0.0, 16.0, 0.0));
        let reg = Arc::new(b.build());
        let trap = reg.id("/composition/columns/1/connect").unwrap();

        apply_packet(&reg, &following(), connect(1, vec![OscType::Int(1)]));
        assert_eq!(reg.target(trap), 0.0, "the column message also wrote a parameter");

        // And with following off, it is still claimed rather than passed on.
        apply_packet(&reg, &Arc::new(ColumnSync::default()), connect(1, vec![OscType::Int(1)]));
        assert_eq!(reg.target(trap), 0.0);
    }

    /// Addresses that only look like Resolume's are left alone, so a
    /// parameter named near one keeps working.
    #[test]
    fn addresses_that_merely_resemble_a_column_are_left_alone() {
        let columns = following();
        let mut b = ParamRegistry::builder();
        for addr in [
            "/composition/columns/2/name",
            "/composition/columns/two/connect",
            "/composition/columns//connect",
        ] {
            b.add(ParamDef::new(addr, 0.0, 9.0, 0.0));
        }
        b.add(ParamDef::new(COLUMN_FIRE, 0.0, 16.0, 0.0));
        let reg = Arc::new(b.build());

        for addr in [
            "/composition/columns/2/name",
            "/composition/columns/two/connect",
            "/composition/columns//connect",
        ] {
            apply_packet(
                &reg,
                &columns,
                OscPacket::Message(OscMessage { addr: addr.into(), args: vec![OscType::Float(4.0)] }),
            );
            assert_eq!(
                reg.target(reg.id(addr).unwrap()),
                4.0,
                "{addr} was swallowed by the column handler"
            );
        }
        assert_eq!(columns.fires.load(Ordering::Acquire), 0, "one of them fired a column");
    }

    /// Column launches arrive inside bundles too — Resolume batches its
    /// output — and the handler has to be reachable through the recursion
    /// rather than only at the top level.
    #[test]
    fn a_column_launch_inside_a_bundle_fires() {
        let (reg, columns) = (registry(), following());
        apply_packet(
            &reg,
            &columns,
            OscPacket::Bundle(OscBundle {
                timetag: OscTime { seconds: 0, fractional: 0 },
                content: vec![
                    OscPacket::Message(OscMessage {
                        addr: "/test/a".into(),
                        args: vec![OscType::Float(9.0)],
                    }),
                    connect(6, vec![OscType::Int(1)]),
                ],
            }),
        );
        assert_eq!(state(&reg, &columns), (6.0, 1));
        assert_eq!(reg.target(reg.id("/test/a").unwrap()), 9.0);
    }

    /// Changing deck in Arena changes the song here. A deck is the song
    /// and a column is the section; following both is what makes one
    /// launch move the video, the lights and the field together.
    #[test]
    fn a_resolume_deck_change_selects_the_matching_song() {
        let (reg, columns) = (registry(), following());
        apply_packet(&reg, &columns, select(7, vec![OscType::Int(1)]));
        assert_eq!(page(&reg), 7.0);
        apply_packet(&reg, &columns, select(2, vec![]));
        assert_eq!(page(&reg), 2.0, "a select with no argument did not land");
    }

    /// A page turn is not a trigger, so it must not disturb the pads.
    ///
    /// Bumping the fire counter here would re-arm both grids and put the
    /// fire controls back to rest under a performer who had just pressed
    /// a pad — a deck message arriving mid-song would silently reset the
    /// section they were playing.
    #[test]
    fn selecting_a_deck_does_not_fire_anything() {
        let (reg, columns) = (registry(), following());
        apply_packet(&reg, &columns, connect(4, vec![]));
        let (column, fires) = state(&reg, &columns);
        apply_packet(&reg, &columns, select(3, vec![]));
        assert_eq!(
            state(&reg, &columns),
            (column, fires),
            "a deck change touched the column transport"
        );
    }

    /// A composition with more decks than this show has songs must not
    /// park the set on its last page.
    #[test]
    fn a_deck_past_the_end_of_the_show_is_ignored() {
        let (reg, columns) = (registry(), following());
        apply_packet(&reg, &columns, select(3, vec![]));
        apply_packet(&reg, &columns, select(99, vec![]));
        assert_eq!(page(&reg), 3.0, "a deck past the end was clamped onto the last page");
        apply_packet(&reg, &columns, select(0, vec![]));
        assert_eq!(page(&reg), 3.0, "deck 0 does not exist in Resolume");
    }

    /// One switch covers both halves of the sync.
    #[test]
    fn a_deck_change_does_nothing_while_following_is_off() {
        let (reg, columns) = (registry(), Arc::new(ColumnSync::default()));
        apply_packet(&reg, &columns, select(5, vec![OscType::Int(1)]));
        assert_eq!(page(&reg), 0.0);
    }

    /// A deck address is claimed whether or not it fires, so it can never
    /// also fall through to the registry.
    #[test]
    fn a_deck_address_never_reaches_the_registry() {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/composition/decks/1/select", 0.0, 9.0, 0.0));
        b.add(ParamDef::new(DECK_SELECT, 0.0, 24.0, 0.0));
        let reg = Arc::new(b.build());
        let trap = reg.id("/composition/decks/1/select").unwrap();
        apply_packet(&reg, &following(), select(1, vec![OscType::Int(1)]));
        assert_eq!(reg.target(trap), 0.0, "the deck message also wrote a parameter");
    }

    /// End to end over a real socket, because everything above calls
    /// `apply_packet` directly and would pass even if the listener never
    /// handed it a Resolume message.
    #[test]
    fn a_column_launch_arrives_over_udp() {
        let (reg, columns) = (registry(), following());
        let server =
            OscServer::spawn(Arc::clone(&reg), Arc::clone(&columns), "127.0.0.1:0").unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let bytes = rosc::encoder::encode(&connect(8, vec![OscType::Int(1)])).unwrap();
        client.send_to(&bytes, server.local_addr()).unwrap();

        assert!(wait_for(&reg, COLUMN_FIRE, 8.0), "the column never arrived");
    }
}
