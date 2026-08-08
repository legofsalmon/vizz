//! Update checking.
//!
//! Deliberately **notify-only**: vizz tells you a newer version exists and
//! links to it, but never downloads or replaces itself. Two reasons, both
//! about this being live-performance software:
//!
//! 1. An update that lands mid-set is exactly the failure a VJ cannot
//!    afford. A human choosing the moment is the whole point.
//! 2. Replacing a running, signed and notarized bundle in place means
//!    re-quarantine edge cases and torn-update recovery — a lot of
//!    fragile machinery for something a drag-and-drop already does
//!    reliably.
//!
//! The check runs once, on a background thread, with a short timeout, and
//! fails silently: no network, an offline venue, a rate-limited API or a
//! changed response shape all end with vizz simply not mentioning it.
//! It is also easy to turn off entirely — see `--no-update-check`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where the check looks. Public so the panel can link to the same place.
pub const RELEASES_URL: &str = "https://github.com/legofsalmon/vizz/releases/latest";
const API_URL: &str = "https://api.github.com/repos/legofsalmon/vizz/releases/latest";
/// Short: an unreachable network must not keep a thread alive for long.
const TIMEOUT: Duration = Duration::from_secs(8);

/// A semantic version, parsed leniently from a `vX.Y.Z` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Parse `v1.2.3`, `1.2.3`, or `1.2` (missing parts are zero).
    /// Anything after a `-` or `+` is ignored, which makes a pre-release
    /// compare equal to its base version — deliberately conservative, so
    /// a `-rc` tag never nags someone already on the final release.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches(['v', 'V']);
        let core = text.split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Some(Self { major, minor, patch })
    }

    /// This build's version, from Cargo.
    pub fn current() -> Self {
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version { major: 0, minor: 0, patch: 0 })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What the panel shows. `None` until the check finishes (or forever, if
/// it fails or is disabled).
#[derive(Debug, Clone, Default)]
pub struct UpdateStatus {
    pub available: Option<Version>,
}

pub type SharedUpdate = Arc<Mutex<UpdateStatus>>;

/// Start the check in the background. Returns immediately.
///
/// Nothing here can fail in a way the caller needs to handle: a failed
/// check simply leaves the status empty.
pub fn spawn_check(shared: SharedUpdate) {
    let current = Version::current();
    let _ = std::thread::Builder::new()
        .name("vizz-update".into())
        .spawn(move || match fetch_latest() {
            Ok(latest) if latest > current => {
                log::info!("update available: {latest} (running {current}) — {RELEASES_URL}");
                if let Ok(mut status) = shared.lock() {
                    status.available = Some(latest);
                }
            }
            Ok(latest) => log::debug!("up to date: running {current}, latest {latest}"),
            // Debug, not warn: an offline venue is normal, not a problem.
            Err(e) => log::debug!("update check skipped: {e:#}"),
        });
}

fn fetch_latest() -> anyhow::Result<Version> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .new_agent();
    let body = agent
        .get(API_URL)
        // GitHub rejects requests without one.
        .header("User-Agent", concat!("vizz/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_to_string()?;
    let tag = extract_tag(&body)
        .ok_or_else(|| anyhow::anyhow!("no tag_name in the release response"))?;
    Version::parse(&tag).ok_or_else(|| anyhow::anyhow!("could not parse version from {tag:?}"))
}

/// Pull `tag_name` out of the release JSON without a JSON dependency —
/// one string field from a response we do not otherwise care about.
fn extract_tag(json: &str) -> Option<String> {
    let key = "\"tag_name\"";
    let start = json.find(key)? + key.len();
    let rest = &json[start..];
    let open = rest.find('"')?;
    let after = &rest[open + 1..];
    let close = after.find('"')?;
    Some(after[..close].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_tag_shapes_releases_actually_use() {
        assert_eq!(Version::parse("v1.2.3"), Some(Version { major: 1, minor: 2, patch: 3 }));
        assert_eq!(Version::parse("1.2.3"), Some(Version { major: 1, minor: 2, patch: 3 }));
        assert_eq!(Version::parse("v0.1"), Some(Version { major: 0, minor: 1, patch: 0 }));
        assert_eq!(Version::parse("v2"), Some(Version { major: 2, minor: 0, patch: 0 }));
        assert_eq!(Version::parse(" v1.0.0\n"), Some(Version { major: 1, minor: 0, patch: 0 }));
        assert_eq!(Version::parse("not-a-version"), None);
        assert_eq!(Version::parse(""), None);
    }

    #[test]
    fn pre_release_tags_do_not_outrank_their_release() {
        // Someone on 0.2.0 must not be nagged by a 0.2.0-rc1 tag.
        let rc = Version::parse("v0.2.0-rc1").unwrap();
        let release = Version::parse("v0.2.0").unwrap();
        assert_eq!(rc, release);
        assert!((rc <= release));
    }

    #[test]
    fn ordering_is_by_component_not_lexicographic() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("v0.10.0") > v("v0.9.0"), "10 must beat 9, not sort before it");
        assert!(v("v1.0.0") > v("v0.99.99"));
        assert!(v("v0.1.10") > v("v0.1.9"));
        assert!((v("v0.1.0") <= v("v0.1.0")), "same version is not an update");
        assert!((v("v0.0.9") <= v("v0.1.0")), "older must never look newer");
    }

    #[test]
    fn current_version_is_readable() {
        // Guards against the Cargo version ever becoming unparseable.
        let current = Version::current();
        assert!(current.major > 0 || current.minor > 0 || current.patch > 0);
    }

    #[test]
    fn extracts_the_tag_from_a_realistic_release_payload() {
        let json = r#"{"url":"https://api.github.com/repos/x/y/releases/1",
            "assets_url":"...","tag_name":"v0.2.1","target_commitish":"main",
            "name":"v0.2.1","draft":false}"#;
        assert_eq!(extract_tag(json).as_deref(), Some("v0.2.1"));
    }

    #[test]
    fn malformed_payloads_yield_nothing_rather_than_panicking() {
        // A changed API shape must degrade to "no update shown".
        assert_eq!(extract_tag("{}"), None);
        assert_eq!(extract_tag(""), None);
        assert_eq!(extract_tag(r#"{"tag_name"}"#), None);
        assert_eq!(extract_tag(r#"{"tag_name": "unterminated"#), None);
        assert_eq!(extract_tag(r#"{"tag_name": "#), None);
    }
}
