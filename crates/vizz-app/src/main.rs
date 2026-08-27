mod engine;
mod headless;
mod outputs;
mod params;
mod settings;
mod textcloud;
mod thumbshot;
mod videoin;
mod windowed;

#[cfg(test)]
pub(crate) mod test_env {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    /// Point the config directory at a scratch path for one test.
    ///
    /// Serialised behind a mutex because it mutates process-wide
    /// environment: two tests redirecting `XDG_CONFIG_HOME` at once would
    /// read each other's files and fail in a way that looks like a bug in
    /// the code under test.
    pub fn scoped(tag: &str) -> (MutexGuard<'static, ()>, std::path::PathBuf) {
        let guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("vizz-app-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the mutex makes this the only thread touching the
        // environment for as long as the guard is held.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        (guard, dir)
    }
}

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

use params::AppParams;

/// vizz — realtime generative visuals for VJing.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Render offscreen (no window); used for benchmarking and CI.
    #[arg(long)]
    headless: bool,

    /// Number of frames to render in headless mode.
    #[arg(long, default_value_t = 600)]
    frames: u32,

    /// Write the last headless frame as a PNG (headless only).
    #[arg(long)]
    dump: Option<PathBuf>,

    /// Write a JSON health/benchmark report at exit (headless only).
    #[arg(long)]
    report: Option<PathBuf>,

    /// UDP port for OSC input.
    #[arg(long, default_value_t = 7000)]
    osc_port: u16,

    /// Address OSC listens on. The default accepts control from any
    /// machine on the network, which is what a tablet running TouchOSC
    /// needs — and also means anyone on venue wifi can drive the show.
    /// Use 127.0.0.1 to accept only this machine.
    #[arg(long, default_value = "0.0.0.0")]
    osc_bind: String,

    /// Output width in pixels. Given explicitly, it wins over the size
    /// remembered from the panel; omitted, the remembered size (or 1280)
    /// is used.
    #[arg(long)]
    width: Option<u32>,

    /// Output height in pixels. Same precedence as --width.
    #[arg(long)]
    height: Option<u32>,

    /// Start fullscreen. Without it the last F11 choice is remembered.
    #[arg(long)]
    fullscreen: bool,

    /// Monitor index for --fullscreen (0-based). Out of range falls back
    /// to the primary.
    #[arg(long)]
    monitor: Option<usize>,

    /// Disable the Syphon output (macOS).
    #[arg(long)]
    no_syphon: bool,

    /// Syphon server name shown to receivers.
    #[arg(long, default_value = "vizz")]
    syphon_name: String,

    /// Publish Syphon frames the right way up. On by default; pass
    /// `--syphon-flip false` if your receiving app shows vizz upside
    /// down, which would mean it is correcting for the flip itself.
    ///
    /// Metal renders with the origin at the top left and Syphon's
    /// convention is OpenGL's, with the origin at the bottom, so a
    /// published Metal texture needs the flip flag set to arrive the
    /// right way up. This defaulted to off and shipped that way: every
    /// receiver showed vizz upside down, and the only remedy was a
    /// command-line flag that nobody double-clicking the app can reach.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    syphon_flip: bool,

    /// Load a point cloud (.ply, .xyz, .csv, .pts) or an image
    /// (.png, .jpg) into a cloud slot. Repeat to fill more of the six
    /// loadable slots: `--cloud a.ply --cloud b.ply`. The last one loaded
    /// is shown; `/cloud/a` and `/cloud/b` choose the morph pair and
    /// `/cloud/morph` blends between them.
    #[arg(long)]
    cloud: Vec<PathBuf>,

    /// Live point-cloud stream: `tcp://host:port`, `listen://host:port`,
    /// a bare `host:port`, or a path to a `.ply` file that is rewritten in
    /// place. Frames land in their own slot, which is shown when the
    /// first frame arrives.
    #[arg(long)]
    live_cloud: Option<String>,

    /// Live video input: `test` for the built-in pattern, or an NDI
    /// source name matched as a substring (`--list-ndi` shows names).
    /// The picture arrives as a cloud in its own slot — select it with
    /// `/cloud/a` and shape its relief with `/video/depth` and
    /// `/video/relief`.
    #[arg(long)]
    video_source: Option<String>,

    /// Audio input device to analyse, matched as a substring of the device
    /// name. Omit to use the system default; `--list-audio` shows names.
    #[arg(long)]
    audio_device: Option<String>,

    /// Disable audio capture entirely.
    #[arg(long)]
    no_audio: bool,

    /// Print available audio input devices and exit.
    #[arg(long)]
    list_audio: bool,

    /// Print the NDI sources visible on the network and exit.
    ///
    /// Discovery is asynchronous, so this waits a moment for the first
    /// announcements rather than reporting an empty network immediately.
    #[arg(long)]
    list_ndi: bool,

    /// Publish the output as an NDI source on the network. Requires the
    /// NDI runtime to be installed; logs a warning and carries on if not.
    #[arg(long)]
    ndi: bool,

    /// NDI source name shown to receivers.
    #[arg(long, default_value = "vizz")]
    ndi_name: String,

    /// Frame rate advertised to NDI receivers.
    #[arg(long, default_value_t = 60)]
    fps: u32,

    /// Start with the control panel hidden (Tab toggles it at runtime).
    #[arg(long)]
    no_gui: bool,

    /// Where MIDI mappings are stored.
    #[arg(long)]
    midi_map: Option<PathBuf>,

