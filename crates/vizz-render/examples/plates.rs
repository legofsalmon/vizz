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
//! Each plate is drawn by the live renderer: the cloud loaded into a slot
//! and shown the way the app shows a loaded cloud, through the particle
//! shader and the graded post chain, with the exposure metered for each
//! plate. Simulations are stepped for a couple of seconds first, with no
//! audio, so what is drawn is the thing settled into its own behaviour.
//!
//! These used to come from a CPU splatter that drew one grey pixel per
//! point, which was never what the app shows. The 2026-09-23 previz
//! review found it read *better* than the live renderer, and the reason
//! was the meter: the splatter set its exposure from each plate's bright
//! end, and the live renderer clipped. With the graded path metering the
//! same way, the page can show the real thing.

use std::path::PathBuf;

use vizz_render::camera::Camera;
use vizz_render::particles::{ParticleScene, Uniforms};
use vizz_render::pointcloud::Point;
use vizz_render::post::{PostChain, PostUniforms, SCENE_FORMAT};
use vizz_render::{GpuContext, generate, output, simulate};

/// Twice the size the page shows, so it is sharp on a dense screen.
const WIDTH: u32 = 900;
const HEIGHT: u32 = 600;
/// Rendered at 2× and filtered down, as the app renders a 1080p output.
const SCALE: u32 = 2;

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
    let mut rig = Rig::new();
    let wanted = |id: &str| only.as_ref().is_none_or(|o| o == id);

    for id in generate::IDS {
        if !wanted(id) {
            continue;
        }
        let start = std::time::Instant::now();
        let points = generate::generate(id).expect("a generator in the list makes a cloud");
        let path = out.join(format!("{id}.png"));
        write(&path, &rig.draw(&points, id));
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
        write(&path, &rig.draw(&points, id));
        println!("sim-{id}: {:.2}s", start.elapsed().as_secs_f64());
    }
}

/// The renderer, set up once and reused for every plate.
struct Rig {
    ctx: GpuContext,
    scene: ParticleScene,
    target: output::OutputTarget,
}

impl Rig {
    fn new() -> Self {
        let ctx = pollster::block_on(GpuContext::new(None)).expect("no GPU adapter");
        let scene = ParticleScene::new(&ctx, SCENE_FORMAT);
        let target = output::OutputTarget::new(&ctx.device, WIDTH, HEIGHT);
        Self { ctx, scene, target }
    }

    /// Show `points` the way the app shows a loaded cloud and return the
    /// frame, RGB.
    fn draw(&mut self, points: &[Point], name: &str) -> Vec<u8> {
        let slot = ParticleScene::loadable_slot(0).expect("a loadable slot");
        self.scene.set_cloud(&self.ctx, slot, points, name);
        // A fresh chain per plate, so one plate's meter does not ease in
        // from the last one's.
        let mut post = PostChain::new(&self.ctx, WIDTH * SCALE, HEIGHT * SCALE, output::OUTPUT_FORMAT);

        // Turned a little off the front and tilted down a little, as the
        // old plates were, and close enough that the cloud fills the frame.
        let camera = Camera {
            orbit: 0.6,
            elevation: 0.42,
            distance: 2.8,
            aspect: WIDTH as f32 / HEIGHT as f32,
            ..Default::default()
        };
        let cam = camera.uniforms();
        let uniforms = Uniforms {
            view_proj: cam.view_proj,
            cam_right: cam.right,
            focus: camera.distance,
            cam_up: cam.up,
            defocus: 0.0,
            cam_position: cam.position,
            viewport_h: 0.0,
            // Held at zero: no spin, so every plate is the same view.
            time: 0.0,
            aspect: camera.aspect,
            // Fine: most of these are structure at the scale of a point,
            // and the footprint floor keeps them from aliasing.
            size: 0.0018,
            spread: 1.0,
            hue: 0.55,
            saturation: 0.7,
            brightness: 1.0,
            // The cloud pair, with the plate's cloud as A.
            shape: 7.0,
            morph: 0.0,
            twist: 0.0,
            palette: 1.0,
            // Colour by height, so a shape reads as one solid thing
            // rather than a field of unrelated dots. A simulation that
            // shades its own points multiplies into this.
            color_spread: 0.8,
            color_drive: 3.0,
            cloud_a: slot as f32,
            cloud_b: slot as f32,
            cloud_morph: 0.0,
            room: Default::default(),
            lamp: Uniforms::UNLIT.lamp,
            lamp_tint: Uniforms::UNLIT.lamp_tint,
            light: Uniforms::UNLIT.light,
            sun_dir: Uniforms::UNLIT.sun_dir,
            sun_tint: Uniforms::UNLIT.sun_tint,
            gravity: Default::default(),
            gravity_radius: Default::default(),
            gravity_amount: Default::default(),
            palette_rows: [4.0, 0.0, 0.0, 0.0],
            video: [0.0, 1.0, 0.0, 0.0],
        };
        let post_uniforms = PostUniforms {
            zoom: 1.0,
            glow: 0.15,
            aspect: camera.aspect,
            grade: 1.0,
            adapt: 1.0,
            // Metered per plate, both ways. The live app only lets the
            // meter darken, so a fade still reaches black; a plate is one
            // still, and a sparse fluid needs lifting as much as a dense
            // dendrite needs pulling down.
            max_gain: 64.0,
            ..Default::default()
        };
        let mut enc = self.ctx.device.create_command_encoder(&Default::default());
        self.scene.render(&self.ctx, &mut enc, &post.scene_view, &uniforms, 65_536, true,
            wgpu::Color::BLACK);
        post.render(&self.ctx, &mut enc, &self.target.view, &post_uniforms);
        self.ctx.queue.submit([enc.finish()]);
        readback(&self.ctx, &self.target.texture)
    }
}

/// The master's pixels as tightly packed RGB. The master is BGRA.
fn readback(ctx: &GpuContext, texture: &wgpu::Texture) -> Vec<u8> {
    let padded = (WIDTH * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("plate-readback"),
        size: (padded * HEIGHT) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
        },
        wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).expect("the readback");
    let data = slice.get_mapped_range().expect("the mapped readback");
    let mut rgb = Vec::with_capacity((WIDTH * HEIGHT * 3) as usize);
    for row in 0..HEIGHT as usize {
        let line = &data[row * padded as usize..][..(WIDTH * 4) as usize];
        for px in line.as_chunks::<4>().0 {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
    }
    rgb
}

fn write(path: &std::path::Path, rgb: &[u8]) {
    image::RgbImage::from_raw(WIDTH, HEIGHT, rgb.to_vec())
        .expect("the buffer is the right size")
        .save(path)
        .unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
}
