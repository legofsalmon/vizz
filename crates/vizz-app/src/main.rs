mod engine;
mod headless;
mod outputs;
mod params;
mod windowed;

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

    /// Output width in pixels.
    #[arg(long, default_value_t = 1280)]
    width: u32,

    /// Output height in pixels.
    #[arg(long, default_value_t = 720)]
    height: u32,

    /// Disable the Syphon output (macOS).
    #[arg(long)]
    no_syphon: bool,

    /// Syphon server name shown to receivers.
    #[arg(long, default_value = "vizz")]
    syphon_name: String,

    /// Mark published Syphon frames as vertically flipped. Use this if
    /// the image is upside down in your receiving app.
    #[arg(long)]
    syphon_flip: bool,

    /// Load a point cloud (.ply, .xyz, .csv, .pts) into a cloud slot.
    /// Repeat to fill both loadable slots: `--cloud a.ply --cloud b.ply`.
    /// Select them with `/cloud/a` and `/cloud/b`, blend with
    /// `/cloud/morph`, and set `/shape/mode 7` to show the pair.
    #[arg(long)]
    cloud: Vec<PathBuf>,

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
        args.audio_device.clone()
    };

    let params = Arc::new(AppParams::build());

    // OSC failing to bind is degraded, not fatal: visuals still run,
    // control just isn't available on that port.
    let _osc = match vizz_osc::OscServer::spawn(
        Arc::clone(&params.registry),
        ("0.0.0.0", args.osc_port),
    ) {
        Ok(server) => Some(server),
        Err(e) => {
            log::error!("OSC bind failed on port {}: {e} — continuing without OSC", args.osc_port);
            None
        }
    };

    let output_opts = outputs::OutputOpts {
        syphon: !args.no_syphon,
        syphon_name: args.syphon_name.clone(),
        syphon_flip: args.syphon_flip,
        ndi: args.ndi,
        ndi_name: args.ndi_name.clone(),
        width: args.width,
        height: args.height,
        fps: args.fps,
    };

    if args.headless {
        headless::run(
            params,
            headless::HeadlessOpts {
                width: args.width,
                height: args.height,
                frames: args.frames,
                dump: args.dump,
                audio_device,
                clouds: args.cloud.clone(),
                report: args.report,
                outputs: output_opts,
            },
        )
    } else {
        let title = if cfg!(target_os = "macos") && output_opts.syphon {
            format!(
                "vizz {}x{} — Syphon '{}' — OSC :{}",
                args.width, args.height, output_opts.syphon_name, args.osc_port
            )
        } else {
            format!("vizz {}x{} — OSC :{}", args.width, args.height, args.osc_port)
        };
        windowed::run(
            params,
            windowed::WindowedOpts {
                width: args.width,
                height: args.height,
                show_gui: !args.no_gui,
                check_updates: !args.no_update_check,
                midi_map_path: args.midi_map.clone().unwrap_or_else(vizz_midi::default_map_path),
                title,
                audio_device,
                clouds: args.cloud.clone(),
                outputs: output_opts,
            },
        )
    }
}
