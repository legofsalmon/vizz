//! Contact sheet for the vector layer stack: the acceptance artifact for
//! the renderer before any of it is wired to parameters.
//!
//!     cargo run -p vizz-render --example render_vector -- sheet.png
//!
//! Six hand-built looks, rendered headlessly and tiled into one image.
//! This is where edges, moiré and blend colours are judged by eye — the
//! looks are chosen to exercise every generator, the kaleido fold, an
//! invert, and five of the six non-normal blend modes.

use vizz_render::vector::{LayerU, StackU, VectorScene};
use vizz_render::{GpuContext, output};

const TILE_W: u32 = 640;
const TILE_H: u32 = 360;
const COLS: u32 = 3;
const ROWS: u32 = 2;

fn layer(kind: f32) -> LayerU {
    LayerU {
        xform: [0.0, 0.0, 0.0, 1.0],
        pat: [kind, 8.0, 0.0, 0.5],
        shape: [4.0, 0.5, 0.0, 0.0],
        style: [0.0, 1.0, 0.0, 0.0],
    }
}

fn base() -> StackU {
    StackU {
        globals: [TILE_W as f32 / TILE_H as f32, 0.0, 1.0, 8.0],
        bg: [0.96, 0.94, 0.90, TILE_H as f32],
        ..StackU::default()
    }
}

