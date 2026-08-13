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
    /// Keep the gravity sequencer on the scene sequencer's rate and
    /// curve. A property of how you play rather than of any patch, so it
    /// lives here with the other preferences.
    #[serde(default)]
    pub autopilot_lock: bool,
    /// Input device name, or `None` for the system default.
    pub audio_device: Option<String>,
    /// Point clouds loaded into the loadable slots, in slot order.
    ///
    /// Paths rather than the point data: a scan is megabytes and the file
    /// is the thing the user actually owns. The cost is that moving or
    /// deleting the file empties the slot on next start, which is the
    /// honest outcome — the alternative is a copy that silently stops
    /// matching the file it came from.
    pub clouds: Vec<String>,
    /// What receivers get, in pixels, as last set from the panel. An
    /// explicit `--width/--height` outranks it for that launch, so a
    /// scripted venue is not overridden by a choice made on a laptop
    /// last week; `None` means the panel has never chosen.
    pub output_size: Option<[u32; 2]>,
    /// Internal render resolution as a multiple of the output.
    ///
    /// Above 1 is supersampling: draw larger and let the downscale do the
    /// anti-aliasing, which is the one thing that reliably cleans up a
    /// field of one-pixel sprites. Below 1 buys frame rate on a machine
    /// that cannot hold the budget. 1.0 renders at output size.
    pub render_scale: Option<f32>,
    /// Sixteen-bit float master instead of eight-bit.
    ///
    /// Off by default and deliberately so: it doubles the master's
    /// bandwidth, and neither Syphon nor NDI can publish it without a
    /// conversion pass, so it costs on both sides for a difference only
    /// visible in slow gradients.
    pub wide_output: bool,
    /// Palette files loaded, in the order they were dropped. Restored on
    /// start so a set's colours come back with it.
    pub palettes: Vec<String>,
    /// Whether the window was fullscreen when last toggled. Restored on
    /// launch; `--fullscreen` forces it for a run regardless.
    pub fullscreen: bool,
    /// Where the beat clock takes its tempo from. `Midi` follows MIDI
    /// clock on the wire; tapping or enabling auto-BPM switches back to
    /// `Internal`, because an explicit human gesture always wins.
    pub clock_source: ClockSource,
    /// Where the modulation canvas was left: pan, zoom, patch name and
    /// whether the palette strip was open. Restored on start so the
    /// canvas opens where you were working, not at the origin with the
    /// name field blank.
    pub graph_view: Option<GraphCanvas>,
}

/// See [`Settings::clock_source`].
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClockSource {
    #[default]
    Internal,
    Midi,
}

/// Mirror of [`vizz_ui::graph_view::ViewMemory`], owned here because the
/// UI crate does not serialise anything and should not start to for this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GraphCanvas {
    pub pan: [f32; 2],
    pub zoom: f32,
    pub patch: String,
    pub palette: bool,
}

impl Default for GraphCanvas {
    fn default() -> Self {
        Self { pan: [40.0, 40.0], zoom: 1.0, patch: String::new(), palette: true }
    }
}

impl From<vizz_ui::graph_view::ViewMemory> for GraphCanvas {
    fn from(m: vizz_ui::graph_view::ViewMemory) -> Self {
        Self { pan: m.pan, zoom: m.zoom, patch: m.patch_name, palette: m.show_palette }
    }
}

impl From<GraphCanvas> for vizz_ui::graph_view::ViewMemory {
    fn from(c: GraphCanvas) -> Self {
        Self { pan: c.pan, zoom: c.zoom, patch_name: c.patch, show_palette: c.palette }
    }
}

/// Clamps for the settings above.
///
/// Bounds rather than free numbers because these allocate GPU memory: a
/// mistyped 100000 would try to allocate forty gigabytes and take the app
/// down, at which point the setting that did it is already saved and it
/// fails again on the next launch.
pub const MIN_DIM: u32 = 160;
/// Longest side. 8192 is wgpu's default `max_texture_dimension_2d`, so
/// this is the hardware's answer rather than a taste one — and it lets 8K
/// DCI through, which 7680 did not.
pub const MAX_DIM: u32 = 8192;
pub const MIN_SCALE: f32 = 0.25;
pub const MAX_SCALE: f32 = 2.0;

