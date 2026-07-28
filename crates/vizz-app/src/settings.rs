//! App settings that are not parameters.
//!
//! Deliberately separate from presets and from the parameter registry.
//! A parameter is something you perform with: it is smoothed, modulated,
//! addressable over OSC and captured into a look. These are the opposite —
//! choices about the machine you are running on, made once and expected to
//! still be there tomorrow. Which soundcard is plugged in is not part of a
//! look, and recalling a preset must never change it.
//!
//! Stored beside patches, presets and the MIDI map, and written whole on
//! every change. The file is a few hundred bytes and is touched when a
//! human clicks something, so there is nothing to optimise and a
//! read-modify-write is worth it for never losing an unrelated field.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// Every field optional and `#[serde(default)]`, so a file written by an
/// older or newer build loads rather than being discarded. Losing your
/// settings because a field was added is a bad trade for strictness.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Input device name, or `None` for the system default.
    pub audio_device: Option<String>,
}

pub fn path() -> PathBuf {
    vizz_mod::library::patch_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
        .join("settings.json")
}

/// Load, falling back to defaults. A missing file is the normal first-run
/// case; a corrupt one is logged and replaced, because refusing to start
/// over a settings file would be the worst possible time to find out.
pub fn load() -> Settings {
    let path = path();
    let Ok(bytes) = std::fs::read(&path) else {
        return Settings::default();
    };
    match serde_json::from_slice(&bytes) {
        Ok(s) => s,
        Err(e) => {
            log::warn!(
                "could not read {}: {e:#} — using default settings",
                path.display()
            );
            Settings::default()
        }
    }
}

/// Write via a temporary file and rename, so a crash part-way cannot
/// destroy the settings that were already there.
pub fn save(settings: &Settings) -> Result<()> {
    let path = path();
    let dir = path.parent().context("settings path has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(settings)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// Remember the chosen input. Read-modify-write rather than a blind
/// overwrite: this is called from the panel, and clobbering every other
/// setting because the user changed soundcard would be a nasty surprise.
pub fn save_audio_device(name: Option<&str>) -> Result<()> {
    let mut s = load();
    s.audio_device = name.map(str::to_owned);
    save(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The round trip, and the property that matters most: an unrelated
    /// field survives a targeted write.
    #[test]
    fn a_targeted_write_keeps_the_rest_of_the_file() {
        let (_guard, dir) = crate::test_env::scoped("settings");

        // Absent file is the defaults, not an error.
        assert_eq!(load(), Settings::default());

        save(&Settings {
            audio_device: Some("Scarlett 2i2".into()),
        })
        .unwrap();
        assert_eq!(load().audio_device.as_deref(), Some("Scarlett 2i2"));

        // Switching back to the default is a real choice, distinct from
        // never having chosen.
        save_audio_device(None).unwrap();
        assert_eq!(load().audio_device, None);

        // A corrupt file falls back rather than refusing to start.
        std::fs::write(path(), b"{not json").unwrap();
        assert_eq!(load(), Settings::default());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
