//! Downloading a release and putting it in place, on macOS.
//!
//! The rest of this crate is notify-only, and said so for two reasons.
//! The first — an update must never land mid-set — is still right, and is
//! answered here by making every step something a person pressed:
//! nothing downloads until asked, nothing installs until asked again, and
//! the install refuses outright while a recording is running. The second
//! — that replacing a running bundle is fragile — is answered by not
//! doing it: the swap happens after vizz has exited, from a small script
//! that vizz leaves behind, so nothing is ever rewritten underneath a
//! running process.
//!
//! macOS only, because the published release is macOS only: one
//! `vizz-X.Y.Z.app.zip` built by `release.yml` on `macos-latest`. On
//! other platforms the notify-only path stands.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};

use crate::{Release, SharedUpdate, Stage, TIMEOUT};

/// Where a download is assembled before it is trusted.
///
/// `temp_dir` is `$TMPDIR` on macOS, which is per-user and mode 700 —
/// not `/tmp`. That is load-bearing rather than incidental: between
/// [`verify`] passing and the swap script moving the bundle there is a
/// window, and a staging directory another account could write to would
/// make that window exploitable. Nobody but this user can reach it.
fn staging_dir() -> PathBuf {
    std::env::temp_dir().join("vizz-update")
}

/// The swap script, deliberately *outside* the staging directory.
///
/// The script's last act is to delete the staging directory, and a
/// script that deletes the directory it is being read from is a script
/// that may or may not finish depending on how much of it the shell has
/// buffered. Keeping it one level up makes that a non-question.
fn script_path() -> PathBuf {
    std::env::temp_dir().join("vizz-update-swap.sh")
}

/// The bundle this process is running from.
///
/// `current_exe` is `…/vizz.app/Contents/MacOS/vizz`, so the bundle is
/// three levels up. `None` when vizz is running as a bare binary — from
/// `cargo run`, or a build somebody put on their PATH — which is not a
/// thing that can be updated in place and must not be guessed at.
pub fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()?.eq_ignore_ascii_case("app")).then(|| bundle.to_path_buf())
}

/// Why this install cannot self-update, in words for the panel.
///
/// Checked before anything is downloaded. Spending a venue's bandwidth on
/// 40 MB and *then* saying "actually this copy cannot be replaced" is the
/// version of this feature nobody forgives.
pub fn blocker() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return Some("in-app updates are macOS only".into());
    }
    let Some(bundle) = running_bundle() else {
        return Some("this is not an app bundle — update by rebuilding".into());
    };
    // App Translocation: launched from a quarantined Downloads folder,
    // macOS runs the bundle from a randomised read-only mount. The path
    // under it cannot be written and will not survive a relaunch.
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return Some("move vizz to Applications first, then updates can install themselves".into());
    }
    if team_id(&bundle).is_none() {
        // Nothing to compare a download against. See `verify`.
        return Some("this build is not Developer ID signed — update by downloading".into());
    }
    // The move happens from a script, but it is still worth saying now
    // rather than after the download. A directory we cannot even stat is
    // *not* reported as a blocker: the install would fail with a real
    // error naming the real problem, which beats guessing at one here.
    let parent = bundle.parent()?;
    let readonly = parent.metadata().ok()?.permissions().readonly();
    readonly.then(|| format!("{} is read-only", parent.display()))
}

/// The Developer ID team a bundle is signed by, or `None` for ad-hoc and
/// unsigned bundles.
///
/// `codesign` prints this to stderr, which is why the output is read from
/// there rather than stdout.
fn team_id(bundle: &Path) -> Option<String> {
    let out = Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=4"])
        .arg(bundle)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let team = text
        .lines()
        .find_map(|l| l.strip_prefix("TeamIdentifier="))?
        .trim();
    // Ad-hoc signatures report the field with this literal value.
    (team != "not set" && !team.is_empty()).then(|| team.to_string())
}

/// Download, unpack and verify. Long-running; call it off the render
/// thread.
///
/// Progress is written through `shared` as it goes, so the panel can show
/// a bar rather than a frozen button.
pub fn spawn_fetch(shared: SharedUpdate, release: Release) {
    let _ = std::thread::Builder::new()
        .name("vizz-update-fetch".into())
        .spawn(move || {
            let set = |stage: Stage| {
                if let Ok(mut s) = shared.lock() {
                    s.stage = stage;
                }
            };
            let progress = Arc::new(AtomicU64::new(0));
            let watched = Arc::clone(&progress);
            let total = release.size;
            // The download writes bytes into the counter; a second thread
            // turns that into stage updates. Locking the shared mutex per
            // 8 KB chunk would be a lock held sixty times a second against
            // the render thread's `try_lock`, which is the one thing this
            // crate must not do.
            let ticker = {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || {
                    loop {
                        let done = watched.load(Ordering::Relaxed);
                        if done == u64::MAX {
                            break;
                        }
                        if let Ok(mut s) = shared.lock() {
                            s.stage = Stage::Downloading { done, total };
                        }
                        std::thread::sleep(std::time::Duration::from_millis(120));
                    }
                })
            };

            let result = fetch_and_verify(&release, &progress);
            progress.store(u64::MAX, Ordering::Relaxed);
            let _ = ticker.join();

            match result {
                Ok(staged) => {
                    log::info!("update {} staged at {}", release.version, staged.display());
                    set(Stage::Ready(staged));
                }
                Err(e) => {
                    log::warn!("update download failed: {e:#}");
                    // The whole chain, not just the outermost frame: "could
                    // not verify the download" without the reason is a
                    // message that sends someone to a forum.
                    set(Stage::Failed(format!("{e:#}")));
                }
            }
        });
}

