//! [`Licence`] end to end, against the vendor's vectors and a stubbed HTTP
//! boundary whose replies are shaped exactly like the service's.

use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::net::stub::Stub;

const VECTORS: &str = include_str!("../testdata/vectors.json");

fn vectors() -> Value {
    serde_json::from_str(VECTORS).unwrap()
}

fn token(name: &str) -> String {
    vectors()["tokens"][name].as_str().unwrap().to_string()
}

/// The vectors' own moment, fingerprint and hash.
fn now() -> i64 {
    1_760_086_400
}
/// Past both vector tokens' `exp`.
fn later() -> i64 {
    1_762_678_400
}
const FP: &str = "ABC123-MACHINE-SERIAL";
const HASH: &str = "8b9dd6da2bcf47bdfe7ceb27c2a58680";
const KEY: &str = "LT-V1ZZ-K7M2-9PQR-4XTC";

fn config(tag: &str, stub: Arc<Stub>) -> Config {
    Config {
        dir: store::scratch(tag),
        fingerprint: Some(FP.into()),
        public_key: vectors()["publicKeyHex"].as_str().unwrap().into(),
        // The vectors' own iat: inside the valid token's update window.
        build_date: 1_760_000_000,
        product: "vizz",
        base: "https://letissier.ie".into(),
        transport: stub,
        clock: now,
    }
}

fn ok(body: Value) -> Result<(u16, String), String> {
    Ok((200, body.to_string()))
}

fn issued(token: &str, machine: &str) -> Result<(u16, String), String> {
    ok(json!({
        "ok": true, "token": token, "machine": machine, "product": "vizz",
        "edition": "standard", "seats": 2, "seatsUsed": 1,
        "checkInBy": "2025-11-08T00:00:00.000Z", "maintenanceUntil": "2026-10-09T00:00:00.000Z"
    }))
}

fn refused(status: u16, reason: &str, message: &str) -> Result<(u16, String), String> {
    Ok((status, json!({ "ok": false, "reason": reason, "message": message }).to_string()))
}

#[test]
fn no_licence_is_unlicensed_and_restricted_unless_open() {
    let l = Licence::open(config("none", Arc::default()));
    assert_eq!(l.verdict().status, Status::Invalid);
    assert_eq!(l.restriction(Policy::Watermark), Restriction::Watermark);
    assert_eq!(l.restriction(Policy::Lock), Restriction::Lock);
    assert_eq!(l.restriction(Policy::Open), Restriction::None);
    let snap = l.snapshot().unwrap();
    assert_eq!(snap.headline.text, "Unlicensed");
    assert_eq!(snap.request_code.as_deref(), Some(FP), "the request code is the raw id");
    assert_eq!(snap.machine, &HASH[..8]);
}

#[test]
fn activating_sends_the_raw_id_and_keeps_a_token_for_this_machine() {
    let stub = Arc::new(Stub::answering(vec![issued(&token("valid"), HASH)]));
    let cfg = config("activate", Arc::clone(&stub));
    let dir = cfg.dir.clone();
    let l = Licence::open(cfg);
    let before = l.revision();
    let said = l.activate_now(&format!("  {KEY} "), "FOH laptop").unwrap();
    assert_eq!(said, "Licensed to Test Buyer");
    assert!(l.revision() > before, "the render loop would not notice");

    let (url, body) = &stub.sent()[0];
    assert!(url.ends_with("/api/licence/activate"));
    assert_eq!(body["machine"], FP);
    assert_eq!(body["product"], "vizz");
    assert_eq!(body["key"], KEY, "sent trimmed, otherwise as typed");

    assert_eq!(l.verdict().status, Status::Active);
    assert_eq!(l.restriction(Policy::Lock), Restriction::None);
    // On disk, and decided the same way by a fresh start.
    let again = Licence::open(config_at(&dir));
    assert_eq!(again.verdict().status, Status::Active);
    assert_eq!(store::load(&dir).key.as_deref(), Some(KEY));
    let _ = std::fs::remove_dir_all(dir);
}

/// A second handle on an existing directory, no network.
fn config_at(dir: &std::path::Path) -> Config {
    Config { dir: dir.to_path_buf(), ..config("unused", Arc::default()) }
}

