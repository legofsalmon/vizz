//! What was on screen and going out, kept so a crash can put it back.
//!
//! Almost everything a set is made of already comes back on every launch:
//! the show, its pads and pages, the modulation, the MIDI map, the clouds,
//! the palettes, the audio device, the output size, fullscreen, the
//! screen that was up. Two things did not, deliberately, because an
//! ordinary launch should open at rest rather than replay last night:
//!
//! - **the look** — where every parameter was sitting, which is usually a
//!   recalled preset plus whatever was tweaked on top of it;
//! - **NDI** — a command-line flag, so a double-click relaunch after a
//!   crash came back without the network feed the room was receiving.
//!
//! After an *unclean* exit, "at rest" is the wrong answer: the projector
//! was showing something a second ago and should be again. So both are
//! written here every few seconds while they change, and read back only
//! when the last run did not end cleanly.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use vizz_mod::preset::{Kind, Preset};
use vizz_params::ParamRegistry;

/// How often the snapshot looks for a change. The same cadence as the
/// modulation autosave: a crash costs seconds, not a set.
pub const EVERY: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Recovery {
    /// Every look and gravity parameter's target. Transport, the punch
    /// gestures and the master dimmer are excluded by the preset rules,
    /// so a restore can never replay a blackout or fire a scene.
    pub look: Preset,
    /// The preset last recalled, for the notice that says what came back.
    pub recalled: Option<String>,
    /// The NDI source name, when NDI was being sent.
    pub ndi: Option<String>,
}

pub fn path() -> PathBuf {
    vizz_mod::project::root().join("recovery.json")
}

impl Recovery {
    /// What is live now.
    pub fn capture(reg: &ParamRegistry, recalled: Option<String>, ndi: Option<String>) -> Self {
        let mut look = Preset::capture_kind(reg, Kind::Look);
        look.values.extend(Preset::capture_kind(reg, Kind::Gravity).values);
        Recovery { look, recalled, ndi }
    }

    /// Put the look back. Targets, not values, so it glides in over the
    /// registry's smoothing like any recall.
    pub fn apply(&self, reg: &ParamRegistry) -> usize {
        self.look.apply(reg)
    }

    pub fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// Temp file and rename, like every other persisted artefact here.
pub fn save_bytes(bytes: &[u8]) -> Result<()> {
    let path = path();
    let dir = path.parent().context("recovery path has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = vizz_mod::library::tmp_path(&path);
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))
}

/// The last snapshot, or `None` when there is none or it will not read —
/// a recovery file is a convenience, never a reason not to start.
pub fn load() -> Option<Recovery> {
    let bytes = std::fs::read(path()).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(r) => Some(r),
        Err(e) => {
            log::warn!("could not read the recovery snapshot: {e:#} — starting at rest");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A look captured mid-set comes back whole after a crash, and the
    /// things a look must never carry — the dimmer, a punch, a recall —
    /// stay where the new launch put them.
    #[test]
    fn a_crashed_look_comes_back_and_nothing_performed_does() {
        let (_guard, dir) = crate::test_env::scoped("recovery");
        let before = crate::params::AppParams::build();
        let reg = &before.registry;
        let id = |addr: &str| before.registry.iter().find(|(_, d)| d.addr == addr).map(|(id, d)| (id, d.clone())).unwrap();
        let (size, def) = id("/particles/size");
        let (dim, dim_def) = id("/master/dim");
        let tweaked = def.min + (def.max - def.min) * 0.8;
        reg.set(size, tweaked);
        reg.set(dim, 0.0);
        let snap = Recovery::capture(reg, Some("Tunnel".into()), Some("vizz".into()));
        assert!(!snap.look.values.contains_key("/master/dim"), "the dimmer is not part of a look");
        assert!(!snap.look.values.contains_key("/preset/recall"));
        save_bytes(&snap.bytes()).unwrap();

        let after = crate::params::AppParams::build();
        let back = load().expect("the snapshot should read back");
        assert_eq!(back, snap);
        assert!(back.apply(&after.registry) > 10);
        assert!((after.registry.target(size) - tweaked).abs() < 1e-4);
        assert_eq!(after.registry.target(dim), dim_def.default, "a restore must not touch the dimmer");
        assert_eq!(back.ndi.as_deref(), Some("vizz"));

        // A corrupt file is no snapshot, not a failure to start.
        std::fs::write(path(), b"{nope").unwrap();
        assert!(load().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
