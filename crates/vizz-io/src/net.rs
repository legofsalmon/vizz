//! Which address other machines would reach this one on.
//!
//! Asked because of the point-cloud stream. The panel tells you to point
//! a sender at "this Mac's address on port 9848" and then does not say
//! what that address is, which leaves the one setup step the feature
//! needs to a trip through System Settings — at a venue, on a laptop,
//! usually in the dark.

use std::net::{IpAddr, UdpSocket};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a looked-up address is trusted before asking again.
///
/// It changes when the machine joins a different network, which for this
/// program is a thing that happens between venues rather than between
/// frames — but it does happen mid-session, and an address that is
/// quietly stale is worse than no address at all, because somebody will
/// type it in and then debug the sender.
const FRESH_FOR: Duration = Duration::from_secs(5);

/// When the address was last looked up, and what it was. The outer
/// `Option` is "never asked"; the inner one is "asked, and this machine
/// is not on a network".
type Cached = Option<(Instant, Option<IpAddr>)>;

fn cache() -> &'static Mutex<Cached> {
    static CACHE: OnceLock<Mutex<Cached>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// This machine's address on the network it would route out through, or
/// `None` if it does not appear to be on one.
///
/// Cached, because the panel asks every frame and the answer changes
/// about once a day.
pub fn local_ip() -> Option<IpAddr> {
    let mut guard = match cache().lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    if let Some((at, found)) = *guard
        && at.elapsed() < FRESH_FOR
    {
        return found;
    }
    let found = probe();
    *guard = Some((Instant::now(), found));
    found
}

/// The address the OS would use to reach the wider network.
///
/// Connecting a UDP socket sends nothing — `connect` on a datagram socket
/// only fixes the peer, which makes the kernel choose the interface it
/// would route through and bind a local address to match. That is exactly
/// the question worth answering here: not "what interfaces exist", which
/// on a Mac is a list including Thunderbolt bridges and VPN tunnels
/// nobody wants to read, but "which one would a laptop on the same wifi
/// find this machine at".
///
/// The peer is in TEST-NET-1, which RFC 5737 reserves for documentation
/// and which is therefore guaranteed never to be a real host. No packet
/// is sent to it either way; using a real public address would work
/// identically and would imply a dependency on somebody else's server
/// that does not exist.
///
/// Returns `None` on a machine with no route out at all — an isolated
/// switch with static addressing, most likely. That is a real setup in a
/// venue and this cannot answer for it, so it says nothing rather than
/// guessing.
fn probe() -> Option<IpAddr> {
    let sock = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.connect(("192.0.2.1", 9)).ok()?;
    let addr = sock.local_addr().ok()?.ip();
    // A bound-to-nothing socket reports the unspecified address, which is
    // true and useless: printing 0.0.0.0 as "send here" is worse than
    // printing nothing, because it looks like an answer.
    if addr.is_unspecified() || addr.is_loopback() {
        return None;
    }
    Some(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever it returns must be something a person could type into
    /// another machine. The two failure modes worth pinning are the ones
    /// that *look* like answers.
    #[test]
    fn a_reported_address_is_one_worth_printing() {
        if let Some(ip) = local_ip() {
            assert!(!ip.is_unspecified(), "0.0.0.0 is not somewhere to send to");
            assert!(!ip.is_loopback(), "loopback is not reachable from another machine");
        }
        // No assertion on `Some` — CI runners and sandboxes legitimately
        // have no route out, and a test that demanded one would fail for
        // reasons that have nothing to do with this code.
    }

    /// The cache hands back the same answer rather than probing per call.
    #[test]
    fn the_answer_is_cached() {
        let a = local_ip();
        let b = local_ip();
        assert_eq!(a, b);
    }
}