#[test]
fn an_echo_for_another_machine_stores_nothing() {
    let hash_of_hash = verify::machine_hash(HASH);
    let stub = Arc::new(Stub::answering(vec![issued(&token("valid"), &hash_of_hash)]));
    let cfg = config("echo", stub);
    let dir = cfg.dir.clone();
    let l = Licence::open(cfg);
    let err = l.activate_now(KEY, "").unwrap_err();
    assert!(err.contains("nothing was stored"), "{err}");
    assert!(!store::path(&dir).exists(), "a token for a machine that does not exist was kept");
    assert_eq!(l.verdict().status, Status::Invalid);
}

#[test]
fn a_refusal_is_shown_in_the_services_words_and_changes_nothing() {
    let msg = "All 2 seats are in use. Release one from your account first.";
    let stub = Arc::new(Stub::answering(vec![refused(409, "no_seats", msg)]));
    let cfg = config("refused", stub);
    let dir = cfg.dir.clone();
    let l = Licence::open(cfg);
    assert_eq!(l.activate_now(KEY, "").unwrap_err(), msg);
    assert!(!store::path(&dir).exists());
}

/// The vectors are all vizz, so a licence handle that is Light's sees a
/// vizz token as another product's: refused, not kept, and the seat the
/// service took for it given back.
#[test]
fn another_products_token_is_refused_and_its_seat_released() {
    let stub = Arc::new(Stub::answering(vec![issued(&token("valid"), HASH), ok(json!({ "ok": true }))]));
    let mut cfg = config("product", Arc::clone(&stub));
    cfg.product = "light";
    let dir = cfg.dir.clone();
    let l = Licence::open(cfg);
    let err = l.activate_now(KEY, "").unwrap_err();
    assert!(err.contains("for vizz"), "{err}");
    assert!(!store::path(&dir).exists());
    let sent = stub.sent();
    assert_eq!(sent.len(), 2);
    assert!(sent[1].0.ends_with("/api/licence/deactivate"), "the seat was left held");
    assert_eq!(sent[1].1["machine"], FP);
}

#[test]
fn a_lapsed_trial_is_expired_and_restricted_except_under_open() {
    let stub = Arc::new(Stub::answering(vec![ok(json!({
        "ok": true, "key": "LT-DATA-TR1A-L000-0000", "token": token("validTrial"),
        "machine": HASH, "expiresAt": "2025-11-08T08:53:20.000Z",
        "checkInBy": "2025-11-08T08:53:20.000Z"
    }))]));
    let cfg = config("trial", Arc::clone(&stub));
    let dir = cfg.dir.clone();
    let l = Licence::open(cfg);
    let said = l.trial_now("vj@example.com", "VJ").unwrap();
    assert!(said.starts_with("Trial, "), "{said}");
    assert_eq!(l.verdict().status, Status::Active);
    // The trial's key was kept, for checking in later.
    assert_eq!(store::load(&dir).key.as_deref(), Some("LT-DATA-TR1A-L000-0000"));
    let (url, body) = &stub.sent()[0];
    assert!(url.ends_with("/api/licence/trial"));
    assert_eq!(body["machine"], FP);
    assert_eq!(body["product"], "vizz");

    // Relaunched after the trial's end.
    let lapsed = Licence::open(Config { clock: later, ..config_at(&dir) });
    assert_eq!(lapsed.verdict().status, Status::Expired, "a lapsed trial is over, not a check-in");
    assert_eq!(lapsed.restriction(Policy::Watermark), Restriction::Watermark);
    assert_eq!(lapsed.restriction(Policy::Lock), Restriction::Lock);
    assert_eq!(lapsed.restriction(Policy::Open), Restriction::None);
    assert_eq!(lapsed.snapshot().unwrap().headline.text, "Trial ended");
    let _ = std::fs::remove_dir_all(dir);
}