/// The looks. Each is (title, stack) — titles go in the log so a bad
/// tile can be named when discussing the sheet.
fn looks() -> Vec<(&'static str, StackU)> {
    let mut all = Vec::new();

    // 1. The classic: two ring fields at near frequencies, multiplied.
    // The interference pattern is the whole point of the architecture.
    let mut s = base();
    s.layers[0] = LayerU {
        pat: [1.0, 14.0, 0.0, 0.5],
        style: [1.0, 1.0, 1.0, 0.0], // multiply, red ink
        xform: [-0.25, 0.0, 0.0, 1.0],
        ..layer(1.0)
    };
    s.layers[1] = LayerU {
        pat: [1.0, 13.2, 0.0, 0.5],
        style: [1.0, 1.0, 2.0, 0.0], // multiply, blue ink
        xform: [0.25, 0.0, 0.0, 1.0],
        ..layer(1.0)
    };
    all.push(("rings x rings multiply moire", s));

    // 2. Stripes over stripes, slightly rotated, difference blend — the
    // inverted-interference colour pop.
    let mut s = base();
    s.bg = [0.08, 0.08, 0.10, TILE_H as f32];
    s.layers[0] = LayerU {
        pat: [2.0, 18.0, 0.0, 0.5],
        style: [0.0, 1.0, 1.0, 0.0], // normal, red
        ..layer(2.0)
    };
    s.layers[1] = LayerU {
        pat: [2.0, 17.5, 0.0, 0.5],
        xform: [0.0, 0.0, 0.015, 1.0],
        style: [4.0, 1.0, 2.0, 0.0], // difference, blue
        ..layer(2.0)
    };
    all.push(("stripes difference pop", s));

    // 3. Checker under a big star, exclusion — hard geometry over a
    // regular field, the screen-print poster look.
    let mut s = base();
    s.layers[0] = LayerU {
        pat: [3.0, 6.0, 0.0, 0.5],
        style: [0.0, 1.0, 3.0, 0.0], // normal, yellow
        ..layer(3.0)
    };
    s.layers[1] = LayerU {
        pat: [5.0, 8.0, 0.0, 0.5],
        shape: [5.0, 0.45, 0.0, 0.0],
        xform: [0.0, 0.0, 0.05, 1.3],
        style: [5.0, 1.0, 2.0, 0.0], // exclusion, blue
    };
    all.push(("checker x star exclusion", s));

    // 4. Rays through a dot grid, screen blend, hexagon punched out by
    // an inverted polygon on top.
    let mut s = base();
    s.bg = [0.06, 0.06, 0.08, TILE_H as f32];
    s.layers[0] = LayerU {
        pat: [7.0, 9.0, 0.0, 0.6],
        style: [2.0, 1.0, 2.0, 0.0], // screen, blue dots
        ..layer(7.0)
    };
    s.layers[1] = LayerU {
        pat: [6.0, 24.0, 0.0, 0.35],
        style: [2.0, 1.0, 1.0, 0.0], // screen, red rays
        ..layer(6.0)
    };
    s.layers[2] = LayerU {
        pat: [4.0, 8.0, 0.0, 0.5],
        shape: [6.0, 0.5, 0.0, 1.0], // hexagon, inverted fill
        xform: [0.0, 0.0, 0.0, 0.9],
        style: [1.0, 1.0, 0.0, 0.0], // multiply near-black outside the hex
    };
    all.push(("dots x rays screen, hex mask", s));

    // 5. Kaleido-folded rings, off-centre, over stripes — the fold's
    // wedge seams are the AA torture test.
    let mut s = base();
    s.layers[0] = LayerU {
        pat: [2.0, 10.0, 0.25, 0.5],
        xform: [0.0, 0.0, 0.125, 1.0],
        style: [0.0, 1.0, 3.0, 0.0], // normal, yellow
        ..layer(2.0)
    };
    s.layers[1] = LayerU {
        pat: [1.0, 16.0, 0.0, 0.4],
        xform: [0.35, 0.1, 0.0, 1.0],
        shape: [4.0, 0.5, 6.0, 0.0], // 6-wedge fold
        style: [1.0, 1.0, 0.0, 0.0], // multiply, black
    };
    all.push(("kaleido rings over stripes", s));

    // 6. The blend-space proof: mid-grey polygon multiplied over
    // mid-grey paper must land at the ENCODED product (~byte 64), the
    // print behaviour the design chose. Eyeball and eyedropper check.
    let mut s = base();
    s.bg = [0.5, 0.5, 0.5, TILE_H as f32];
    s.palette[0] = [0.5, 0.5, 0.5, 1.0];
    s.layers[0] = LayerU {
        pat: [4.0, 8.0, 0.0, 0.5],
        shape: [4.0, 0.5, 0.0, 0.0],
        style: [1.0, 1.0, 0.0, 0.0], // multiply, mid-grey on mid-grey
        ..layer(4.0)
    };
    all.push(("grey multiply grey = encoded product", s));

    all
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "vector-sheet.png".into());
    let ctx = pollster::block_on(GpuContext::new(None)).expect("no GPU adapter");
    let scene = VectorScene::new(&ctx, output::OUTPUT_FORMAT);
    let target = output::OutputTarget::new(&ctx.device, TILE_W, TILE_H);

    let mut sheet = image::RgbaImage::new(TILE_W * COLS, TILE_H * ROWS);
    for (i, (title, stack)) in looks().into_iter().enumerate() {
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        scene.render(&ctx, &mut enc, &target.view, &stack);
        ctx.queue.submit([enc.finish()]);
        let pixels = read_back(&ctx, &target.texture);
        let (cx, cy) = (i as u32 % COLS * TILE_W, i as u32 / COLS * TILE_H);
        for y in 0..TILE_H {
            for x in 0..TILE_W {
                let o = ((y * TILE_W + x) * 4) as usize;
                // BGRA from the master format; swap for the image crate.
                let px = image::Rgba([pixels[o + 2], pixels[o + 1], pixels[o], 255]);
                sheet.put_pixel(cx + x, cy + y, px);
            }
        }
        println!("tile {}: {title}", i + 1);
    }
    sheet.save(&path).expect("png write failed");
    println!("wrote {path}");
}

fn read_back(ctx: &GpuContext, texture: &wgpu::Texture) -> Vec<u8> {
    let unpadded = TILE_W * 4;
    let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * TILE_H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
        wgpu::Extent3d {
            width: TILE_W,
            height: TILE_H,
            depth_or_array_layers: 1,
        },
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
    let mut out = Vec::with_capacity((TILE_W * TILE_H * 4) as usize);
    for row in 0..TILE_H as usize {
        let start = row * padded as usize;
        out.extend_from_slice(&data[start..start + unpadded as usize]);
    }
    drop(data);
    buffer.unmap();
    out
}