/// Most pixels a target is allowed to be.
///
/// The per-dimension limits are not enough on their own, because they
/// constrain the sides and the cost is the area. 7680 × 7680 sits inside
/// both of them and is fifty-nine megapixels — with a 16-bit float master
/// and the post chain's ping-pong buffers behind it, several gigabytes of
/// GPU memory, reachable by typing a number into a box. A render scale of
/// 2× on a merely large output gets there the same way without anyone
/// typing anything unusual at all.
///
/// 8192 × 4320 is the widest format anyone actually drives, and lets every
/// real shape through: 8K UHD, 8K DCI, and the very wide short canvases a
/// multi-projector edge blend wants.
pub const MAX_PIXELS: u64 = 8192 * 4320;

/// Bring a size inside every limit at once, proportionally.
///
/// One factor, applied to both axes, rather than a clamp per axis. Clamping
/// each side separately does not merely round the size off — it changes the
/// *shape*: 7680 × 4320 at a 2× render scale clamps to 8192 × 8192, turning
/// a 16:9 canvas into a square. The aspect is what the projector was set up
/// for and what every framing decision was made against, so a size that has
/// to come down comes down whole.
pub fn fit([w, h]: [u32; 2]) -> [u32; 2] {
    let (wf, hf) = (w.max(1) as f64, h.max(1) as f64);
    let factor = (MAX_DIM as f64 / wf)
        .min(MAX_DIM as f64 / hf)
        .min((MAX_PIXELS as f64 / (wf * hf)).sqrt())
        .min(1.0);
    if factor >= 1.0 {
        return [w.max(MIN_DIM), h.max(MIN_DIM)];
    }
    let fitted = [
        ((wf * factor) as u32).max(MIN_DIM),
        ((hf * factor) as u32).max(MIN_DIM),
    ];
    log::warn!(
        "{w}x{h} ({:.0} megapixels) is past what vizz will allocate — using {}x{}",
        wf * hf / 1e6,
        fitted[0],
        fitted[1]
    );
    fitted
}

impl Settings {
    /// Output size, clamped, falling back to the caller's default.
    pub fn output_or(&self, fallback: [u32; 2]) -> [u32; 2] {
        fit(self.output_size.unwrap_or(fallback))
    }

    pub fn scale(&self) -> f32 {
        self.render_scale.unwrap_or(1.0).clamp(MIN_SCALE, MAX_SCALE)
    }

