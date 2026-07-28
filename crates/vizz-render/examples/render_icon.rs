//! Render the app icon from the renderer itself.
//!
//! The icon is a real frame — the same particle shader, the same glow and
//! tone-map the app puts on screen — rather than a drawing of one. That is
//! worth the trouble twice over: it cannot drift away from what vizz
//! actually looks like, and regenerating it after a change to the look is
//! one command rather than an afternoon in a drawing program.
//!
//!     cargo run -p vizz-render --example render_icon -- assets/icon-1024.png
//!
//! # Designing for 16 points
//!
//! An icon has to survive being 16 points wide in a Dock or a Finder list,
//! and a particle field at 16 points is grey mush. So the frame is chosen
//! for its *silhouette*: a bright orb, centred, falling off to nothing. At
//! 16 points that reads as one glowing dot, which is distinct and
//! recognisable; at 512 it resolves into the field it actually is. A frame
//! with detail spread across it would read as noise at both.
//!
//! The rounded square, the margin and the corner radius follow Apple's
//! macOS grid, so the icon sits at the same visual weight as its
//! neighbours instead of looking slightly too big or slightly too small.

use vizz_render::camera::Camera;
use vizz_render::particles::{ParticleScene, Uniforms};
use vizz_render::post::{PostChain, PostUniforms};
use vizz_render::room::RoomUniforms;
use vizz_render::{GpuContext, output};

/// Master size. Every other size in the iconset is derived from this by
/// downsampling, which is sharper than rendering each one — small renders
/// lose the thin particles entirely.
const SIZE: u32 = 1024;

/// Apple's macOS icon grid: the art sits in a rounded square inset from
/// the canvas, so the icon has room for its own shadow and lines up with
/// every other icon in the Dock. These are the 1024-point figures.
const INSET: f32 = 100.0;
const RADIUS: f32 = 185.0;

/// Frames to render before the capture. The glow and tone-map settle
/// almost immediately, but the field animates from a cold start, and the
/// first frame has every particle still stacked at its seed position.
const WARMUP: u32 = 90;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "icon-1024.png".into());
    let ctx = pollster::block_on(GpuContext::new(None)).expect("no GPU adapter");

    let mut post = PostChain::new(&ctx, SIZE, SIZE, output::OUTPUT_FORMAT);
    let scene = ParticleScene::new(&ctx, vizz_render::post::SCENE_FORMAT);
    let target = output::OutputTarget::new(&ctx.device, SIZE, SIZE);

    let camera = Camera {
        distance: 7.2,
        orbit: 0.6,
        elevation: 0.16,
        fov: 0.9,
        aspect: 1.0,
        focus: 7.2,
        // A little depth of field: the near particles soften and the core
        // stays sharp, which is what stops the orb reading as a flat disc.
        defocus: 0.22,
        // Dead centre: an icon is cropped to a square by whatever draws it,
        // so anything off-axis loses a limb.
        pan_x: 0.0,
        pan_y: 0.0,
    };
    let cam = camera.uniforms();
    // The room is off, but its placement still has to be supplied — with
    // `embed` at zero it leaves the field exactly where it was.
    let room = RoomUniforms::for_camera(&camera, 1.5, 7.0, 0.0, 0.75, 0.35, 0.0, 0.0);

    let uniforms = Uniforms {
        view_proj: cam.view_proj,
        cam_right: cam.right,
        focus: camera.focus,
        cam_up: cam.up,
        defocus: camera.defocus,
        cam_position: cam.position,
        _pad_cam: 0.0,
        // Fixed, so the icon is the same picture every time it is built.
        time: 7.5,
        aspect: 1.0,
        size: 0.0080,
        spread: 1.15,
        hue: 0.52,
        saturation: 0.85,
        brightness: 3.2,
        // A sphere, barely twisted. The recognisable shape at any size.
        shape: 0.0,
        morph: 0.0,
        twist: 0.35,
        palette: 0.0,
        // Colour driven by radius, so the orb runs warm at the core and
        // cool at the rim — a gradient that survives being 16 points wide
        // when individual particles do not.
        color_spread: 0.12,
        color_drive: 1.0,
        cloud_a: 0.0,
        cloud_b: 1.0,
        cloud_morph: 0.0,
        room: room.placement(0.35, 0.0),
        gravity: Default::default(),
        gravity_radius: Default::default(),
        gravity_amount: Default::default(),
    };
    let post_uniforms = PostUniforms {
        trail: 0.0,
        zoom: 1.0,
        spin: 0.0,
        mirror: 0.0,
        // Generous: the bloom is what turns a cloud of dots into an object
        // with a silhouette, and the silhouette is the whole design.
        glow: 0.95,
        aspect: 1.0,
        shift: 0.12,
        _pad0: 0.0,
    };

    for _ in 0..WARMUP {
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        scene.render(&ctx, &mut enc, &post.scene_view, &uniforms, 95_000, true,
            vizz_render::particles::SCENE_CLEAR);
        post.render(&ctx, &mut enc, &target.view, &post_uniforms);
        ctx.queue.submit([enc.finish()]);
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }

    let scene_px = readback(&ctx, &target.texture);
    let icon = compose(&scene_px);
    icon.save(&path).expect("could not write the icon");
    println!("wrote {path}");

    // A strip of the sizes it will actually be seen at. An icon is judged
    // at 16 and 32 points, not at 1024, and the difference is total: the
    // master can look wonderful and go to grey mush the moment it is
    // small. Reviewing the master alone is reviewing the wrong picture.
    let strip = contact_sheet(&icon);
    let strip_path = std::path::Path::new(&path).with_file_name("icon-sizes.png");
    strip.save(&strip_path).expect("could not write the size strip");
    println!("wrote {}", strip_path.display());
}

