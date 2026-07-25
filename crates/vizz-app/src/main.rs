mod engine;
mod headless;
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
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

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

    if args.headless {
        headless::run(
            params,
            headless::HeadlessOpts {
                width: args.width,
                height: args.height,
                frames: args.frames,
                dump: args.dump,
                report: args.report,
            },
        )
    } else {
        windowed::run(
            params,
            windowed::WindowedOpts {
                width: args.width,
                height: args.height,
            },
        )
    }
}
