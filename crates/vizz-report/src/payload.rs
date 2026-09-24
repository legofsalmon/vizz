//! The two bodies the intake takes, built so that the limits hold and the
//! forbidden fields cannot be expressed.
//!
//! A report is a struct with exactly the contract's fields and nothing
//! else — no free-form map, no `extra` — so a licence key, a show name or
//! an NDI source name has nowhere to go even by accident. The one field
//! that is free text by design, a crash's `note`, is typed by the person
//! and shown to them before it is sent; it is scrubbed like everything
//! else, and the server never forwards it anywhere public.

use serde::{Deserialize, Serialize};

use crate::scrub::{Identity, cut, scrub};

/// The contract's limits, in characters.
pub mod limit {
    pub const VERSION: usize = 32;
    pub const OS_VERSION: usize = 32;
    pub const ARCH: usize = 16;
    pub const INSTALL: usize = 64;
    pub const SUMMARY: usize = 300;
    pub const DETAIL: usize = 32_768;
    pub const SIGNATURE: usize = 128;
    pub const NOTE: usize = 2000;
    pub const MESSAGE: usize = 5000;
    pub const EMAIL: usize = 254;
    pub const NAME: usize = 100;
    /// Whole body, in bytes.
    pub const BODY: usize = 64 * 1024;
}

/// What happened, in the contract's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CrashKind {
    Panic,
    Exception,
    Signal,
    UncleanExit,
    Gpu,
    Hang,
    Other,
}

/// `POST /api/reports/crash`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub product: String,
    pub version: String,
    pub os: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<String>,
    pub kind: CrashKind,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The four kinds of feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackKind {
    Bug,
    Idea,
    Question,
    Praise,
}

impl FeedbackKind {
    pub const ALL: [FeedbackKind; 4] =
        [FeedbackKind::Bug, FeedbackKind::Idea, FeedbackKind::Question, FeedbackKind::Praise];
}

/// `POST /api/reports/feedback`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feedback {
    pub product: String,
    pub version: String,
    pub os: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<String>,
    #[serde(rename = "type")]
    pub kind: FeedbackKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
    pub public: bool,
}

/// Who is sending: the same for every report from this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub product: &'static str,
    pub version: String,
    pub os: &'static str,
    pub os_version: Option<String>,
    pub arch: Option<String>,
    pub install: Option<String>,
}

impl Origin {
    /// This build on this machine.
    pub fn current(product: &'static str, version: &str, install: Option<String>) -> Self {
        Origin {
            product,
            version: cut(version, limit::VERSION),
            os: os(),
            os_version: sysinfo::System::os_version().map(|v| cut(&v, limit::OS_VERSION)),
            arch: Some(arch().to_string()),
            install: install.and_then(|i| valid_install(&i).then_some(i)),
        }
    }
}

/// The contract's name for this OS.
pub fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "linux"
    }
}

/// The contract spells Apple Silicon `arm64`; Rust spells it `aarch64`.
pub fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    }
}

/// `[A-Za-z0-9-]`, at most 64: what the contract accepts as an install id.
pub fn valid_install(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= limit::INSTALL
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// What a crash looked like at the moment it was caught, before any of
/// it has been scrubbed.
#[derive(Debug, Clone, Default)]
pub struct Raw {
    pub summary: String,
    pub detail: Option<String>,
    pub occurred_at: Option<String>,
}

/// Build a crash report: scrubbed, cut to the limits, signed.
pub fn crash(origin: &Origin, who: &Identity, kind: CrashKind, raw: &Raw) -> CrashReport {
    let first_line = raw.summary.lines().find(|l| !l.trim().is_empty()).unwrap_or("(no message)");
    let summary = scrub(first_line.trim(), who, limit::SUMMARY);
    let detail = raw
        .detail
        .as_deref()
        .map(|d| scrub(d, who, limit::DETAIL))
        .filter(|d| !d.trim().is_empty());
    let signature = Some(signature(kind, &summary, detail.as_deref()));
    CrashReport {
        product: origin.product.to_string(),
        version: origin.version.clone(),
        os: origin.os.to_string(),
        os_version: origin.os_version.clone(),
        arch: origin.arch.clone().map(|a| cut(&a, limit::ARCH)),
        install: origin.install.clone(),
        kind,
        summary,
        detail,
        signature,
        occurred_at: raw.occurred_at.clone(),
        note: None,
    }
}

/// Attach what the person typed about what they were doing. Scrubbed:
/// people paste paths.
pub fn with_note(mut report: CrashReport, note: &str, who: &Identity) -> CrashReport {
    let note = note.trim();
    report.note = (!note.is_empty()).then(|| scrub(note, who, limit::NOTE));
    report
}

/// What the feedback form produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FeedbackForm {
    pub message: String,
    pub email: String,
    pub name: String,
    /// "Include my licence so you know who I am" was ticked.
    pub include_licence: bool,
    /// "OK to post this publicly…" was ticked.
    pub public: bool,
}

