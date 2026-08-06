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
    /// Point clouds loaded into the loadable slots, in slot order.
    ///
    /// Paths rather than the point data: a scan is megabytes and the file
    /// is the thing the user actually owns. The cost is that moving or
    /// deleting the file empties the slot on next start, which is the
    /// honest outcome — the alternative is a copy that silently stops
    /// matching the file it came from.
    pub clouds: Vec<String>,
    /// What receivers get, in pixels. `None` keeps whatever the command
    /// line asked for, so a scripted venue is not overridden by a choice
    /// made on a laptop last week.
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

/// Remember which cloud is in which slot. Same read-modify-write reason as
/// the audio device: this is called from a drop handler, and dropping a
/// scan onto the window should not forget your soundcard.
pub fn save_clouds(clouds: &[String]) -> Result<()> {
    let mut s = load();
    s.clouds = clouds.to_vec();
    save(&s)
}

/// Remember the loaded palettes, same read-modify-write reason.
pub fn save_palettes(palettes: &[String]) -> Result<()> {
    let mut s = load();
    s.palettes = palettes.to_vec();
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
