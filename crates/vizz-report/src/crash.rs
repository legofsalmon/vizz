//! Noticing a crash: the panic hook, and the marker that catches the
//! crashes a hook never sees.
//!
//! A panic runs the hook, which writes a report and carries on to the
//! default handler. Plenty of ways to die run nothing at all — a segfault
//! in a GPU driver, an abort in a C library, the OS killing a hung
//! process, the power going — so the marker is the backstop: a small file
//! written at launch and removed on a clean exit. Finding one at the next
//! launch, from a process that is no longer running, means the last run
//! did not end the way it should have, whatever the reason.
//!
//! One marker per process (`running-<pid>.json`), because two instances
//! at once — one window per projector — is a real way to run vizz, and a
//! single shared marker would read the other instance as a crash.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What a marker says about the run that wrote it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Marker {
    pub pid: u32,
    pub version: String,
    /// Unix seconds.
    pub started: u64,
    /// A report was already written for this run by the panic hook, so
    /// the unclean exit it caused needs no second report of its own.
    pub reported: bool,
}

/// How the previous run ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Previous {
    /// A run did not reach a clean exit.
    pub unclean: bool,
    /// And none of the unclean ones had already reported itself.
    pub unreported: bool,
    /// The version that was running, for the report.
    pub version: Option<String>,
}

fn marker_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("running-{pid}.json"))
}

pub(crate) fn write_marker(dir: &Path, marker: &Marker) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!("could not create {}: {e}", dir.display());
        return;
    }
    let path = marker_path(dir, marker.pid);
    let tmp = dir.join(format!(".running-{}.tmp", marker.pid));
    let written = serde_json::to_vec(marker)
        .map_err(std::io::Error::other)
        .and_then(|b| std::fs::write(&tmp, b))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = written {
        log::warn!("could not write the run marker {}: {e}", path.display());
    }
}

/// Read every marker a previous run left, decide how it ended, and write
/// this run's own. `alive` answers whether a pid is a vizz that is still
/// running; its markers are left alone.
pub fn begin(dir: &Path, version: &str, now: u64, alive: &dyn Fn(u32) -> bool) -> Previous {
    let me = std::process::id();
    let mut previous = Previous::default();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pid) = name
                .strip_prefix("running-")
                .and_then(|n| n.strip_suffix(".json"))
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == me || alive(pid) {
                continue;
            }
            let marker: Marker = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            previous.unclean = true;
            if !marker.reported {
                previous.unreported = true;
            }
            if !marker.version.is_empty() {
                previous.version = Some(marker.version);
            }
            let _ = std::fs::remove_file(&path);
        }
    }
    write_marker(dir, &Marker { pid: me, version: version.to_string(), started: now, reported: false });
    previous
}

/// A clean exit: this run's marker goes.
pub fn end(dir: &Path) {
    let _ = std::fs::remove_file(marker_path(dir, std::process::id()));
}

/// Mark this run as having reported itself. Called from the panic hook.
pub(crate) fn mark_reported(dir: &Path) {
    let path = marker_path(dir, std::process::id());
    let Some(mut marker) = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Marker>(&b).ok())
    else {
        return;
    };
    if !marker.reported {
        marker.reported = true;
        write_marker(dir, &marker);
    }
}

/// Whether `pid` is a running vizz. The name check is there because pids
/// are reused: a marker left by a crash must not be kept forever because
/// some unrelated process happens to have its number now.
pub fn vizz_is_running(pid: u32) -> bool {
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    sys.process(p)
        .is_some_and(|proc_| proc_.name().to_string_lossy().to_ascii_lowercase().contains("vizz"))
}

/// Seconds to `2026-09-24T02:10:00Z`, without a date crate.
pub fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The first line of a panic's message.
pub fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "panic with a non-string payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::tests::scratch;

    #[test]
    fn a_clean_exit_leaves_nothing_to_find() {
        let dir = scratch("marker-clean");
        let first = begin(&dir, "1.0.0", 100, &|_| false);
        assert!(!first.unclean, "a first launch is not a crash");
        end(&dir);
        assert!(!begin(&dir, "1.0.0", 200, &|_| false).unclean);
        end(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_marker_from_a_dead_process_is_an_unclean_exit() {
        let dir = scratch("marker-dead");
        write_marker(&dir, &Marker { pid: 9_999_991, version: "0.9.0".into(), started: 5, reported: false });
        let prev = begin(&dir, "1.0.0", 100, &|_| false);
        assert!(prev.unclean && prev.unreported);
        assert_eq!(prev.version.as_deref(), Some("0.9.0"));
        // Asked once: the next launch does not ask again.
        end(&dir);
        assert!(!begin(&dir, "1.0.0", 200, &|_| false).unclean);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_running_instance_is_not_a_crash() {
        let dir = scratch("marker-alive");
        write_marker(&dir, &Marker { pid: 4242, version: "1.0.0".into(), started: 5, reported: false });
        let prev = begin(&dir, "1.0.0", 100, &|pid| pid == 4242);
        assert!(!prev.unclean);
        assert!(dir.join("running-4242.json").exists(), "the other instance's marker must stay");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_that_reported_its_own_panic_is_not_reported_twice() {
        let dir = scratch("marker-reported");
        write_marker(&dir, &Marker { pid: 9_999_992, version: "1.0.0".into(), started: 5, reported: true });
        let prev = begin(&dir, "1.0.0", 100, &|_| false);
        assert!(prev.unclean);
        assert!(!prev.unreported);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn this_process_is_seen_as_running_and_a_dead_pid_is_not() {
        // The test binary is called vizz_report-<hash>, so the name check
        // passes for this process.
        assert!(vizz_is_running(std::process::id()));
        assert!(!vizz_is_running(u32::MAX - 7));
    }

    #[test]
    fn timestamps_are_iso_8601() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_790_215_800), "2026-09-24T02:10:00Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
    }
}