/// Pull the rendered frame back as RGBA8.
fn readback(ctx: &GpuContext, texture: &wgpu::Texture) -> Vec<u8> {
    const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = (SIZE * 4).div_ceil(ALIGN) * ALIGN;
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("icon-readback"),
        size: (padded * SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for row in 0..SIZE as usize {
        let start = row * padded as usize;
        out.extend_from_slice(&data[start..start + (SIZE * 4) as usize]);
    }
    drop(data);
    buffer.unmap();
    out
}

/// Composite the frame into the icon: rounded square, dark ground, the
/// render added on top, everything outside the square transparent.
fn compose(scene: &[u8]) -> image::RgbaImage {
    let mut out = image::RgbaImage::new(SIZE, SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let coverage = squircle(x as f32 + 0.5, y as f32 + 0.5);
            if coverage <= 0.0 {
                continue;
            }
            // The master target is BGRA — that is what CAMetalLayer and
            // Syphon receivers want — so the channels come back swapped
            // from what an RGBA image expects. Reading them in order turns
            // the blue field brown, which is a convincing enough picture
            // to be mistaken for a colour choice rather than a bug.
            let i = ((y * SIZE + x) * 4) as usize;
            let (b, g, r) = (scene[i] as f32, scene[i + 1] as f32, scene[i + 2] as f32);
            // A ground that is not flat black: a dark blue that lifts
            // towards the top, so the icon has a body of its own on a dark
            // Dock instead of reading as a hole with an orb in it.
            let lift = 1.0 - y as f32 / SIZE as f32;
            let ground = [
                8.0 + 6.0 * lift,
                10.0 + 9.0 * lift,
                18.0 + 15.0 * lift,
            ];
            // Screen rather than add: the render is already tone-mapped,
            // and adding on top of the ground would clip the core to a
            // white blob and lose the colour that carries the identity.
            let px = |v: f32, base: f32| 255.0 - (255.0 - v) * (255.0 - base) / 255.0;
            out.put_pixel(
                x,
                y,
                image::Rgba([
                    px(r, ground[0]) as u8,
                    px(g, ground[1]) as u8,
                    px(b, ground[2]) as u8,
                    (coverage * 255.0) as u8,
                ]),
            );
        }
    }
    out
}

/// The icon at every size it is used at, laid out left to right on a dark
/// ground, so all of them can be judged in one look.
fn contact_sheet(icon: &image::RgbaImage) -> image::RgbaImage {
    const SIZES: [u32; 6] = [16, 32, 64, 128, 256, 512];
    const PAD: u32 = 12;
    let width: u32 = SIZES.iter().map(|s| s + PAD).sum::<u32>() + PAD;
    let height = SIZES.iter().max().copied().unwrap_or(0) + PAD * 2;
    let mut sheet = image::RgbaImage::from_pixel(width, height, image::Rgba([24, 25, 30, 255]));
    let mut x = PAD;
    for size in SIZES {
        let scaled =
            image::imageops::resize(icon, size, size, image::imageops::FilterType::Lanczos3);
        // Bottom-aligned, so the small ones are not lost in the middle of
        // a tall strip.
        image::imageops::overlay(&mut sheet, &scaled, x as i64, (height - PAD - size) as i64);
        x += size + PAD;
    }
    sheet
}

/// How much of this pixel is inside the rounded square, 0..1.
///
/// Antialiased over one pixel rather than a hard test, because a hard edge
/// on a 1024-point master turns into visible stair-stepping the moment it
/// is downsampled to 32.
fn squircle(x: f32, y: f32) -> f32 {
    let lo = INSET;
    let hi = SIZE as f32 - INSET;
    // Distance outside the rounded rectangle, negative inside.
    let dx = (lo + RADIUS - x).max(x - (hi - RADIUS)).max(0.0);
    let dy = (lo + RADIUS - y).max(y - (hi - RADIUS)).max(0.0);
    let corner = (dx * dx + dy * dy).sqrt() - RADIUS;
    let straight = (lo - x).max(x - hi).max(lo - y).max(y - hi);
    let d = if dx > 0.0 && dy > 0.0 { corner } else { straight };
    (0.5 - d).clamp(0.0, 1.0)
}
