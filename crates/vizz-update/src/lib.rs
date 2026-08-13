//! Update checking, and — on macOS — installing.
//!
//! This was notify-only, on two arguments. The first still stands and is
//! now enforced rather than assumed: **an update must never land
//! mid-set**, so every step here is something a person pressed. The check
//! is automatic; the download is not, the install is not, and the install
//! refuses outright while a recording is running. Nothing about a venue's
//! bandwidth or a laptop's disk is spent without being asked.
//!
//! The second argument — that replacing a running signed bundle is
//! fragile machinery — is answered by not replacing it while it is
//! running. vizz downloads and verifies the new bundle, then leaves a
//! script that waits for the process to exit before swapping and
//! relaunching. See [`install`]. What is left is a drag-and-drop that
//! somebody else does, which is the part that was actually painful.
//!
//! The check runs once, on a background thread, with a short timeout, and
//! fails silently: no network, an offline venue, a rate-limited API or a
//! changed response shape all end with vizz simply not mentioning it.
//! It is also easy to turn off entirely — see `--no-update-check`.

pub mod install;

use std::path::PathBuf;
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

/// A release, and the bundle hanging off it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// Direct download for the macOS bundle.
    pub asset_url: String,
    pub asset_name: String,
    /// Bytes, for the progress bar. Zero when GitHub did not say.
    pub size: u64,
}

/// How far along an update is.
///
/// One value rather than a set of booleans: the states are genuinely
/// exclusive, and a struct of flags is how a UI ends up showing a
/// progress bar beside a download button.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Stage {
    /// Nothing doing — either up to date, or offered and not taken up.
    #[default]
    Idle,
    Downloading {
        done: u64,
        total: u64,
    },
    /// Downloaded, verified, and waiting for the word to go in.
    Ready(PathBuf),
    /// The swap is scheduled and vizz is on its way out.
    Installing,
    /// Something went wrong, in words worth showing.
    Failed(String),
}

/// What the panel shows. `None` until the check finishes (or forever, if
/// it fails or is disabled).
#[derive(Debug, Clone, Default)]
pub struct UpdateStatus {
    pub available: Option<Version>,
    /// The release behind `available`, when the check could resolve one.
    /// `None` on a release with no macOS asset, which is a release that
    /// can be linked to but not installed.
    pub release: Option<Release>,
    pub stage: Stage,
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
            Ok(release) if release.version > current => {
                let latest = release.version;
                log::info!("update available: {latest} (running {current}) — {RELEASES_URL}");
                if let Ok(mut status) = shared.lock() {
                    status.available = Some(latest);
                    status.release = Some(release);
                }
            }
            Ok(release) => {
                log::debug!("up to date: running {current}, latest {}", release.version)
            }
            // Debug, not warn: an offline venue is normal, not a problem.
            Err(e) => log::debug!("update check skipped: {e:#}"),
        });
}

fn fetch_latest() -> anyhow::Result<Release> {
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
    let version = Version::parse(&tag)
        .ok_or_else(|| anyhow::anyhow!("could not parse version from {tag:?}"))?;
    // The asset is optional: a release can exist and be worth telling
    // somebody about even if this build cannot install it.
    let (asset_url, asset_name, size) = extract_asset(&body).unwrap_or_default();
    Ok(Release { version, asset_url, asset_name, size })
}

/// Find the macOS bundle among a release's assets.
///
/// Hand-parsed for the same reason `extract_tag` is: three fields out of
/// a response we do not otherwise care about, and a JSON dependency for
/// it would be the largest thing in this crate. The assets array is
/// walked by splitting on the download-url key, so the name and size
/// picked up are the ones belonging to that same asset rather than the
/// first of each field anywhere in the document.
fn extract_asset(json: &str) -> Option<(String, String, u64)> {
    for chunk in json.split("\"browser_download_url\"").skip(1) {
        let url = quoted(chunk)?;
        if !url.ends_with(".app.zip") {
            continue;
        }
        // Name and size sit before the url within the same asset object,
        // so look back rather than forward.
        let before = &json[..json.find(&url)?];
        let start = before.rfind('{')?;
        let object = &before[start..];
        let name = object
            .split("\"name\"")
            .nth(1)
            .and_then(quoted)
            .unwrap_or_else(|| "vizz.app.zip".to_string());
        let size = object
            .split("\"size\"")
            .nth(1)
            .and_then(|s| {
                s.trim_start_matches([':', ' '])
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|n| n.parse().ok())
            })
            .unwrap_or(0);
        return Some((url, name, size));
    }
    None
}

