//! Draw every cloud this crate can make, as a picture.
//!
//! The catalogue on the site needs a plate per generator and per
//! simulation, and the site is served with a content-security policy
//! that allows no scripts at all — so the plates have to be images,
//! made here rather than drawn in a browser. Making them from the
//! shipping code rather than from a port of it is the point: what the
//! page shows is what the app makes.
//!
//! ```sh
//! cargo run --release --example plates -- site/img/clouds
//! ```
//!
//! Each image is the cloud seen from the same angle, orthographic,
//! with a point's depth as its brightness so the far side of a shape
//! reads as the far side. Simulations are stepped for a couple of
//! seconds first, with no audio, so what is drawn is the thing settled
//! into its own behaviour.

use std::path::PathBuf;

use vizz_render::generate;
use vizz_render::pointcloud::Point;
use vizz_render::simulate;

/// Twice the size the page shows, so it is sharp on a dense screen.
const WIDTH: u32 = 900;
const HEIGHT: u32 = 600;

/// Seconds to run a simulation before its portrait is taken. Most
/// settle into their behaviour in a couple; the ones that are *about*
/// growing need longer, and saying so here is more honest than
/// photographing them before they have done anything.
fn settle(id: &str) -> f32 {
    match id {
        "sand" => 45.0,
        // The flake and the knot are both *about* growing into their
        // shape, so a portrait taken early is a portrait of nothing.
        "crystal" => 20.0,
        "tangle" => 25.0,
        "slime" => 12.0,
        "life" => 8.0,
        "smoke" => 7.0,
        "reaction" | "swarm" => 8.0,
        _ => 2.5,
    }
}

fn main() {
    let out = PathBuf::from(
        std::env::args().nth(1).unwrap_or_else(|| "site/img/clouds".to_string()),
    );
    std::fs::create_dir_all(&out).expect("the output directory");
    let only = std::env::args().nth(2);
    let wanted = |id: &str| only.as_ref().is_none_or(|o| o == id);

    for id in generate::IDS {
        if !wanted(id) {
            continue;
        }
        let start = std::time::Instant::now();
        let points = generate::generate(id).expect("a generator in the list makes a cloud");
        let path = out.join(format!("{id}.png"));
        write(&path, &draw(&points));
        println!("{id}: {:.2}s", start.elapsed().as_secs_f64());
    }

    for id in simulate::IDS {
        if !wanted(id) {
            continue;
        }
        let start = std::time::Instant::now();
        let mut sim = simulate::start(id).expect("a simulation in the list starts");
        let drive = simulate::Drive::default();
        let frames = (settle(id) * 60.0) as usize;
        for _ in 0..frames {
            sim.step(1.0 / 60.0, &drive);
        }
        let mut points = Vec::new();
        sim.points(&mut points);
        let path = out.join(format!("sim-{id}.png"));
        write(&path, &draw(&points));
        println!("sim-{id}: {:.2}s", start.elapsed().as_secs_f64());
    }
}

/// Project a cloud and accumulate it into a greyscale image.
///
/// Points are added rather than painted, so where the cloud is dense
/// the image is bright — which is how a point cloud reads on screen,
/// and the only way a sixty-five-thousand-point surface shows its
/// shape at this size. The camera is the one the app opens with:
/// turned a little off the front, tilted down a little.
fn draw(points: &[Point]) -> Vec<u8> {
    const YAW: f32 = 0.6;
    const PITCH: f32 = 0.42;
    let (sin_yaw, cos_yaw) = YAW.sin_cos();
    let (sin_pitch, cos_pitch) = PITCH.sin_cos();
    let scale = HEIGHT as f32 * 0.42;
    let mut light = vec![0.0f32; (WIDTH * HEIGHT) as usize];
    for p in points {
        let [x, y, z] = p.pos;
        let (x1, z1) = (x * cos_yaw + z * sin_yaw, -x * sin_yaw + z * cos_yaw);
        let y1 = y * cos_pitch - z1 * sin_pitch;
        let depth = y * sin_pitch + z1 * cos_pitch;
        let px = WIDTH as f32 / 2.0 + x1 * scale;
        let py = HEIGHT as f32 / 2.0 - y1 * scale;
        if px < 0.0 || py < 0.0 || px >= WIDTH as f32 || py >= HEIGHT as f32 {
            continue;
        }
        // The cloud's own colour, dimmed with distance: a simulation
        // that shades its points is saying something with them.
        // Squared, because the shade carries the reading — the four
        // terraces of a sandpile, the hot core of a plume — and adding
        // light linearly leaves a three-to-one difference looking like
        // one grey.
        let own = f32::from(p.color[0]) / 255.0;
        let near = 0.45 + 0.55 * (depth + 1.0) * 0.5;
        light[py as usize * WIDTH as usize + px as usize] += own * own * near * 0.75;
    }
    // Exposure, per plate. A fluid's tracers spread over a sheet and a
    // dendrite's grains pile into a line; at one fixed gain the first
    // is invisible and the second is a white blob. Setting the gain
    // from the plate's own bright end is what a light meter does, and
    // it is the difference between a catalogue that can be read and
    // one that cannot.
    let mut lit: Vec<f32> = light.iter().copied().filter(|v| *v > 0.0).collect();
    lit.sort_by(f32::total_cmp);
    let bright = lit.get(lit.len().saturating_sub(1).min(lit.len() * 995 / 1000)).copied();
    let gain = 1.7 / bright.unwrap_or(1.0).max(1e-6);
    light
        .into_iter()
        .map(|v| {
            // A curve rather than a clamp, so a dense cloud keeps some
            // shape where it piles up instead of flattening to white.
            let v = 1.0 - (-v * gain).exp();
            (v.powf(0.85) * 255.0) as u8
        })
        .collect()
}

fn write(path: &std::path::Path, grey: &[u8]) {
    image::GrayImage::from_raw(WIDTH, HEIGHT, grey.to_vec())
        .expect("the buffer is the right size")
        .save(path)
        .unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
}