fn fetch_and_verify(release: &Release, progress: &AtomicU64) -> Result<PathBuf> {
    let dir = staging_dir();
    // A previous attempt's leftovers must never be mistaken for this
    // one's download.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    let zip = dir.join(&release.asset_name);

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT.saturating_mul(60)))
        .build()
        .new_agent();
    let mut response = agent
        .get(&release.asset_url)
        .header("User-Agent", concat!("vizz/", env!("CARGO_PKG_VERSION")))
        .call()
        .context("could not reach the download")?;

    {
        let mut out = std::io::BufWriter::new(
            std::fs::File::create(&zip).with_context(|| format!("could not write {}", zip.display()))?,
        );
        let mut reader = response.body_mut().as_reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut done: u64 = 0;
        loop {
            let n = reader.read(&mut buf).context("the download stopped early")?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut out, &buf[..n])?;
            done += n as u64;
            progress.store(done, Ordering::Relaxed);
        }
        std::io::Write::flush(&mut out)?;
    }

    // `ditto`, not a zip crate: this is a signed app bundle, and the
    // signature covers extended attributes and symlinks that a naive
    // extractor drops — producing a bundle that unpacks fine and then
    // fails to launch. `ditto` is what `release.yml` packed it with.
    let unpacked = dir.join("unpacked");
    let status = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(&zip)
        .arg(&unpacked)
        .status()
        .context("could not run ditto")?;
    if !status.success() {
        bail!("the download could not be unpacked (ditto exited {status})");
    }

    let bundle = find_bundle(&unpacked)?;
    verify(&bundle, release)?;
    Ok(bundle)
}

