//! Crash reports and feedback, to letissier.ie and nowhere else.
//!
//! The contract lives in the release review's `intake-api.md`; the rules
//! this crate keeps, in the order a crash meets them:
//!
//! 1. **Recover first, report second.** Nothing here runs before the app
//!    has restored itself; the prompt comes after the show is back.
//! 2. **Opt-in.** Crash reports wait in `pending/` until the person says
//!    Send, or has ticked "Send crash reports automatically" (off by
//!    default). Feedback goes only when its Send is pressed.
//! 3. **Scrubbed on the device.** See [`scrub`]; every report is scrubbed
//!    as it is built, so nothing unscrubbed is ever written to disk.
//! 4. **Offline first.** Everything agreed to goes through `outbox/` and
//!    leaves on a background thread, now or on a later launch.
//! 5. **Never in the way.** No network on the render thread or the
//!    startup path, an eight-second timeout, and no error on screen when
//!    sending fails.
//!
//! What a crash report can carry is fixed by [`payload::CrashReport`]:
//! the product, version, OS, architecture, a random install id, the kind,
//! the first line of the error, the scrubbed backtrace, a signature, the
//! time, and a note only if the person typed one. There is no field for a
//! licence key, an email, a show, a preset, a source name, a picture or a
//! sound, so none of those can be sent even by mistake.

pub mod crash;
pub mod net;
pub mod payload;
pub mod queue;
pub mod scrub;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use crash::Previous;
pub use payload::{CrashKind, FeedbackForm, FeedbackKind, FormError};
pub use queue::{Envelope, Queue};

/// This app, as the intake names it.
pub const PRODUCT: &str = "vizz";
/// Shown in the prompt and the User-Agent.
pub const DISPLAY_NAME: &str = "Vizz";
/// Where reports go. `LETISSIER_API` overrides it, for tests and staging.
pub const BASE_URL: &str = "https://letissier.ie";

/// The service to send to: the override if one is set, else the live site.
pub fn base_url() -> String {
    std::env::var("LETISSIER_API")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| v.starts_with("http://") || v.starts_with("https://"))
        .unwrap_or_else(|| BASE_URL.to_string())
}

