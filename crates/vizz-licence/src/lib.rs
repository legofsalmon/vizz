//! Licence keys from letissier.ie: shop sales, keys and trials issued from
//! the admin panel, and trials started in the app — all the same format,
//! all through the same four endpoints.
//!
//! Split the way the rest of vizz splits I/O from decisions:
//!
//! - [`verify`] — the vendor's SDK: a token, a machine, a build date and a
//!   clock in; a status out. Pure.
//! - [`policy`] — what a status costs ([`policy::POLICY`] is the one
//!   switch) and when that cost may change. Pure.
//! - [`status`], [`key`] — words for the panel. Pure.
//! - [`machine`], [`store`], [`net`] — the platform id, `licence.json`,
//!   and the HTTP calls.
//! - [`Licence`] — the handle the app holds: the cached verdict, and the
//!   actions the panel asks for, each run on its own thread.
//!
//! The rules it keeps, from the integration brief:
//!
//! 1. Nothing licence-related stops a running show or blocks startup on
//!    the network. The startup decision reads a file and checks a
//!    signature; a failed network call keeps the cached answer.
//! 2. `update_required` and `check_in_required` never restrict anything.
//! 3. The key and token live in the machine's config directory, never in
//!    a show.
//! 4. A build with no usable public key restricts nothing.

pub mod key;
pub mod machine;
pub mod net;
pub mod policy;
pub mod status;
pub mod store;
pub mod verify;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use policy::{POLICY, Policy, Restriction, Session};
pub use status::{Headline, Tone};
pub use verify::{Claims, Status, Verdict};

/// This app, as the licence service names it.
pub const PRODUCT: &str = "vizz";
pub const BASE_URL: &str = "https://letissier.ie";
/// Where an owner releases a seat, or gets a token for an offline machine.
pub const ACCOUNT_URL: &str = "https://letissier.ie/account";
pub const SHOP_URL: &str = "https://letissier.ie/shop";
/// What the "start a trial" button promises. The service decides the real
/// length; this is only the label, and matches its published `trialDays`.
pub const TRIAL_DAYS: u32 = 30;

/// When this binary was built, in unix seconds — see `build.rs`.
pub const BUILD_DATE: i64 = match i64::from_str_radix(env!("VIZZ_LICENCE_BUILD_DATE"), 10) {
    Ok(v) => v,
    Err(_) => 0,
};

/// The vendor's licence signing key, from <https://letissier.ie/integrate>
/// ("Signing public key (embed this)").
///
/// Public: it can only verify, never mint, so it is safe in a binary and
/// in this repository. Compiled in as the default rather than read from
/// the environment, because a release built without the variable set
/// would otherwise ship a check that verifies nothing and looks exactly
/// like a working one. `VIZZ_LICENCE_PUBLIC_KEY` at build time overrides
/// it, for testing against another deployment.
const DEFAULT_PUBLIC_KEY: &str = "1fca6c21f2eb7963fd646272a731a41a191d3a4cda839e295c5cda67978fcc85";

pub const PUBLIC_KEY: &str = match option_env!("VIZZ_LICENCE_PUBLIC_KEY") {
    Some(k) => k,
    None => DEFAULT_PUBLIC_KEY,
};

/// Whether `key` could verify anything at all. False means rule 4: the
/// build restricts nothing.
pub fn key_usable(key: &str) -> bool {
    key.len() == 64 && verify::from_hex(key).is_some()
}

/// How often a machine checks in once it is up. The service's guidance
/// is "on launch and roughly daily"; the lease is set per licence and
/// nothing here assumes its length.
const CHECK_IN_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a decided verdict is trusted before the panel's next look
/// re-decides it against the clock, so "12 days left" becomes "11" and a
/// lapsed lease reads as one without anybody pressing anything.
const REDECIDE_AFTER: i64 = 60;

/// Everything [`Licence`] depends on, so a test can supply all of it.
pub struct Config {
    /// Where `licence.json` lives: the machine's config directory.
    pub dir: PathBuf,
    /// The RAW platform id. `None` when the platform will not say.
    pub fingerprint: Option<String>,
    pub public_key: String,
    pub build_date: i64,
    pub product: &'static str,
    pub base: String,
    pub transport: Arc<dyn net::Transport>,
    pub clock: fn() -> i64,
}