/// Why a form cannot be sent yet, in words for the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
    Empty,
    TooLong,
    BadEmail,
}

impl std::fmt::Display for FormError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FormError::Empty => "write a message first",
            FormError::TooLong => "that is longer than 5000 characters — trim it a little",
            FormError::BadEmail => "that email address looks wrong",
        })
    }
}

/// Build a feedback body. The licence is attached only when the box was
/// ticked *and* the app has one; the message is the person's own words
/// and is sent as written, apart from the home-path rule — a pasted log
/// line should not carry a user name the person never meant to share.
pub fn feedback(
    origin: &Origin,
    who: &Identity,
    kind: FeedbackKind,
    form: &FeedbackForm,
    licence: Option<&str>,
) -> Result<Feedback, FormError> {
    let message = form.message.trim();
    if message.is_empty() {
        return Err(FormError::Empty);
    }
    if message.chars().count() > limit::MESSAGE {
        return Err(FormError::TooLong);
    }
    let email = form.email.trim();
    if !email.is_empty() && (email.chars().count() > limit::EMAIL || !looks_like_email(email)) {
        return Err(FormError::BadEmail);
    }
    let name = form.name.trim();
    Ok(Feedback {
        product: origin.product.to_string(),
        version: origin.version.clone(),
        os: origin.os.to_string(),
        install: origin.install.clone(),
        kind,
        message: scrub(message, who, limit::MESSAGE),
        email: (!email.is_empty()).then(|| email.to_string()),
        name: (!name.is_empty()).then(|| cut(name, limit::NAME)),
        licence: if form.include_licence { licence.map(str::to_string) } else { None },
        public: form.public,
    })
}

/// The service's own test: something, an `@`, something with a dot.
fn looks_like_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else { return false };
    !local.is_empty()
        && !s.chars().any(char::is_whitespace)
        && !domain.contains('@')
        && domain.split('.').count() >= 2
        && domain.split('.').all(|p| !p.is_empty())
}

/// "The same crash": a short hash of the kind, the summary with its
/// numbers taken out, and the first few frames of our own code with
/// their addresses taken out. Stable across Rust releases because it is
/// SHA-256 rather than std's hasher, and across runs because nothing in
/// it moves when ASLR does.
pub fn signature(kind: CrashKind, summary: &str, detail: Option<&str>) -> String {
    let mut text = format!("{kind:?}\n{}\n", normalise(summary));
    for frame in detail.map(frames).unwrap_or_default().into_iter().take(5) {
        text.push_str(&frame);
        text.push('\n');
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, text.as_bytes());
    digest.as_ref()[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// Digits to `N`, so "len is 3 but index is 7" and "len is 4 but index
/// is 9" are one crash.
fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_number = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !in_number {
                out.push('N');
            }
            in_number = true;
        } else {
            in_number = false;
            out.push(c);
        }
    }
    out
}

/// The function names of a Rust backtrace, ours first: the frames that
/// are the panic machinery itself (`std::panicking`, `core::panicking`,
/// `rust_begin_unwind`, the backtrace capture) say nothing about which
/// crash this is.
pub fn frames(backtrace: &str) -> Vec<String> {
    backtrace
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (index, name) = line.split_once(": ")?;
            index.chars().all(|c| c.is_ascii_digit()).then_some(name)
        })
        .map(|name| {
            // `vizz::engine::Frame::tick::h1a2b3c4d5e6f7a8b` → drop the
            // hash suffix, which changes with every build.
            match name.rsplit_once("::h") {
                Some((head, hash)) if hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit()) => {
                    head.to_string()
                }
                _ => name.to_string(),
            }
        })
        .filter(|name| {
            !(name.starts_with("std::")
                || name.starts_with("core::")
                || name.starts_with("alloc::")
                || name.starts_with("<alloc::")
                || name.starts_with("rust_begin_unwind")
                || name.starts_with("__rust")
                || name.starts_with("vizz_report::")
                || name.starts_with("0x"))
        })
        .collect()
}

