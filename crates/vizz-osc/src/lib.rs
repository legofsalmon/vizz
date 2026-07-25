//! OSC input: a background UDP listener that writes straight into the
//! [`ParamRegistry`].
//!
//! The listener thread never touches the renderer — it only stores atomic
//! target values, so a flood of OSC traffic cannot stall a frame. Malformed
//! packets and unknown addresses are logged and dropped; the show goes on.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use rosc::{OscPacket, OscType};
use vizz_params::ParamRegistry;

/// Handle to the OSC listener thread. Dropping it shuts the thread down.
pub struct OscServer {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    local_addr: SocketAddr,
}

impl OscServer {
    /// Bind a UDP socket and start the listener thread.
    pub fn spawn(registry: Arc<ParamRegistry>, bind: impl ToSocketAddrs) -> io::Result<Self> {
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
                let mut buf = [0u8; rosc::decoder::MTU];
                while !thread_stop.load(Ordering::Relaxed) {
                    match socket.recv_from(&mut buf) {
                        Ok((n, _peer)) => match rosc::decoder::decode_udp(&buf[..n]) {
                            Ok((_, packet)) => apply_packet(&registry, packet),
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
fn apply_packet(registry: &ParamRegistry, packet: OscPacket) {
    match packet {
        OscPacket::Message(msg) => {
            let Some(value) = msg.args.iter().find_map(as_f32) else {
                log::debug!("OSC message {} had no numeric argument", msg.addr);
                return;
            };
            registry.set_by_addr(&msg.addr, value);
        }
        OscPacket::Bundle(bundle) => {
            for inner in bundle.content {
                apply_packet(registry, inner);
            }
        }
    }
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
        Arc::new(b.build())
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
        let server = OscServer::spawn(Arc::clone(&reg), "127.0.0.1:0").unwrap();

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
        apply_packet(&reg, bundle);
        assert_eq!(reg.target(reg.id("/test/a").unwrap()), 7.0);
        assert_eq!(reg.target(reg.id("/test/b").unwrap()), 0.25);
    }

    #[test]
    fn garbage_and_unknown_addresses_are_ignored() {
        let reg = registry();
        let server = OscServer::spawn(Arc::clone(&reg), "127.0.0.1:0").unwrap();
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
}