fn find_bundle(dir: &Path) -> Result<PathBuf> {
    std::fs::read_dir(dir)
        .with_context(|| format!("could not read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("app")))
        .context("the download contained no .app bundle")
}

/// Refuse anything we cannot show came from the same publisher as the
/// copy that is running.
///
/// This is the security boundary of the whole feature. Everything up to
/// here is bytes off the network being turned into an executable, and
/// TLS only says they came from GitHub — not that GitHub served what we
/// think, nor that the release was published by us.
///
/// So: the bundle must carry a valid signature, and its Developer ID team
/// must be the team that signed the copy currently running. An attacker
/// who can serve a substitute download still cannot produce one signed by
/// that team. A build with no team of its own is refused earlier, in
/// [`blocker`], because it has no chain to compare against — it would be
/// trusting the download to vouch for itself.
fn verify(bundle: &Path, release: &Release) -> Result<()> {
    let out = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--deep"])
        .arg(bundle)
        .output()
        .context("could not run codesign")?;
    if !out.status.success() {
        bail!(
            "the download is not correctly signed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let running = running_bundle().context("no running bundle to compare against")?;
    let ours = team_id(&running).context("the running copy is not Developer ID signed")?;
    let theirs = team_id(bundle).context("the download is not Developer ID signed")?;
    if ours != theirs {
        bail!("the download is signed by {theirs}, but this copy is signed by {ours}");
    }

    // And it must actually be the version that was offered — a release
    // whose asset does not match its tag is a mistake worth catching
    // before it is installed rather than after.
    let got = bundle_version(bundle).context("the download has no readable version")?;
    if got != release.version {
        bail!("the download is version {got}, but {} was offered", release.version);
    }
    Ok(())
}

fn bundle_version(bundle: &Path) -> Option<crate::Version> {
    let out = Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(bundle.join("Contents/Info.plist"))
        .arg("CFBundleShortVersionString")
        .output()
        .ok()?;
    crate::Version::parse(&String::from_utf8_lossy(&out.stdout))
}

/// Swap the staged bundle in and relaunch, once this process has exited.
///
/// Returns after the helper is running and detached; the caller is
/// expected to quit promptly. Nothing is moved while vizz is alive — the
/// helper waits for the pid to go away first — so a crash between here
/// and exit leaves the installed copy untouched.
///
/// The rollback matters more than it looks: the move-aside and the
/// move-in are two operations, and the window between them is the one
/// state where there is no vizz in /Applications. If the second fails,
/// the helper puts the original back rather than leaving a machine with
/// no app on it an hour before doors.
pub fn install_and_restart(staged: &Path) -> Result<()> {
    // Belt and braces with `blocker`, which the panel consults before
    // showing the button: nothing should be able to reach the swap on a
    // platform whose bundle layout this does not understand.
    if !cfg!(target_os = "macos") {
        bail!("in-app updates are macOS only");
    }
    let target = running_bundle().context("not running from an app bundle")?;
    let script = script_path();
    let body = format!(
        r#"#!/bin/sh
# Written by vizz to replace itself. Safe to delete.
set -u
PID={pid}
TARGET={target}
NEW={new}
BAK="$TARGET.replaced"

# Wait for vizz to exit; give up after 60s rather than lingering forever.
i=0
while kill -0 "$PID" 2>/dev/null; do
    sleep 0.2
    i=$((i + 1))
    [ "$i" -gt 300 ] && exit 1
done

rm -rf "$BAK"
mv "$TARGET" "$BAK" || exit 1
if ! mv "$NEW" "$TARGET"; then
    # Put it back. A machine with no vizz on it is the one outcome
    # this whole dance exists to avoid.
    mv "$BAK" "$TARGET"
    exit 1
fi
rm -rf "$BAK"
# Clear the quarantine the download carries, or the replacement asks
# for confirmation on first launch — from a bundle the user never saw
# arrive, which reads as something having gone wrong.
xattr -dr com.apple.quarantine "$TARGET" 2>/dev/null
open "$TARGET"
rm -rf {staging}
# Last, and outside the directory just removed: a script deleting itself
# after its final command is fine; one deleting the directory it is being
# read from is not.
rm -f "$0"
"#,
        pid = std::process::id(),
        target = shell_quote(&target),
        new = shell_quote(staged),
        staging = shell_quote(&staging_dir()),
    );
    std::fs::write(&script, body).with_context(|| format!("could not write {}", script.display()))?;
    // The script has to be executable, and `PermissionsExt` does not
    // exist off Unix. This module compiles on every platform on purpose
    // — `blocker` is then the single place that says "macOS only",
    // rather than every call site carrying its own cfg — so the one
    // genuinely Unix-only call is the one thing gated.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = std::fs::metadata(&script)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms)?;
    }

    // Detached, and explicitly not a child that dies with us: `setsid`
    // via `nohup`-style redirection is not enough on macOS, so the
    // process group is broken with `setsid`-equivalent behaviour by
    // spawning through `/bin/sh -c` with the job backgrounded.
    Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{} >/dev/null 2>&1 &", shell_quote(&script)))
        .spawn()
        .context("could not start the installer")?;
    log::info!("installer started; quitting to let it replace {}", target.display());
    Ok(())
}

/// Single-quote a path for `sh`, so a space or an apostrophe in
/// `/Applications` cannot turn into two arguments — or into a command.
fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paths reach the shell as one argument, whatever is in them.
    ///
    /// The script interpolates paths the user controls — an app moved to
    /// a folder with a quote in the name, a staging directory under a
    /// home directory with a space. Getting this wrong is not a cosmetic
    /// bug: it is a command injection into a script that runs `rm -rf`.
    #[test]
    fn paths_cannot_break_out_of_the_installer_script() {
        assert_eq!(shell_quote(Path::new("/Applications/vizz.app")), "'/Applications/vizz.app'");
        assert_eq!(
            shell_quote(Path::new("/Users/dj/My Apps/vizz.app")),
            "'/Users/dj/My Apps/vizz.app'"
        );
        // The dangerous one: a quote would end the quoting and everything
        // after it would be read as shell.
        assert_eq!(
            shell_quote(Path::new("/tmp/x'; rm -rf /; echo '")),
            r"'/tmp/x'\''; rm -rf /; echo '\'''"
        );
        // And whatever is quoted contains no unescaped quote left to
        // close early — the property, stated independently of the exact
        // escaping scheme.
        for nasty in ["a'b", "a\"b", "$(whoami)", "`id`", "a b; rm -rf /", "'"] {
            let quoted = shell_quote(Path::new(nasty));
            assert!(quoted.starts_with('\'') && quoted.ends_with('\''));
            let inner = &quoted[1..quoted.len() - 1];
            assert!(
                !inner.contains('\'') || inner.contains(r"'\''"),
                "{nasty:?} quoted as {quoted} leaves a bare quote"
            );
        }
    }

    /// A bare binary is not a thing that can replace itself.
    ///
    /// In the test binary `current_exe` is under `target/debug/deps`, so
    /// this exercises the real "not a bundle" path rather than a mock.
    #[test]
    fn a_non_bundle_build_declines_to_update_itself() {
        assert!(
            running_bundle().is_none(),
            "the test binary was mistaken for an app bundle"
        );
        let why = blocker().expect("a bare binary claimed it could update itself");
        assert!(!why.is_empty(), "the refusal gave no reason");
        // And it says what to do instead, rather than only that it will
        // not — a dead end in a panel is a support request.
        assert!(
            why.contains("rebuild") || why.contains("macOS"),
            "the refusal does not say what to do instead: {why}"
        );
    }
}