    /// Internal render size for a given output size.
    pub fn render_size(&self, output: [u32; 2]) -> [u32; 2] {
        let s = self.scale();
        // Fitted after scaling, not before and not per axis. This is the
        // easier way to blow the budget and nobody types anything unusual
        // to do it: 2× on a 4K output is sixteen times the pixels of
        // 1080p.
        fit([
            (output[0] as f32 * s) as u32,
            (output[1] as f32 * s) as u32,
        ])
    }
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
            vizz_mod::library::quarantine(&path);
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
    let tmp = vizz_mod::library::tmp_path(&path);
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

/// Remember which cloud is in which slot. Same read-modify-write reason as
/// the audio device: this is called from a drop handler, and dropping a
/// scan onto the window should not forget your soundcard.
pub fn save_clouds(clouds: &[String]) -> Result<()> {
    let mut s = load();
    s.clouds = clouds.to_vec();
    save(&s)
}

/// Remember the loaded palettes, same read-modify-write reason.
/// Where a new recording lands: a fresh timestamped directory under the
/// platform's video folder, falling back to the config directory when no
/// home exists. UTC in the name — std has no timezone database, and a
/// name that sorts correctly matters more than local wall time.
pub fn take_dir() -> PathBuf {
    let base = std::env::home_dir()
        .map(|h| {
            if cfg!(target_os = "macos") {
                h.join("Movies").join("vizz")
            } else {
                h.join("Videos").join("vizz")
            }
        })
        .unwrap_or_else(|| {
            vizz_mod::library::patch_dir()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default()
                .join("recordings")
        });
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_unix(secs);
    let (hh, mm, ss) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    base.join(format!("vizz-{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}"))
}

/// Days-since-epoch to calendar date (Howard Hinnant's civil algorithm).
fn civil_from_unix(secs: u64) -> (i64, u64, u64) {
    let z = secs as i64 / 86_400 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u64;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u64;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Persist the fullscreen choice alone.
pub fn save_fullscreen(fullscreen: bool) -> Result<()> {
    let mut s = load();
    s.fullscreen = fullscreen;
    save(&s)
}

/// Remember whether the two sequencers are locked together.
pub fn save_autopilot_lock(locked: bool) -> Result<()> {
    let mut s = load();
    s.autopilot_lock = locked;
    save(&s)
}

/// Persist the clock source alone.
pub fn save_clock_source(source: ClockSource) -> Result<()> {
    let mut s = load();
    s.clock_source = source;
    save(&s)
}

pub fn save_palettes(palettes: &[String]) -> Result<()> {
    let mut s = load();
    s.palettes = palettes.to_vec();
    save(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canvas view round-trips, and an old file without the field
    /// still loads — losing your settings because a field was added is
    /// the failure the serde defaults exist to prevent.
    #[test]
    fn the_canvas_view_survives_a_restart() {
        let (_guard, dir) = crate::test_env::scoped("settings-canvas");

        let canvas = GraphCanvas {
            pan: [-320.0, 12.5],
            zoom: 0.6,
            patch: "warehouse".into(),
            palette: false,
        };
        save(&Settings { graph_view: Some(canvas.clone()), ..Default::default() }).unwrap();
        assert_eq!(load().graph_view, Some(canvas));

        // A file from a build before the field existed.
        std::fs::write(path(), br#"{"palettes":["/p.png"]}"#).unwrap();
        let s = load();
        assert_eq!(s.graph_view, None);
        assert_eq!(s.palettes, vec!["/p.png".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The recording directory name is a real calendar date, sortable,
    /// and lands under a base that exists on every platform.
    #[test]
    fn take_dirs_are_dated_and_sortable() {
        let (y, m, d) = civil_from_unix(0);
        assert_eq!((y, m, d), (1970, 1, 1));
        // 2026-08-08 00:00:00 UTC.
        let (y, m, d) = civil_from_unix(1_786_147_200);
        assert_eq!((y, m, d), (2026, 8, 8), "civil date drifted");
        let dir = take_dir();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("vizz-20"), "take dir name: {name}");
        assert_eq!(name.len(), "vizz-YYYYMMDD-HHMMSS".len(), "take dir name: {name}");
    }

    /// Fullscreen and clock-source choices survive a restart, and a file
    /// from before the fields existed still loads.
    #[test]
    fn fullscreen_and_clock_source_round_trip() {
        let (_guard, dir) = crate::test_env::scoped("settings-fullscreen");
        save(&Settings {
            fullscreen: true,
            clock_source: ClockSource::Midi,
            ..Default::default()
        })
        .unwrap();
        let s = load();
        assert!(s.fullscreen);
        assert_eq!(s.clock_source, ClockSource::Midi);

        std::fs::write(path(), br#"{"palettes":[]}"#).unwrap();
        let s = load();
        assert!(!s.fullscreen, "an old file should default windowed");
        assert_eq!(s.clock_source, ClockSource::Internal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A typed cloud persists as its `text:` pseudo-path, exactly like a
    /// file path — the clouds list is plain strings and must stay that
    /// way for old files to load.
    #[test]
    fn text_cloud_entries_round_trip_in_the_clouds_list() {
        let (_guard, dir) = crate::test_env::scoped("settings-textcloud");
        save_clouds(&["text:VIZZ".into(), String::new(), "/scans/torso.ply".into()]).unwrap();
        assert_eq!(
            load().clouds,
            vec!["text:VIZZ".to_string(), String::new(), "/scans/torso.ply".into()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The round trip, and the property that matters most: an unrelated
    /// field survives a targeted write.
    #[test]
    fn a_targeted_write_keeps_the_rest_of_the_file() {
        let (_guard, dir) = crate::test_env::scoped("settings");

        // Absent file is the defaults, not an error.
        assert_eq!(load(), Settings::default());

        save(&Settings {
            audio_device: Some("Scarlett 2i2".into()),
            clouds: vec!["/scans/torso.ply".into()],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(load().audio_device.as_deref(), Some("Scarlett 2i2"));

        // Switching back to the default is a real choice, distinct from
        // never having chosen.
        save_audio_device(None).unwrap();
        assert_eq!(load().audio_device, None);
        // ...and the unrelated field it did not mention survived.
        assert_eq!(load().clouds, vec!["/scans/torso.ply".to_string()]);

        // A corrupt file falls back rather than refusing to start.
        std::fs::write(path(), b"{not json").unwrap();
        assert_eq!(load(), Settings::default());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sizes allocate GPU memory, so nothing may reach the renderer
    /// unclamped. A mistyped dimension that got through would fail to
    /// allocate, take the app down, and — because the setting is already
    /// saved — do it again on every launch after that.
    #[test]
    fn sizes_are_clamped_before_they_reach_the_renderer() {
        let wild = Settings {
            output_size: Some([100_000, 1]),
            render_scale: Some(50.0),
            ..Default::default()
        };
        let out = wild.output_or([1920, 1080]);
        assert_eq!(out, [MAX_DIM, MIN_DIM]);
        assert_eq!(wild.scale(), MAX_SCALE);
        let render = wild.render_size(out);
        assert!(render.iter().all(|d| (MIN_DIM..=MAX_DIM).contains(d)), "{render:?}");

        // And an untouched settings file renders at exactly the size asked
        // for, so this whole mechanism is invisible until used.
        let plain = Settings::default();
        assert_eq!(plain.output_or([1920, 1080]), [1920, 1080]);
        assert_eq!(plain.render_size([1920, 1080]), [1920, 1080]);
    }

    /// The side limits constrain the sides; the cost is the area. A square
    /// at the maximum side length is inside every per-dimension check and
    /// is fifty-nine megapixels — several gigabytes of GPU memory once the
    /// float master and the post chain's buffers are behind it, and one
    /// typed number away.
    #[test]
    fn a_size_inside_the_side_limits_can_still_be_too_many_pixels() {
        let square = Settings {
            output_size: Some([MAX_DIM, MAX_DIM]),
            ..Default::default()
        };
        let out = square.output_or([1920, 1080]);
        let pixels = out[0] as u64 * out[1] as u64;
        assert!(pixels <= MAX_PIXELS, "{out:?} is {pixels} pixels");
        // Squashed to fit would be a stranger surprise than smaller: the
        // aspect is what every framing decision was made against.
        assert_eq!(out[0], out[1], "a square output came back non-square: {out:?}");
    }

    /// The easier way to blow the budget, and nobody types anything odd to
    /// do it: 2× render scale on a 4K output.
    #[test]
    fn the_render_scale_cannot_blow_the_pixel_budget_either() {
        let hot = Settings {
            output_size: Some([7680, 4320]),
            render_scale: Some(2.0),
            ..Default::default()
        };
        let out = hot.output_or([1920, 1080]);
        let render = hot.render_size(out);
        let pixels = render[0] as u64 * render[1] as u64;
        assert!(pixels <= MAX_PIXELS, "render {render:?} is {pixels} pixels");
        // Still 16:9, so the projector gets the shape it was set up for.
        let asked = out[0] as f64 / out[1] as f64;
        let got = render[0] as f64 / render[1] as f64;
        assert!((asked - got).abs() < 0.01, "aspect changed: {asked} -> {got}");
    }

    /// Every real format has to fit, or the limit is in the wrong place.
    #[test]
    fn the_shapes_people_actually_drive_are_untouched() {
        for size in [
            [1920, 1080],
            [3840, 2160],
            [4096, 2160],
            [7680, 4320],  // 8K UHD
            [8192, 4320],  // 8K DCI
            [7680, 1080],  // four projectors, edge blended
            [1080, 1920],  // portrait
        ] {
            let s = Settings { output_size: Some(size), ..Default::default() };
            assert_eq!(s.output_or([1920, 1080]), size, "{size:?} was resized");
        }
    }
}