/// Make a new random install id: a v4 UUID from the OS's random source.
///
/// Random rather than derived from anything — never the licence, the
/// hardware id, a MAC, the hostname or the user name — so it says nothing
/// about who or where, and only lets the service count installs.
pub fn new_install_id() -> Option<String> {
    use ring::rand::SecureRandom as _;
    let mut b = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut b).ok()?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Some(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

/// Everything a [`Reporter`] depends on, so a test can supply all of it.
pub struct Config {
    /// `<config root>/reports`.
    pub dir: PathBuf,
    pub version: String,
    pub install: Option<String>,
    /// "Send crash reports automatically".
    pub auto: bool,
    pub base: String,
    pub transport: Arc<dyn net::Transport>,
    pub who: scrub::Identity,
}

impl Config {
    /// The real thing: this machine, this build, the live service.
    pub fn for_this_machine(dir: PathBuf, version: &str, install: Option<String>, auto: bool) -> Self {
        let user_agent = format!("{DISPLAY_NAME}/{version} ({})", payload::os());
        Config {
            dir,
            version: version.to_string(),
            install,
            auto,
            base: base_url(),
            transport: Arc::new(net::Http::new(user_agent)),
            who: scrub::Identity::current(),
        }
    }
}

struct Inner {
    dir: PathBuf,
    origin: payload::Origin,
    who: scrub::Identity,
    auto: AtomicBool,
    base: String,
    transport: Arc<dyn net::Transport>,
    /// A flush is running; a second request while it does is folded in.
    flushing: AtomicBool,
    /// Signatures already written this run, so a panic caught on every
    /// frame is one report rather than sixty a second.
    seen: Mutex<Vec<String>>,
}

/// The app's handle. Cheap to clone; every clone is the same reporter.
#[derive(Clone)]
pub struct Reporter {
    inner: Arc<Inner>,
}

impl Reporter {
    pub fn new(cfg: Config) -> Self {
        let origin = payload::Origin::current(PRODUCT, &cfg.version, cfg.install);
        Reporter {
            inner: Arc::new(Inner {
                dir: cfg.dir,
                origin,
                who: cfg.who,
                auto: AtomicBool::new(cfg.auto),
                base: cfg.base,
                transport: cfg.transport,
                flushing: AtomicBool::new(false),
                seen: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Crash reports waiting for a yes.
    pub fn pending(&self) -> Queue {
        Queue::new(self.inner.dir.join("pending"))
    }

    /// Everything agreed to and not yet taken by the service.
    pub fn outbox(&self) -> Queue {
        Queue::new(self.inner.dir.join("outbox"))
    }

    pub fn auto(&self) -> bool {
        self.inner.auto.load(Ordering::Relaxed)
    }

    /// Change "Send crash reports automatically" for this run. The caller
    /// persists it. Turning it on sends whatever was already waiting — the
    /// person has just said yes to exactly that.
    pub fn set_auto(&self, on: bool) {
        self.inner.auto.store(on, Ordering::Relaxed);
        if on {
            self.send_pending("");
        }
    }

    /// Note the start of a run; say how the last one ended. After an
    /// unclean exit that did not report itself, a report of the exit is
    /// added — to the outbox if the person has opted in, to pending if
    /// they have not.
    pub fn begin_session(&self, alive: &dyn Fn(u32) -> bool) -> Previous {
        let now = crash::now();
        let previous = crash::begin(&self.inner.dir, &self.inner.origin.version, now, alive);
        if previous.unclean && previous.unreported {
            let mut origin = self.inner.origin.clone();
            if let Some(v) = &previous.version {
                origin.version = scrub::cut(v, payload::limit::VERSION);
            }
            let report = payload::crash(
                &origin,
                &self.inner.who,
                CrashKind::UncleanExit,
                &payload::Raw {
                    summary: format!(
                        "{DISPLAY_NAME} did not exit cleanly and no panic was caught — a native crash, a hang the \
                         system ended, a force quit or a power cut"
                    ),
                    detail: None,
                    occurred_at: None,
                },
            );
            let target = if self.auto() { self.outbox() } else { self.pending() };
            if let Err(e) = target.push(&Envelope::Crash(report)) {
                log::warn!("could not queue the unclean-exit report: {e}");
            }
        }
        previous
    }

    /// A clean exit.
    pub fn end_session(&self) {
        crash::end(&self.inner.dir);
    }

    /// Catch every panic, on every thread: write a scrubbed report and let
    /// the default handler carry on. Nothing is sent from here — the hook
    /// runs on whatever thread panicked, possibly the render thread.
    pub fn install_panic_hook(&self) {
        let me = self.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            previous(info);
            let message = crash::panic_message(info);
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_default();
            let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
            let backtrace = std::backtrace::Backtrace::force_capture();
            me.record(
                CrashKind::Panic,
                payload::Raw {
                    summary: message,
                    detail: Some(format!("thread '{thread}' panicked at {location}\n{backtrace}")),
                    occurred_at: Some(crash::iso8601(crash::now())),
                },
            );
        }));
    }

    /// Queue a report of something caught — a panic, a lost GPU. Written
    /// once per signature per run. Returns whether it was written.
    pub fn record(&self, kind: CrashKind, raw: payload::Raw) -> bool {
        let report = payload::crash(&self.inner.origin, &self.inner.who, kind, &raw);
        let sig = report.signature.clone().unwrap_or_default();
        // try_lock: the hook may run while another panic holds it, and a
        // hook that blocks is a hook that deadlocks the process.
        match self.inner.seen.try_lock() {
            Ok(seen) if seen.contains(&sig) => return false,
            Ok(mut seen) => seen.push(sig),
            Err(_) => return false,
        }
        let target = if self.auto() { self.outbox() } else { self.pending() };
        let written = target.push(&Envelope::Crash(report)).is_ok();
        if written {
            crash::mark_reported(&self.inner.dir);
        }
        written
    }

    /// The person said Send. The note goes on the newest report — the one
    /// that describes what they were doing — and all of them move to the
    /// outbox and go.
    pub fn send_pending(&self, note: &str) -> usize {
        let pending = self.pending();
        let newest = pending.entries().last().map(|e| e.path.clone());
        let mut n = 0;
        for entry in pending.entries() {
            let envelope = match entry.envelope {
                Envelope::Crash(c) if Some(&entry.path) == newest.as_ref() => {
                    Envelope::Crash(payload::with_note(c, note, &self.inner.who))
                }
                other => other,
            };
            if self.outbox().push(&envelope).is_ok() {
                let _ = std::fs::remove_file(&entry.path);
                n += 1;
            }
        }
        if n > 0 {
            self.flush_in_background(Duration::ZERO);
        }
        n
    }

    /// The person said Don't send.
    pub fn discard_pending(&self) {
        self.pending().clear();
    }

    /// Queue feedback from the form and send it. Only called from the
    /// form's Send button.
    pub fn send_feedback(
        &self,
        kind: FeedbackKind,
        form: &FeedbackForm,
        licence: Option<&str>,
    ) -> Result<(), String> {
        let body = payload::feedback(&self.inner.origin, &self.inner.who, kind, form, licence)
            .map_err(|e| e.to_string())?;
        self.outbox()
            .push(&Envelope::Feedback(body))
            .map_err(|e| format!("could not save it to send: {e}"))?;
        self.flush_in_background(Duration::ZERO);
        Ok(())
    }

    /// Send the outbox on a thread of its own, after `delay`. Returns at
    /// once; a flush already running picks up anything new.
    pub fn flush_in_background(&self, delay: Duration) {
        if self.inner.flushing.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        let spawned = std::thread::Builder::new().name("vizz-report".into()).spawn(move || {
            std::thread::sleep(delay);
            let done = me.flush_now();
            if done.sent + done.dropped > 0 {
                log::info!("reports: {} sent, {} refused, {} waiting", done.sent, done.dropped, done.kept);
            }
            me.inner.flushing.store(false, Ordering::Release);
        });
        if spawned.is_err() {
            self.inner.flushing.store(false, Ordering::Release);
        }
    }

    /// Send the outbox now, on this thread. For the background flush and
    /// for tests.
    pub fn flush_now(&self) -> net::Flushed {
        net::flush(&self.outbox(), self.inner.transport.as_ref(), &self.inner.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::stub::Stub;
    use crate::queue::tests::scratch;

    fn reporter(tag: &str, auto: bool, stub: Arc<Stub>) -> (Reporter, PathBuf) {
        let dir = scratch(tag);
        let r = Reporter::new(Config {
            dir: dir.clone(),
            version: "1.0.0".into(),
            install: new_install_id(),
            auto,
            base: "https://letissier.test".into(),
            transport: stub,
            who: scrub::Identity { home: Some("/Users/colm".into()), user: Some("colm".into()) },
        });
        (r, dir)
    }

    fn crash_after(r: &Reporter, dir: &std::path::Path) -> Previous {
        // A run that died: its marker is still there, its pid long gone.
        crash::write_marker(
            dir,
            &crash::Marker { pid: 9_999_993, version: "1.0.0".into(), started: 1, reported: false },
        );
        r.begin_session(&|_| false)
    }

    /// The rule that matters most: with the setting off and no click,
    /// nothing leaves the machine — not after a crash, not after a flush,
    /// not after a caught panic.
    #[test]
    fn setting_off_and_no_click_sends_nothing() {
        let stub = Arc::new(Stub::answering(vec![Ok(202); 10]));
        let (r, dir) = reporter("off-no-click", false, stub.clone());
        let prev = crash_after(&r, &dir);
        assert!(prev.unclean);
        r.record(
            CrashKind::Panic,
            payload::Raw { summary: "boom".into(), detail: None, occurred_at: None },
        );
        assert_eq!(r.pending().len(), 2, "both reports should be waiting for a yes");
        let done = r.flush_now();
        assert_eq!(done.sent, 0);
        assert!(stub.sent().is_empty(), "something was sent without consent: {:?}", stub.sent());

        // Don't send: they are gone, and still nothing went.
        r.discard_pending();
        assert!(r.pending().is_empty());
        r.flush_now();
        assert!(stub.sent().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pressing_send_sends_with_the_note_on_the_newest() {
        let stub = Arc::new(Stub::answering(vec![Ok(202); 10]));
        let (r, dir) = reporter("send-click", false, stub.clone());
        crash_after(&r, &dir);
        r.record(
            CrashKind::Panic,
            payload::Raw { summary: "boom".into(), detail: None, occurred_at: None },
        );
        // send_pending flushes in the background; move and flush here
        // instead so the test does not race the thread.
        r.inner.flushing.store(true, Ordering::SeqCst);
        assert_eq!(r.send_pending("fading /Users/colm/Shows/x to black"), 2);
        r.inner.flushing.store(false, Ordering::SeqCst);
        assert_eq!(r.flush_now().sent, 2);
        let sent = stub.sent();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|(url, _)| url == "https://letissier.test/api/reports/crash"));
        assert_eq!(sent[0].1["kind"], "unclean-exit");
        assert!(sent[0].1.get("note").is_none(), "the note belongs on the newest report only");
        assert_eq!(sent[1].1["note"], "fading ~/Shows/x to black", "the note must be scrubbed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_the_setting_on_reports_go_straight_to_the_outbox() {
        let stub = Arc::new(Stub::answering(vec![Ok(202); 10]));
        let (r, dir) = reporter("auto-on", true, stub.clone());
        crash_after(&r, &dir);
        assert!(r.pending().is_empty());
        assert_eq!(r.outbox().len(), 1);
        assert_eq!(r.flush_now().sent, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_panic_caught_every_frame_is_one_report() {
        let stub = Arc::new(Stub::default());
        let (r, dir) = reporter("dedupe", false, stub);
        for _ in 0..60 {
            r.record(
                CrashKind::Panic,
                payload::Raw { summary: "same thing".into(), detail: None, occurred_at: None },
            );
        }
        assert_eq!(r.pending().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn feedback_is_queued_then_sent_and_waits_when_offline() {
        let stub = Arc::new(Stub::answering(vec![Err("offline".into())]));
        let (r, dir) = reporter("feedback", false, stub.clone());
        r.inner.flushing.store(true, Ordering::SeqCst);
        let form = FeedbackForm { message: "more palettes please".into(), ..Default::default() };
        r.send_feedback(FeedbackKind::Idea, &form, Some("LT-VIZZ-AAAA-BBBB-CCCC")).unwrap();
        r.inner.flushing.store(false, Ordering::SeqCst);
        assert_eq!(r.flush_now().kept, 1, "offline: it must wait for the next launch");
        assert_eq!(r.outbox().len(), 1);
        let body = &stub.sent()[0].1;
        assert_eq!(body["type"], "idea");
        assert!(body.get("licence").is_none(), "the licence box was not ticked");
        assert_eq!(body["install"].as_str().map(str::len), Some(36));

        // Empty messages never reach the queue.
        assert!(r.send_feedback(FeedbackKind::Bug, &FeedbackForm::default(), None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_ids_are_random_v4_uuids() {
        let a = new_install_id().unwrap();
        let b = new_install_id().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert!(payload::valid_install(&a));
    }

    #[test]
    fn the_base_url_can_be_pointed_elsewhere_for_tests() {
        // Read-only check of the default; the override is an env var and
        // setting it here would race other tests.
        if std::env::var_os("LETISSIER_API").is_none() {
            assert_eq!(base_url(), "https://letissier.ie");
        }
    }
}