/// The same lapse on a bought licence is a note and nothing more.
#[test]
fn a_lapsed_lease_on_a_bought_licence_restricts_nothing() {
    let cfg = config("lease", Arc::default());
    let dir = cfg.dir.clone();
    store::save(&dir, &store::Stored { key: Some(KEY.into()), token: Some(token("valid")) }).unwrap();
    let l = Licence::open(Config { clock: later, ..cfg });
    assert_eq!(l.verdict().status, Status::CheckInRequired);
    for policy in [Policy::Open, Policy::Watermark, Policy::Lock] {
        assert_eq!(l.restriction(policy), Restriction::None);
    }
    assert_eq!(l.snapshot().unwrap().headline.text, "Needs check-in");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_check_in_that_cannot_reach_the_service_keeps_the_cached_licence() {
    let stub = Arc::new(Stub::answering(vec![Err("connection refused".into())]));
    let cfg = config("offline", stub);
    let dir = cfg.dir.clone();
    let stored = store::Stored { key: Some(KEY.into()), token: Some(token("valid")) };
    store::save(&dir, &stored).unwrap();
    let l = Licence::open(cfg);
    assert!(matches!(l.check_in_now(), Err(net::Failure::Offline(_))));
    assert_eq!(store::load(&dir), stored, "a network failure changed the stored licence");
    assert_eq!(l.verdict().status, Status::Active);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_check_in_persists_the_new_token_and_sends_the_product() {
    let stub = Arc::new(Stub::answering(vec![ok(json!({
        "ok": true, "token": token("valid"), "machine": HASH,
        "checkInBy": "2025-12-08T00:00:00.000Z", "maintenanceUntil": "2026-10-09T00:00:00.000Z"
    }))]));
    let cfg = config("heartbeat", Arc::clone(&stub));
    let dir = cfg.dir.clone();
    store::save(&dir, &store::Stored { key: Some(KEY.into()), token: Some("stale.token".into()) }).unwrap();
    let l = Licence::open(cfg);
    assert_eq!(l.verdict().status, Status::Invalid);
    l.check_in_now().unwrap();
    assert_eq!(store::load(&dir).token, Some(token("valid")), "the fresh token was not persisted");
    assert_eq!(l.verdict().status, Status::Active);
    let (_, body) = &stub.sent()[0];
    assert_eq!(body["product"], "vizz");
    assert_eq!(body["machine"], FP);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_offline_token_is_kept_only_if_it_is_ours() {
    let cfg = config("offline-token", Arc::default());
    let dir = cfg.dir.clone();
    let l = Licence::open(cfg);
    assert!(l.use_token_now(&token("tampered")).is_err());
    assert!(l.use_token_now("not a token").is_err());
    assert!(!store::path(&dir).exists());
    // Pasted with a line break in it, as a terminal or an email wraps it.
    let valid = token("valid");
    let wrapped = format!("{}\n{}", &valid[..40], &valid[40..]);
    assert_eq!(l.use_token_now(&wrapped).unwrap(), "Licensed to Test Buyer");
    assert_eq!(store::load(&dir).key.as_deref(), Some(KEY), "the key comes from the token");

    // The same token on another machine is refused rather than stored.
    let other = config("offline-token-other", Arc::default());
    let other_dir = other.dir.clone();
    let l = Licence::open(Config { fingerprint: Some("ANOTHER-MACHINE".into()), ..other });
    let err = l.use_token_now(&valid).unwrap_err();
    assert!(err.contains("nothing was stored"), "{err}");
    assert!(!store::path(&other_dir).exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn release_forgets_locally_even_when_the_service_is_unreachable() {
    let stub = Arc::new(Stub::answering(vec![Err("offline".into())]));
    let cfg = config("release", Arc::clone(&stub));
    let dir = cfg.dir.clone();
    store::save(&dir, &store::Stored { key: Some(KEY.into()), token: Some(token("valid")) }).unwrap();
    let l = Licence::open(cfg);
    let said = l.release_now().unwrap();
    assert!(said.contains("release the seat from your account"), "{said}");
    assert!(!store::path(&dir).exists());
    assert_eq!(l.verdict().status, Status::Invalid);
    let (url, body) = &stub.sent()[0];
    assert!(url.ends_with("/api/licence/deactivate"));
    assert_eq!(body["machine"], FP);
    assert_eq!(body["key"], KEY);
}

/// Rule 4, through the whole handle: a build whose key cannot verify
/// restricts nothing, whatever is stored and whatever the policy.
#[test]
fn a_build_without_a_usable_key_restricts_nothing() {
    for key in ["", "REPLACE_WITH_YOUR_PUBLIC_KEY_HEX", "zz"] {
        let cfg = Config { public_key: key.into(), ..config("nokey", Arc::default()) };
        let l = Licence::open(cfg);
        assert!(!l.key_configured());
        for policy in [Policy::Open, Policy::Watermark, Policy::Lock] {
            assert_eq!(l.restriction(policy), Restriction::None, "{key:?} {policy:?}");
        }
        assert_eq!(l.snapshot().unwrap().headline.tone, Tone::Note);
    }
}

#[test]
fn the_shipped_key_is_compiled_in_and_the_build_is_dated() {
    assert!(key_usable(PUBLIC_KEY), "this build verifies nothing");
    assert_eq!(DEFAULT_PUBLIC_KEY, "1fca6c21f2eb7963fd646272a731a41a191d3a4cda839e295c5cda67978fcc85");
    const { assert!(BUILD_DATE > 1_750_000_000, "build.rs did not stamp a plausible date") };
    const { assert!(BUILD_DATE < 4_000_000_000) };
}

/// The spawning form: the panel presses, a thread does the work, and the
/// answer lands in the snapshot with the revision moved.
#[test]
fn a_pressed_button_runs_off_thread_and_reports_back() {
    let stub = Arc::new(Stub::answering(vec![refused(404, "unknown_key", "No licence has that key.")]));
    let l = Licence::open(config("spawn", stub));
    l.activate(KEY.into(), String::new());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let snap = loop {
        if let Some(s) = l.snapshot()
            && s.busy.is_none()
            && s.message.is_some()
        {
            break s;
        }
        assert!(std::time::Instant::now() < deadline, "the action never finished");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert_eq!(snap.message, Some(Message { text: "No licence has that key.".into(), error: true }));
}

/// A full refund ends the licence: a check-in the service answers with
/// `revoked` forgets the token and keeps the key, so this copy is
/// unlicensed from the next launch — and a reinstated licence comes back
/// with the next check-in, on the same key.
#[test]
fn a_revoked_licence_forgets_its_token_and_keeps_its_key() {
    let msg = "This licence was refunded and is no longer valid.";
    let stub = Arc::new(Stub::answering(vec![
        refused(403, "revoked", msg),
        ok(json!({
            "ok": true, "token": token("valid"), "machine": HASH,
            "checkInBy": "2025-12-08T00:00:00.000Z", "maintenanceUntil": "2026-10-09T00:00:00.000Z"
        })),
    ]));
    let cfg = config("revoked", Arc::clone(&stub));
    let dir = cfg.dir.clone();
    store::save(&dir, &store::Stored { key: Some(KEY.into()), token: Some(token("valid")) }).unwrap();
    let l = Licence::open(cfg);
    assert_eq!(l.verdict().status, Status::Active);
    let before = l.revision();

    let err = l.check_in_now().unwrap_err();
    assert_eq!(err.reason(), Some("revoked"));
    assert_eq!(err.to_string(), msg, "the service's own words");
    assert_eq!(
        store::load(&dir),
        store::Stored { key: Some(KEY.into()), token: None },
        "the token must go and the key must stay"
    );
    assert_eq!(l.verdict().status, Status::Invalid);
    assert!(l.revision() > before, "the render loop would not notice");
    assert_eq!(l.restriction(Policy::Lock), Restriction::Lock);
    // And a fresh start decides the same way.
    assert_eq!(Licence::open(config_at(&dir)).verdict().status, Status::Invalid);

    // Reinstated: the key still checks in, and the licence comes back.
    assert!(l.wants_check_in(), "a revoked copy stopped checking in, so it can never be reinstated");
    l.check_in_now().unwrap();
    assert_eq!(l.verdict().status, Status::Active);
    let _ = std::fs::remove_dir_all(dir);
}

/// Only `revoked` ends a licence. Every other refusal, and a server
/// error, keeps the cached token exactly as it was.
#[test]
fn any_other_refusal_keeps_the_token() {
    for (status, reason) in [
        (404, "not_activated"),
        (404, "unknown_key"),
        (403, "expired"),
        (409, "wrong_product"),
        (500, "server_error"),
    ] {
        let stub = Arc::new(Stub::answering(vec![refused(status, reason, "no")]));
        let cfg = config(&format!("keep-{reason}"), stub);
        let dir = cfg.dir.clone();
        let stored = store::Stored { key: Some(KEY.into()), token: Some(token("valid")) };
        store::save(&dir, &stored).unwrap();
        let l = Licence::open(cfg);
        assert_eq!(l.check_in_now().unwrap_err().reason(), Some(reason));
        assert_eq!(store::load(&dir), stored, "{reason} changed the stored licence");
        assert_eq!(l.verdict().status, Status::Active, "{reason}");
        let _ = std::fs::remove_dir_all(dir);
    }
    // And a 500 that is not even JSON.
    let stub = Arc::new(Stub::answering(vec![Ok((500, "Internal Server Error".into()))]));
    let cfg = config("keep-500", stub);
    let dir = cfg.dir.clone();
    let stored = store::Stored { key: Some(KEY.into()), token: Some(token("valid")) };
    store::save(&dir, &stored).unwrap();
    let l = Licence::open(cfg);
    assert!(matches!(l.check_in_now(), Err(net::Failure::Protocol(_))));
    assert_eq!(store::load(&dir), stored);
    let _ = std::fs::remove_dir_all(dir);
}