    /// Do not contact GitHub at startup to check for a newer release.
    #[arg(long)]
    no_update_check: bool,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    if args.list_audio {
        for name in vizz_audio::input_devices() {
            println!("{name}");
        }
        return Ok(());
    }
    if args.list_ndi {
        // Two seconds: long enough for sources on a quiet LAN to
        // announce, short enough not to feel hung.
        match vizz_io::ndi_recv::sources(2000) {
            Ok(names) if names.is_empty() => println!("no NDI sources found"),
            Ok(names) => names.iter().for_each(|n| println!("{n}")),
            // A missing runtime is the common case and its message names
            // every path tried, so print it rather than a bare error.
            Err(e) => println!("{e:#}"),
        }
        return Ok(());
    }
    // `--no-audio` is expressed as "no device can match", so the engine
    // takes its normal unavailable path rather than needing a second one.
    let audio_device = if args.no_audio {
        Some(String::from("\0none"))
    } else {
        // The flag wins when given, so a venue can be scripted; otherwise
        // fall back to whatever was last picked in the panel. Without this
        // the picker would appear to forget every restart.
        args.audio_device
            .clone()
            .or_else(|| settings::load().audio_device)
    };

    // Parsed once here so a malformed source is a clear startup error
    // rather than a warning buried in the log of a running show.
    let live_cloud = match args.live_cloud.as_deref() {
        Some(spec) => Some(spec.parse::<vizz_render::plystream::Source>()?),
        None => None,
    };

    let params = Arc::new(AppParams::build());

    // OSC failing to bind is degraded, not fatal: visuals still run,
    // control just isn't available on that port.
    // The default bind is every interface, deliberately: pointing a
    // tablet at vizz from across the stage is the whole point of OSC
    // here. But that is a choice worth being able to unmake — on venue
    // wifi it means anyone can drive the show — so the address is a flag
    // and the log says what is exposed either way.
    if args.osc_bind == "0.0.0.0" {
        log::info!(
            "OSC accepts control from ANY machine on the network (udp/{}) — \
             restrict with --osc-bind 127.0.0.1",
            args.osc_port
        );
    }
    // Shared with the render side, which mirrors the follow toggle and the
    // live deck's column origin into it. Made here rather than inside the
    // engine because the listener has to have it before the window exists,
    // and handed to the engine afterwards.
    // Made here because the listener binds before the window exists, and
    // handed to the engine afterwards. It starts inert: the engine turns
    // following on once it knows which of Resolume's columns the live page
    // covers, which it cannot until the deck book has been read.
    let columns = Arc::new(vizz_osc::ColumnSync::default());
    let _osc = match vizz_osc::OscServer::spawn(
        Arc::clone(&params.registry),
        Arc::clone(&columns),
        (args.osc_bind.as_str(), args.osc_port),
    ) {
        Ok(server) => Some(server),
        Err(e) => {
            log::error!(
                "OSC bind failed on {}:{}: {e} — continuing without OSC",
                args.osc_bind,
                args.osc_port
            );
            None
        }
    };

    // Whether a size was actually asked for, before the defaults fill in:
    // the windowed path lets an explicit request outrank the remembered
    // panel setting, and cannot tell "asked for 1280" from "defaulted to
    // 1280" once the Option is gone.
    let size_from_cli = args.width.is_some() || args.height.is_some();
    let width = args.width.unwrap_or(1280);
    let height = args.height.unwrap_or(720);

    let output_opts = outputs::OutputOpts {
        syphon: !args.no_syphon,
        syphon_name: args.syphon_name.clone(),
        syphon_flip: args.syphon_flip,
        ndi: args.ndi,
        ndi_name: args.ndi_name.clone(),
        width,
        height,
        fps: args.fps,
    };

    if args.headless {
        headless::run(
            params,
            headless::HeadlessOpts {
                width,
                height,
                frames: args.frames,
                dump: args.dump,
                audio_device,
                clouds: args.cloud.clone(),
                live_cloud: live_cloud.clone(),
                video_source: args.video_source.clone(),
                report: args.report,
                outputs: output_opts,
            },
        )
    } else {
        // No size in the base title: the window init appends the size
        // actually allocated, and baking the requested one in here gave
        // the title two resolutions — leading with the stale number, in
        // the one place a performer checks what is going out. The OSC
        // claim is real, not aspirational: a second instance whose bind
        // failed used to advertise a port the first instance owned.
        let osc = if _osc.is_some() {
            format!(" — OSC :{}", args.osc_port)
        } else {
            String::new()
        };
        let title = if cfg!(target_os = "macos") && output_opts.syphon {
            format!("vizz — Syphon '{}'{osc}", output_opts.syphon_name)
        } else {
            format!("vizz{osc}")
        };
        windowed::run(
            params,
            windowed::WindowedOpts {
                width,
                height,
                size_from_cli,
                fullscreen: args.fullscreen,
                monitor: args.monitor,
                show_gui: !args.no_gui,
                check_updates: !args.no_update_check,
                midi_map_path: args.midi_map.clone().unwrap_or_else(vizz_midi::default_map_path),
                title,
                audio_device,
                clouds: args.cloud.clone(),
                clouds_from_cli: !args.cloud.is_empty(),
                live_cloud: live_cloud.clone(),
                video_source: args.video_source.clone(),
                outputs: output_opts,
                columns,
            },
        )
    }
}