impl Config {
    /// The real thing: this machine, the shipped key, the live service.
    /// Reads the platform id, which on macOS and Windows spawns a small
    /// process — once, at startup.
    pub fn for_this_machine(dir: PathBuf) -> Self {
        Config {
            dir,
            fingerprint: machine::fingerprint(),
            public_key: PUBLIC_KEY.to_string(),
            build_date: BUILD_DATE,
            product: PRODUCT,
            base: BASE_URL.to_string(),
            transport: Arc::new(net::Http::default()),
            clock: unix_now,
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A line the panel shows after an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

/// Everything the licence section draws, taken in one go.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub status: Status,
    pub headline: Headline,
    pub details: Vec<String>,
    /// A key is stored, so check-in and release have something to act on.
    pub has_key: bool,
    /// The stored key, masked for display.
    pub key: Option<String>,
    /// An action is running: what it is, for the button row.
    pub busy: Option<&'static str>,
    pub message: Option<Message>,
    /// The raw platform id: what the account page asks for to issue an
    /// offline token. `None` when the platform would not give one.
    pub request_code: Option<String>,
    /// Short machine id, the first characters of the hash.
    pub machine: String,
    pub key_configured: bool,
}

struct State {
    verdict: Verdict,
    stored: store::Stored,
    decided_at: i64,
    busy: Option<&'static str>,
    message: Option<Message>,
}

struct Inner {
    cfg: Config,
    /// Our hash of the fingerprint: what a token's claim must equal.
    machine: String,
    state: Mutex<State>,
    /// Bumped whenever the verdict may have changed, so the render loop can
    /// notice with one atomic load a frame instead of a lock.
    revision: AtomicU64,
}

/// The app's handle on its licence. Cheap to clone; every clone is the
/// same licence.
#[derive(Clone)]
pub struct Licence {
    inner: Arc<Inner>,
}

impl Licence {
    /// Read the stored licence and decide it, offline. No network, and
    /// nothing here can fail: an unreadable file is simply unlicensed.
    pub fn open(cfg: Config) -> Self {
        let machine = verify::machine_hash(cfg.fingerprint.as_deref().unwrap_or(""));
        let stored = store::load(&cfg.dir);
        let now = (cfg.clock)();
        let verdict = decide(&cfg, &stored, now);
        let state = State { verdict, stored, decided_at: now, busy: None, message: None };
        Licence {
            inner: Arc::new(Inner { cfg, machine, state: Mutex::new(state), revision: AtomicU64::new(0) }),
        }
    }

    pub fn key_configured(&self) -> bool {
        key_usable(&self.inner.cfg.public_key)
    }

    /// The current verdict. Takes the lock, which no thread holds across
    /// anything slower than a signature check.
    pub fn verdict(&self) -> Verdict {
        self.lock().verdict.clone()
    }

    /// What the current verdict costs under `policy`.
    pub fn restriction(&self, policy: Policy) -> Restriction {
        policy::restriction(self.verdict().status, policy, self.key_configured())
    }

    /// [`Licence::restriction`] without waiting: `None` if a licence thread
    /// holds the lock this instant. For the render loop, which retries
    /// next frame rather than wait on anything.
    pub fn try_restriction(&self, policy: Policy) -> Option<Restriction> {
        let status = self.inner.state.try_lock().ok()?.verdict.status;
        Some(policy::restriction(status, policy, self.key_configured()))
    }

    /// Changes whenever the verdict may have. One atomic load.
    pub fn revision(&self) -> u64 {
        self.inner.revision.load(Ordering::Acquire)
    }

    /// Everything the panel shows, or `None` if a background action holds
    /// the lock this instant — the caller keeps last frame's copy rather
    /// than wait.
    pub fn snapshot(&self) -> Option<Snapshot> {
        let mut st = self.inner.state.try_lock().ok()?;
        let now = (self.inner.cfg.clock)();
        if now - st.decided_at >= REDECIDE_AFTER {
            let verdict = decide(&self.inner.cfg, &st.stored, now);
            st.decided_at = now;
            if verdict.status != st.verdict.status {
                self.bump();
            }
            st.verdict = verdict;
        }
        let configured = self.key_configured();
        Some(Snapshot {
            status: st.verdict.status,
            headline: status::headline(&st.verdict, now, configured),
            details: status::details(&st.verdict, configured, st.stored.token.is_some()),
            has_key: st.stored.key.is_some(),
            key: st.stored.key.as_deref().map(key::masked),
            busy: st.busy,
            message: st.message.clone(),
            request_code: self.inner.cfg.fingerprint.clone(),
            machine: status::short(&self.inner.machine).to_string(),
            key_configured: configured,
        })
    }

    // ------------------------------------------------------ the actions
    //
    // Each has a blocking form, which does the work and returns what to
    // say, and a spawning form, which is what the panel calls: it runs the
    // blocking form on its own thread and leaves the answer in the
    // snapshot. Only one runs at a time; a second press while one is in
    // flight is ignored, and the panel greys the buttons anyway.

    pub fn activate(&self, key: String, label: String) {
        self.spawn("activating…", move |l| l.activate_now(&key, &label));
    }

    pub fn start_trial(&self, email: String, name: String) {
        self.spawn("starting the trial…", move |l| l.trial_now(&email, &name));
    }

    pub fn use_token(&self, token: String) {
        self.spawn("checking the token…", move |l| l.use_token_now(&token));
    }

    pub fn release(&self) {
        self.spawn("releasing…", |l| l.release_now());
    }

    pub fn check_in(&self) {
        self.spawn("checking in…", |l| l.check_in_now().map_err(|e| e.to_string()));
    }

    /// Check in shortly after launch and then daily, on a thread of its
    /// own. Never on the startup path, and silent when offline: the
    /// cached token is still the answer, and a dialog about a lease in the
    /// middle of a set would be worse than the lapse.
    pub fn start_heartbeat(&self, first_after: Duration) {
        let me = self.clone();
        let spawned = std::thread::Builder::new().name("vizz-licence-checkin".into()).spawn(move || {
            std::thread::sleep(first_after);
            loop {
                if me.wants_check_in() {
                    match me.check_in_now() {
                        Ok(msg) => log::info!("licence: {msg}"),
                        Err(net::Failure::Offline(e)) => {
                            log::debug!("licence check-in skipped, offline: {e}")
                        }
                        Err(e) => log::warn!("licence check-in refused, keeping the cached licence: {e}"),
                    }
                }
                std::thread::sleep(CHECK_IN_EVERY);
            }
        });
        if let Err(e) = spawned {
            log::warn!("could not start the licence check-in: {e}");
        }
    }

    fn wants_check_in(&self) -> bool {
        let st = self.lock();
        // A finished trial has nothing to renew; buying means a new key.
        st.stored.key.is_some() && st.verdict.status != Status::Expired
    }

    pub fn activate_now(&self, key: &str, label: &str) -> Result<String, String> {
        let key = key.trim();
        if key.is_empty() {
            return Err("type or paste a licence key first".into());
        }
        let caller = self.caller()?;
        let issued = caller.activate(key, label).map_err(|e| e.to_string())?;
        // Another product's key. The service is gaining a check that
        // refuses these before a seat is taken; until then it activates
        // them, so give that seat straight back rather than leave a Light
        // or Datamosh licence holding one for a token Vizz will not keep.
        if let Some(claims) = verify::verify(&issued.token, &self.inner.cfg.public_key)
            && claims.product != self.inner.cfg.product
        {
            let _ = caller.deactivate(key);
        }
        self.persist(issued, Some(key))?;
        Ok(self.success_line())
    }

    pub fn trial_now(&self, email: &str, name: &str) -> Result<String, String> {
        let email = email.trim();
        if !email.contains('@') {
            return Err("an email address is needed to start the trial".into());
        }
        let issued = self.caller()?.trial(email, name.trim()).map_err(|e| e.to_string())?;
        self.persist(issued, None)?;
        Ok(self.success_line())
    }

    /// Renew the lease and persist the new token — which is what resets
    /// the offline window. Any failure keeps the cached token as it was.
    pub fn check_in_now(&self) -> Result<String, net::Failure> {
        let key = self
            .lock()
            .stored
            .key
            .clone()
            .ok_or_else(|| net::Failure::Protocol("no licence key is stored on this machine".into()))?;
        let caller = self.caller().map_err(net::Failure::Protocol)?;
        let issued = caller.heartbeat(&key)?;
        self.persist(issued, None).map_err(net::Failure::Protocol)?;
        Ok(format!("checked in — {}", self.verdict().status.as_str()))
    }

    /// Offline activation: a token the owner fetched from the account page
    /// with this machine's request code. Checked here before it is kept —
    /// it must be ours, for this product, and for this machine.
    pub fn use_token_now(&self, token: &str) -> Result<String, String> {
        let token: String = token.chars().filter(|c| !c.is_whitespace()).collect();
        let cfg = &self.inner.cfg;
        let Some(claims) = verify::verify(&token, &cfg.public_key) else {
            return Err(
                "that token does not verify — paste the whole of it, exactly as the account page shows it"
                    .into(),
            );
        };
        let key = claims.key.clone();
        self.persist(net::Issued { token, key: Some(key) }, None)?;
        Ok(self.success_line())
    }

    /// Give this machine's seat back and forget the licence here. The
    /// service call is best effort — a machine with no network must still
    /// be able to forget its licence — and the local forget always runs.
    pub fn release_now(&self) -> Result<String, String> {
        let key = self.lock().stored.key.clone();
        let mut note = String::new();
        if let (Some(key), Ok(caller)) = (key, self.caller())
            && let Err(e) = caller.deactivate(&key)
        {
            note = format!(
                " — the service was not told ({e}); release the seat from your account if it is still held"
            );
        }
        store::forget(&self.inner.cfg.dir)?;
        self.refresh(store::Stored::default());
        Ok(format!("released this machine{note}"))
    }

    // ---------------------------------------------------------- plumbing

    fn caller(&self) -> Result<net::Caller<'_>, String> {
        let cfg = &self.inner.cfg;
        let fingerprint = cfg.fingerprint.as_deref().ok_or_else(|| {
            "this machine will not report a hardware id, so it cannot be licensed online".to_string()
        })?;
        Ok(net::Caller { transport: &*cfg.transport, base: &cfg.base, product: cfg.product, fingerprint })
    }

    /// Keep a token the service just issued — after checking it is one this
    /// app can use — then re-decide from what was written, so the status
    /// shown is the one a restart would produce.
    fn persist(&self, issued: net::Issued, typed_key: Option<&str>) -> Result<(), String> {
        let cfg = &self.inner.cfg;
        if let Some(claims) = verify::verify(&issued.token, &cfg.public_key) {
            if claims.product != cfg.product {
                return Err(format!("that licence is for {}, not Vizz — nothing was stored", claims.product));
            }
            if claims.machine != self.inner.machine {
                return Err(format!(
                    "that token is for machine {}, and this one is {} — nothing was stored",
                    status::short(&claims.machine),
                    status::short(&self.inner.machine)
                ));
            }
        }
        let mut stored = self.lock().stored.clone();
        stored.token = Some(issued.token);
        if let Some(key) = issued.key.or_else(|| typed_key.map(str::to_string)) {
            stored.key = Some(key);
        }
        store::save(&cfg.dir, &stored)?;
        self.refresh(stored);
        Ok(())
    }

    fn refresh(&self, stored: store::Stored) {
        let now = (self.inner.cfg.clock)();
        let verdict = decide(&self.inner.cfg, &stored, now);
        {
            let mut st = self.lock();
            st.stored = stored;
            st.verdict = verdict;
            st.decided_at = now;
        }
        self.bump();
    }

    fn success_line(&self) -> String {
        let now = (self.inner.cfg.clock)();
        status::headline(&self.verdict(), now, self.key_configured()).text
    }

    fn spawn(
        &self,
        what: &'static str,
        work: impl FnOnce(&Licence) -> Result<String, String> + Send + 'static,
    ) {
        {
            let mut st = self.lock();
            if st.busy.is_some() {
                return;
            }
            st.busy = Some(what);
            st.message = None;
        }
        self.bump();
        let me = self.clone();
        let spawned = std::thread::Builder::new().name("vizz-licence".into()).spawn(move || {
            let result = work(&me);
            me.finish(result);
        });
        if let Err(e) = spawned {
            self.finish(Err(format!("could not start: {e}")));
        }
    }

    fn finish(&self, result: Result<String, String>) {
        {
            let mut st = self.lock();
            st.busy = None;
            st.message = Some(match result {
                Ok(text) => Message { text, error: false },
                Err(text) => Message { text, error: true },
            });
        }
        self.bump();
    }

    fn bump(&self) {
        self.inner.revision.fetch_add(1, Ordering::AcqRel);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic on a licence thread must not take the licence down with
        // it: the state is plain data and always consistent between writes.
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The whole offline decision: stored token, this machine, the build, the
/// clock. No token is `Invalid` — unlicensed.
fn decide(cfg: &Config, stored: &store::Stored, now: i64) -> Verdict {
    match &stored.token {
        // No fingerprint matches nothing, which is the honest answer for a
        // machine that will not say what it is.
        Some(token) => verify::check_for(
            token,
            cfg.fingerprint.as_deref().unwrap_or(""),
            cfg.build_date,
            now,
            &cfg.public_key,
            cfg.product,
        ),
        None => Verdict::invalid(),
    }
}

#[cfg(test)]
mod tests;
