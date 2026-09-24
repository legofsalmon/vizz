//! Where the key and the current token live: `licence.json` in the
//! machine's config directory (`~/.config/vizz`, beside `settings.json`).
//!
//! Not project data, on purpose. A show folder travels — it is copied to
//! another laptop, zipped for a collaborator, synced — and a licence is
//! for this machine. vizz keeps nothing in an OS keychain, and this does
//! not need one: the token is signed and useless on another machine, and
//! the key is the thing written on the customer's receipt.
//!
//! Written whole through a temporary file and a rename, like the settings,
//! so a crash mid-write cannot leave half a licence behind.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "licence.json";

/// What is on disk. Both optional: a pasted offline token may come with
/// no key the app was told about (it is read from the token's claims), and
/// a key can outlive a token the service refused to renew.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub key: Option<String>,
    pub token: Option<String>,
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Read it. Missing is the normal unlicensed case; unreadable is logged
/// and treated the same, because a licence file must never be a reason
/// the app does not start.
pub fn load(dir: &Path) -> Stored {
    let path = path(dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return Stored::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        log::warn!("could not read {}: {e} — treating this copy as unlicensed", path.display());
        Stored::default()
    })
}

pub fn save(dir: &Path, stored: &Stored) -> Result<(), String> {
    let path = path(dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(stored).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, bytes).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    // Owner-only: nothing else on the machine has any business reading the
    // key, even though it is not much use to anyone who does.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("could not save {}: {e}", path.display()))
}

/// Forget the licence on this machine. Absent already is success.
pub fn forget(dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(path(dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove the licence file: {e}")),
    }
}

#[cfg(test)]
pub(crate) fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vizz-licence-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_forgets() {
        let dir = scratch("store");
        assert_eq!(load(&dir), Stored::default(), "nothing there is unlicensed, not an error");
        let s = Stored { key: Some("LT-V1ZZ-K7M2-9PQR-4XTC".into()), token: Some("a.b".into()) };
        save(&dir, &s).unwrap();
        assert_eq!(load(&dir), s);
        forget(&dir).unwrap();
        assert_eq!(load(&dir), Stored::default());
        forget(&dir).expect("forgetting twice is fine");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_reads_as_unlicensed_rather_than_failing() {
        let dir = scratch("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path(&dir), b"{ not json").unwrap();
        assert_eq!(load(&dir), Stored::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_can_read_it() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("perms");
        save(&dir, &Stored { key: Some("k".into()), token: None }).unwrap();
        let mode = std::fs::metadata(path(&dir)).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "readable by others: {mode:o}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