/// The first double-quoted string in `s`.
fn quoted(s: &str) -> Option<String> {
    let open = s.find('"')?;
    let rest = &s[open + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_owned())
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

    /// The right asset, with the name and size that belong to it.
    ///
    /// A release carries several assets and GitHub's JSON repeats the
    /// same field names inside each one. Taking the first `"name"` or
    /// `"size"` in the document rather than the ones inside the matched
    /// asset's own object is the obvious way to write this and is wrong
    /// in a way nothing would notice until a progress bar filled to 400%
    /// — so the payload below deliberately puts a decoy asset first,
    /// with a different name and size.
    #[test]
    fn finds_the_mac_bundle_among_a_releases_other_assets() {
        let json = r#"{"tag_name":"v0.18.0","assets":[
            {"name":"checksums.txt","size":128,
             "browser_download_url":"https://example.test/checksums.txt"},
            {"name":"vizz-0.18.0.app.zip","size":41234567,
             "browser_download_url":"https://example.test/vizz-0.18.0.app.zip"}
        ]}"#;
        let (url, name, size) = extract_asset(json).expect("no bundle found");
        assert_eq!(url, "https://example.test/vizz-0.18.0.app.zip");
        assert_eq!(name, "vizz-0.18.0.app.zip", "took a neighbouring asset's name");
        assert_eq!(size, 41_234_567, "took a neighbouring asset's size");
    }

    /// A release with no macOS bundle is still a release.
    ///
    /// It has to degrade to "there is an update, here is the link"
    /// rather than to an error or, worse, to a download button that
    /// fetches a checksums file and tries to run it.
    #[test]
    fn a_release_without_a_bundle_offers_no_download() {
        let json = r#"{"tag_name":"v0.18.0","assets":[
            {"name":"notes.txt","size":10,
             "browser_download_url":"https://example.test/notes.txt"}]}"#;
        assert_eq!(extract_asset(json), None);
        assert_eq!(extract_asset(r#"{"tag_name":"v0.18.0","assets":[]}"#), None);
        assert_eq!(extract_asset("{}"), None);
        // And a truncated response is nothing, not a panic.
        assert_eq!(extract_asset(r#"{"browser_download_url":"#), None);
        assert_eq!(extract_asset(r#"{"browser_download_url":"unterminated"#), None);
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

    /// The site's download button must name the version this workspace
    /// builds.
    ///
    /// The button used to use GitHub's `/releases/latest/download/`
    /// permalink, which never goes stale — but a permalink can only
    /// resolve a constant filename, and the published download is now
    /// version-stamped so a Downloads folder full of them can be told
    /// apart. Pinning the link is the price of that, and a pinned link
    /// is exactly the kind of thing that is forgotten during a release
    /// and discovered by a user downloading the wrong version. So it is
    /// checked here, against the version the binaries will report.
    ///
    /// Only the release path is asserted, not the filename: the naming
    /// scheme is enforced where the asset is produced (make-app.sh and
    /// release.yml), and releases published before the scheme changed
    /// legitimately carry the older name.
    #[test]
    fn the_sites_download_button_points_at_this_version() {
        let site = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../site/index.html");
        let html = std::fs::read_to_string(&site).expect("site/index.html missing");
        let href = html
            .split("href=\"")
            .find(|h| h.contains("/releases/download/"))
            .map(|h| h.split('"').next().unwrap_or_default())
            .expect("no pinned download link on the landing page");
        let want = format!("/releases/download/v{}/", env!("CARGO_PKG_VERSION"));
        assert!(
            href.contains(&want),
            "the download button points at {href}, but this workspace is \
             version {} — update site/index.html in the version-bump commit",
            env!("CARGO_PKG_VERSION")
        );
        assert!(
            href.contains("vizz") && href.ends_with(".app.zip"),
            "the download button does not point at an app bundle: {href}"
        );
    }
}
