//! Reports on disk, waiting.
//!
//! Two folders, and the difference between them is consent:
//!
//! - `pending/` holds crash reports nobody has agreed to send. They are
//!   written by the panic hook and by the unclean-exit check, and they
//!   leave only by the person pressing Send (or having ticked "Always
//!   send"), which moves them to the outbox, or by Don't send, which
//!   deletes them. Nothing reads this folder to send it.
//! - `outbox/` holds what the person has agreed to send — every feedback
//!   message, and crash reports once agreed — until the service takes
//!   it. It is the only folder the sender reads.
//!
//! One JSON file per report, named so they sort oldest first. At most
//! [`MAX`] in each folder; past that the oldest goes, since the newest
//! crash is the one that describes the build in use.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::payload::{CrashReport, Feedback};

/// Reports kept per folder.
pub const MAX: usize = 20;

/// Which endpoint a queued report goes to, with its body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "endpoint", content = "body", rename_all = "lowercase")]
pub enum Envelope {
    Crash(CrashReport),
    Feedback(Feedback),
}

impl Envelope {
    pub fn path(&self) -> &'static str {
        match self {
            Envelope::Crash(_) => "/api/reports/crash",
            Envelope::Feedback(_) => "/api/reports/feedback",
        }
    }
}

/// One folder of reports.
#[derive(Debug, Clone)]
pub struct Queue {
    dir: PathBuf,
}

/// A report on disk, and where.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub envelope: Envelope,
}

static SEQ: AtomicU64 = AtomicU64::new(0);

impl Queue {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Queue { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Add a report, dropping the oldest past [`MAX`]. Written to a
    /// temporary name and renamed, so a crash mid-write leaves either
    /// the whole report or nothing — never a half file the sender would
    /// choke on every launch.
    pub fn push(&self, envelope: &Envelope) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("{millis:015}-{:08}-{seq:04}.json", std::process::id());
        let path = self.dir.join(&name);
        let tmp = self.dir.join(format!(".{name}.tmp"));
        let body = serde_json::to_vec_pretty(envelope).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &path)?;
        self.trim();
        Ok(path)
    }

    /// Report files, oldest first.
    fn files(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().is_some_and(|x| x == "json")
                    && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
            })
            .collect();
        files.sort();
        files
    }

    fn trim(&self) {
        let files = self.files();
        if files.len() > MAX {
            for old in &files[..files.len() - MAX] {
                let _ = std::fs::remove_file(old);
            }
        }
    }

    /// Everything readable, oldest first. A file that will not parse is
    /// removed rather than kept: it would fail the same way every launch
    /// forever, and it holds a place one of the twenty could use.
    pub fn entries(&self) -> Vec<Entry> {
        self.files()
            .into_iter()
            .filter_map(|path| {
                let parsed = std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Envelope>(&b).ok());
                match parsed {
                    Some(envelope) => Some(Entry { path, envelope }),
                    None => {
                        log::warn!("dropping an unreadable queued report {}", path.display());
                        let _ = std::fs::remove_file(&path);
                        None
                    }
                }
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.files().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forget every report here.
    pub fn clear(&self) {
        for f in self.files() {
            let _ = std::fs::remove_file(f);
        }
    }

    /// Move every report here into `to`, letting `edit` change each on the
    /// way — how a crash picks up the note typed in the prompt. Returns how
    /// many moved.
    pub fn move_all(&self, to: &Queue, mut edit: impl FnMut(Envelope) -> Envelope) -> usize {
        let mut moved = 0;
        for entry in self.entries() {
            if to.push(&edit(entry.envelope)).is_ok() {
                let _ = std::fs::remove_file(&entry.path);
                moved += 1;
            }
        }
        moved
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::payload::{CrashKind, FeedbackKind};

    pub(crate) fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vizz-report-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    pub(crate) fn crash(summary: &str) -> Envelope {
        Envelope::Crash(CrashReport {
            product: "vizz".into(),
            version: "1.0.0".into(),
            os: "macos".into(),
            os_version: None,
            arch: None,
            install: None,
            kind: CrashKind::Panic,
            summary: summary.into(),
            detail: None,
            signature: None,
            occurred_at: None,
            note: None,
        })
    }

    #[test]
    fn a_queue_keeps_twenty_and_drops_the_oldest() {
        let dir = scratch("queue-max");
        let q = Queue::new(&dir);
        for i in 0..25 {
            q.push(&crash(&format!("crash {i}"))).unwrap();
        }
        let left = q.entries();
        assert_eq!(left.len(), MAX);
        let summaries: Vec<String> = left
            .iter()
            .map(|e| match &e.envelope {
                Envelope::Crash(c) => c.summary.clone(),
                Envelope::Feedback(_) => unreachable!(),
            })
            .collect();
        assert_eq!(summaries.first().unwrap(), "crash 5", "the oldest five should have gone");
        assert_eq!(summaries.last().unwrap(), "crash 24");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_survive_a_restart_and_garbage_is_cleared() {
        let dir = scratch("queue-restart");
        Queue::new(&dir).push(&crash("one")).unwrap();
        std::fs::write(dir.join("000000000000000-garbage.json"), b"{not json").unwrap();
        // A half-written temp file from a crash mid-push is invisible.
        std::fs::write(dir.join(".999.json.tmp"), b"{").unwrap();
        let q = Queue::new(&dir);
        assert_eq!(q.entries().len(), 1);
        assert_eq!(q.len(), 1, "the unreadable file should have been removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_envelope_names_its_endpoint() {
        let e = crash("x");
        assert_eq!(e.path(), "/api/reports/crash");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["endpoint"], "crash");
        assert_eq!(v["body"]["summary"], "x");
        let f = Envelope::Feedback(crate::payload::Feedback {
            product: "vizz".into(),
            version: "1.0.0".into(),
            os: "macos".into(),
            install: None,
            kind: FeedbackKind::Idea,
            message: "m".into(),
            email: None,
            name: None,
            licence: None,
            public: false,
        });
        assert_eq!(f.path(), "/api/reports/feedback");
    }

    #[test]
    fn moving_applies_the_edit_and_empties_the_source() {
        let dir = scratch("queue-move");
        let from = Queue::new(dir.join("pending"));
        let to = Queue::new(dir.join("outbox"));
        from.push(&crash("a")).unwrap();
        from.push(&crash("b")).unwrap();
        let moved = from.move_all(&to, |e| match e {
            Envelope::Crash(mut c) => {
                c.note = Some("was fading to black".into());
                Envelope::Crash(c)
            }
            other => other,
        });
        assert_eq!(moved, 2);
        assert!(from.is_empty());
        assert!(to.entries().iter().all(|e| matches!(&e.envelope,
            Envelope::Crash(c) if c.note.as_deref() == Some("was fading to black"))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