/// A body that is safe to send: under the size limit whatever the
/// detail was. Shortens the detail rather than refusing, because a crash
/// with half its trace is worth more than no crash.
pub fn to_body(report: &CrashReport) -> String {
    let mut r = report.clone();
    loop {
        let body = serde_json::to_string(&r).unwrap_or_default();
        if body.len() <= limit::BODY {
            return body;
        }
        match r.detail.as_mut() {
            Some(d) if !d.is_empty() => {
                let keep = d.chars().count() * 3 / 4;
                *d = cut(d, keep);
            }
            _ => {
                r.note = r.note.map(|n| cut(&n, 200));
                return serde_json::to_string(&r).unwrap_or_default();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> Origin {
        Origin {
            product: "vizz",
            version: "1.0.0".into(),
            os: "macos",
            os_version: Some("14.6".into()),
            arch: Some("arm64".into()),
            install: Some("3f0c9a52-5d1e-4c1b-9a3e-2b7f0d8c1e44".into()),
        }
    }

    fn who() -> Identity {
        Identity { home: Some("/Users/colm".into()), user: Some("colm".into()) }
    }

    const TRACE: &str = "   0: __rustc::rust_begin_unwind
   0: std::backtrace::Backtrace::force_capture
   1: vizz_report::crash::hook::{{closure}}
   2: std::panicking::rust_panic_with_hook
   3: std::panicking::begin_panic_handler::{{closure}}
   4: rust_begin_unwind
   5: core::panicking::panic_fmt
   6: core::panicking::panic_bounds_check
   7: vizz_render::particles::ParticleScene::draw::h0123456789abcdef
             at /Users/colm/src/vizz/crates/vizz-render/src/particles.rs:812:17
   8: vizz::windowed::App::redraw::hfedcba9876543210
             at /Users/colm/src/vizz/crates/vizz-app/src/windowed.rs:1400:9";

    #[test]
    fn the_crash_body_has_exactly_the_contracts_fields() {
        let r = crash(
            &origin(),
            &who(),
            CrashKind::Panic,
            &Raw {
                summary: "index out of bounds: the len is 3 but the index is 7\nsecond line".into(),
                detail: Some(TRACE.into()),
                occurred_at: Some("2026-09-24T02:10:00Z".into()),
            },
        );
        let v: serde_json::Value = serde_json::from_str(&to_body(&r)).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        let allowed = [
            "product", "version", "os", "osVersion", "arch", "install", "kind", "summary",
            "detail", "signature", "occurredAt", "note",
        ];
        for k in &keys {
            assert!(allowed.contains(k), "field {k} is not in the contract");
        }
        for k in ["product", "version", "os", "kind", "summary"] {
            assert!(keys.contains(&k), "required field {k} missing");
        }
        assert_eq!(v["product"], "vizz");
        assert_eq!(v["kind"], "panic");
        assert_eq!(v["summary"], "index out of bounds: the len is 3 but the index is 7");
        let detail = v["detail"].as_str().unwrap();
        assert!(!detail.contains("/Users/colm"), "home path survived: {detail}");
        assert!(detail.contains("~/src/vizz/crates"), "{detail}");
        assert_eq!(v["signature"].as_str().unwrap().len(), 12);
        assert!(v.get("note").is_none(), "no note unless one was typed");
    }

    #[test]
    fn kinds_are_spelled_as_the_contract_spells_them() {
        let spelled: Vec<String> = [
            CrashKind::Panic,
            CrashKind::Exception,
            CrashKind::Signal,
            CrashKind::UncleanExit,
            CrashKind::Gpu,
            CrashKind::Hang,
            CrashKind::Other,
        ]
        .iter()
        .map(|k| serde_json::to_value(k).unwrap().as_str().unwrap().to_string())
        .collect();
        assert_eq!(spelled, ["panic", "exception", "signal", "unclean-exit", "gpu", "hang", "other"]);
    }

    #[test]
    fn every_field_is_cut_to_its_limit_and_the_body_to_64k() {
        let mut o = origin();
        o.version = cut(&"9".repeat(80), limit::VERSION);
        let r = crash(
            &o,
            &Identity::default(),
            CrashKind::Panic,
            &Raw { summary: "s".repeat(1000), detail: Some("d\n".repeat(60_000)), occurred_at: None },
        );
        assert_eq!(r.version.chars().count(), limit::VERSION);
        assert_eq!(r.summary.chars().count(), limit::SUMMARY);
        assert!(r.detail.as_ref().unwrap().chars().count() <= limit::DETAIL);
        let r = with_note(r, &"n".repeat(5000), &Identity::default());
        assert_eq!(r.note.as_ref().unwrap().chars().count(), limit::NOTE);
        assert!(to_body(&r).len() <= limit::BODY);

        // A multi-byte detail right at the limit still fits the body.
        let r = crash(
            &origin(),
            &Identity::default(),
            CrashKind::Panic,
            &Raw { summary: "x".into(), detail: Some("é".repeat(40_000)), occurred_at: None },
        );
        assert!(to_body(&r).len() <= limit::BODY);
    }

    #[test]
    fn the_same_crash_signs_the_same_and_a_different_one_does_not() {
        let a = signature(CrashKind::Panic, "index 7 of len 3", Some(TRACE));
        let b = signature(
            CrashKind::Panic,
            "index 9 of len 4",
            Some(&TRACE.replace("h0123456789abcdef", "haaaaaaaaaaaaaaaa")),
        );
        assert_eq!(a, b, "numbers and build hashes must not split one crash in two");
        let c = signature(CrashKind::Panic, "index 7 of len 3", Some(&TRACE.replace("draw", "upload")));
        assert_ne!(a, c);
        assert_ne!(a, signature(CrashKind::Gpu, "index 7 of len 3", Some(TRACE)));
    }

    #[test]
    fn frames_skip_the_panic_machinery() {
        let f = frames(TRACE);
        assert_eq!(f[0], "vizz_render::particles::ParticleScene::draw");
        assert_eq!(f[1], "vizz::windowed::App::redraw");
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn feedback_carries_the_licence_only_when_ticked() {
        let form = FeedbackForm {
            message: "Let me map a MIDI fader to the feedback amount".into(),
            ..Default::default()
        };
        let f = feedback(&origin(), &who(), FeedbackKind::Idea, &form, Some("LT-VIZZ-AAAA-BBBB-CCCC")).unwrap();
        assert_eq!(f.licence, None, "a licence went out without the box ticked");
        assert!(!f.public, "public must default to false");
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["type"], "idea");
        assert!(v.get("licence").is_none() && v.get("email").is_none() && v.get("name").is_none());
        assert_eq!(v["public"], false);

        let ticked = FeedbackForm { include_licence: true, public: true, ..form.clone() };
        let f = feedback(&origin(), &who(), FeedbackKind::Bug, &ticked, Some("LT-VIZZ-AAAA-BBBB-CCCC")).unwrap();
        assert_eq!(f.licence.as_deref(), Some("LT-VIZZ-AAAA-BBBB-CCCC"));
        assert!(f.public);
        // Ticked, but the app has no licence: nothing to include.
        let f = feedback(&origin(), &who(), FeedbackKind::Bug, &ticked, None).unwrap();
        assert_eq!(f.licence, None);
    }

    #[test]
    fn feedback_validates_like_the_form_says() {
        let o = origin();
        let w = Identity::default();
        let form = |m: &str, e: &str| FeedbackForm { message: m.into(), email: e.into(), ..Default::default() };
        assert_eq!(feedback(&o, &w, FeedbackKind::Bug, &form("  ", ""), None), Err(FormError::Empty));
        assert_eq!(
            feedback(&o, &w, FeedbackKind::Bug, &form(&"x".repeat(5001), ""), None),
            Err(FormError::TooLong)
        );
        assert_eq!(feedback(&o, &w, FeedbackKind::Bug, &form("hi", "nope"), None), Err(FormError::BadEmail));
        assert_eq!(feedback(&o, &w, FeedbackKind::Bug, &form("hi", "a@b"), None), Err(FormError::BadEmail));
        let f = feedback(&o, &w, FeedbackKind::Question, &form("hi", " vj@example.com "), None).unwrap();
        assert_eq!(f.email.as_deref(), Some("vj@example.com"));
        assert!(feedback(&o, &w, FeedbackKind::Praise, &form(&"x".repeat(5000), ""), None).is_ok());
    }

    #[test]
    fn an_install_id_is_only_ever_the_contracts_shape() {
        assert!(valid_install("3f0c9a52-5d1e-4c1b-9a3e-2b7f0d8c1e44"));
        assert!(!valid_install(""));
        assert!(!valid_install("has space"));
        assert!(!valid_install(&"a".repeat(65)));
        let o = Origin::current("vizz", "1.0.0", Some("bad id!".into()));
        assert_eq!(o.install, None, "a malformed id must be dropped, not sent");
        assert_eq!(o.os, os());
    }
}
