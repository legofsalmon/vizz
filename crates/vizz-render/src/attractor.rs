//! Strange attractors, integrated once on the CPU into a lookup texture.
//!
//! The obvious way to draw an attractor is to iterate the ODE in the
//! vertex shader. It does not work here. A strange attractor only appears
//! after its transient decays, which for Lorenz is a couple of time units
//! — a few hundred Euler steps. Paying that per *vertex* means paying it
//! six times per particle (two triangles), twice over while morphing
//! between two modes, every frame, forever. Measured at 120k particles
//! that was 96 ms/frame, and the picture was still just the box of
//! starting points because 110 steps had not been enough to converge.
//!
//! The shape is static, so integrate it once at startup instead and store
//! the trajectory. Lookup replaces iteration: the vertex shader does one
//! `textureLoad`, cheaper than any of the parametric shapes.
//!
//! Storing the path in trajectory order also buys the animation for free.
//! Consecutive texels are consecutive points in time, so advancing every
//! particle's index by the same amount makes the whole cloud crawl
//! *along* the attractor — the flow you actually want, rather than the
//! chaotic shimmer you get from perturbing initial conditions.

use crate::GpuContext;

/// Points per attractor. 65536 covers both manifolds densely enough that
/// the cloud reads as a surface rather than a wire.
pub const POINTS: usize = 256 * 256;
const WIDTH: u32 = 256;
/// Slots in the bank: two built-in attractors plus six loadable — files,
/// text, images and the live stream all compete for the loadable ones,
/// and with only two, loading a scan meant evicting the stream.
/// Fixed rather than dynamic because the texture is allocated once and the
/// shader indexes it by row — growing it live would mean reallocating and
/// rebuilding every bind group mid-frame. Eight slots is a 32 MB texture,
/// still far inside any real limit.
pub const SLOTS: usize = 9;

/// The slot a live video input writes into.
///
/// Video is a cloud like any other so that everything already built for
/// clouds works on it unchanged: `/cloud/a`, `/cloud/b` and
/// `/cloud/morph` select and blend it, the palette multiplies its
/// colour, and it can be morphed against a scan or an attractor. It sits
/// past the loadable range rather than inside it, because a dropped file
/// and a live feed should not be able to evict each other.
pub const VIDEO_SLOT: usize = SLOTS - 1;
/// Which slots the built-in attractors occupy.
pub const SLOT_LORENZ: usize = 0;
pub const SLOT_AIZAWA: usize = 1;

/// Discarded before recording: the run-in from an arbitrary start point to
/// the attractor itself. This is exactly the work the old shader could not
/// afford to do.
const TRANSIENT: usize = 20_000;

/// White, for procedurally generated slots: they carry no colour of their
/// own and should take the palette untinted.
const WHITE: f32 = f32::from_bits(0x00FF_FFFF);

pub struct Attractors {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// A label per slot for the UI, so a loaded cloud is identifiable.
    pub names: [String; SLOTS],
}

/// Classic Lorenz, sigma 10 / rho 28 / beta 8/3.
fn lorenz_step(p: [f64; 3], dt: f64) -> [f64; 3] {
    let [x, y, z] = p;
    let d = [
        10.0 * (y - x),
        x * (28.0 - z) - y,
        x * y - (8.0 / 3.0) * z,
    ];
    [x + d[0] * dt, y + d[1] * dt, z + d[2] * dt]
}

/// Aizawa: rounder and shell-like, with a spike through the poles. A good
/// contrast partner for Lorenz in the morph chain.
fn aizawa_step(p: [f64; 3], dt: f64) -> [f64; 3] {
    let [x, y, z] = p;
    let d = [
        (z - 0.7) * x - 3.5 * y,
        3.5 * x + (z - 0.7) * y,
        0.6 + 0.95 * z - z * z * z / 3.0 - (x * x + y * y) * (1.0 + 0.25 * z)
            + 0.1 * z * x * x * x,
    ];
    [x + d[0] * dt, y + d[1] * dt, z + d[2] * dt]
}

/// Integrate, drop the transient, then normalise into roughly the unit box
/// the parametric shapes occupy so `/particles/spread` means the same
/// thing across every mode.
///
/// `f64` throughout: the attractors are chaotic by definition, and while
/// f32 would give a perfectly good-looking curve, the doubled precision
/// costs nothing at startup and keeps the trajectory on the true manifold
/// for the full 65k points instead of drifting off it.
fn trace(step: fn([f64; 3], f64) -> [f64; 3], start: [f64; 3], dt: f64) -> Vec<[f32; 4]> {
    let mut p = start;
    for _ in 0..TRANSIENT {
        p = step(p, dt);
    }

    let mut pts = Vec::with_capacity(POINTS);
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for _ in 0..POINTS {
        p = step(p, dt);
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
        pts.push(p);
    }

    // Uniform scale, not per-axis: squashing each axis to the same extent
    // would distort the shape into something that is no longer the
    // attractor.
    let centre = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ];
    let extent = (0..3).fold(0.0f64, |m, i| m.max(hi[i] - lo[i]));
    let scale = if extent > 0.0 { 2.0 / extent } else { 1.0 };

    pts.into_iter()
        .map(|p| {
            // Y-up: the attractors are defined with z as the axis of
            // symmetry, and our camera expects height in y.
            [
                ((p[0] - centre[0]) * scale) as f32,
                ((p[2] - centre[2]) * scale) as f32,
                ((p[1] - centre[1]) * scale) as f32,
                // White, not zero: the w channel carries packed colour,
                // and an unpacked 0.0 is RGB(0,0,0) — a procedural slot
                // would multiply the palette by black and vanish.
                WHITE,
            ]
        })
        .collect()
}

impl Attractors {
    pub fn new(ctx: &GpuContext) -> Self {
        let mut data = trace(lorenz_step, [0.1, 0.0, 20.0], 0.005);
        data.extend(trace(aizawa_step, [0.1, 0.0, 0.0], 0.01));
        // Loadable slots start empty — a slot with nothing in it collapses
        // to the origin, which reads as "no cloud here" rather than as a
        // broken shape.
        data.resize(POINTS * SLOTS, [0.0, 0.0, 0.0, WHITE]);

        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("attractors"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: (POINTS * SLOTS) as u32 / WIDTH,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Rgba32Float is not filterable without an optional feature,
            // but `textureLoad` never filters, so this is portable.
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        ctx.queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 16),
                rows_per_image: None,
            },
            texture.size(),
        );

        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            names: std::array::from_fn(|i| match i {
                0 => "Lorenz".into(),
                1 => "Aizawa".into(),
                _ => "(empty)".into(),
            }),
        }
    }

    /// Replace a slot with points read from a file.
    ///
    /// Clouds have whatever point count the scanner produced; the slot has
    /// a fixed size, so the cloud is resampled to fill it. Fewer points than
    /// the slot means each is repeated with a small jitter rather than
    /// leaving the tail at the origin — a dense clump at the centre is far
    /// more visually wrong than slight duplication.
    pub fn load_slot(
        &mut self,
        ctx: &GpuContext,
        slot: usize,
        points: &[crate::pointcloud::Point],
        name: &str,
    ) {
        if slot >= SLOTS || points.is_empty() {
            return;
        }
        let mut data = Vec::with_capacity(POINTS);
        // Deterministic jitter, so reloading the same file gives the same
        // cloud rather than a subtly different one each run.
        let mut seed = 0x9E37_79B9u32;
        let mut rng = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
        };
        for i in 0..POINTS {
            let p = &points[i % points.len()];
            // Only jitter the repeats; the first pass through the cloud is
            // exact, so a cloud that already fills the slot is untouched.
            let j = if i < points.len() { 0.0 } else { 0.004 };
            data.push([
                p.pos[0] + rng() * j,
                p.pos[1] + rng() * j,
                p.pos[2] + rng() * j,
                pack_color(p.color),
            ]);
        }

        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: (slot * POINTS) as u32 / WIDTH,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 16),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: POINTS as u32 / WIDTH,
                depth_or_array_layers: 1,
            },
        );
        self.names[slot] = name.to_string();
    }
}

/// Colour packed into the unused w channel: 8 bits per channel in a u32,
/// bitcast to f32. Costs nothing over carrying positions alone, and an
/// imported scan without its colour is much less recognisable.
fn pack_color(c: [u8; 3]) -> f32 {
    let bits = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
    f32::from_bits(bits)
}



#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the transient discard: every recorded point should sit
    /// on the manifold, well inside the normalised box, with none of the
    /// long run-in streak from the arbitrary start point.
    #[test]
    fn traced_points_are_bounded_and_centred() {
        for (step, start, dt) in [
            (lorenz_step as fn([f64; 3], f64) -> [f64; 3], [0.1, 0.0, 20.0], 0.005),
            (aizawa_step as fn([f64; 3], f64) -> [f64; 3], [0.1, 0.0, 0.0], 0.01),
        ] {
            let pts = trace(step, start, dt);
            assert_eq!(pts.len(), POINTS);
            for p in &pts {
                assert!(p[0].is_finite() && p[1].is_finite() && p[2].is_finite());
                // Normalisation puts the widest axis in [-1, 1]; allow a
                // hair over for the others.
                assert!(p[0].abs() <= 1.01 && p[1].abs() <= 1.01 && p[2].abs() <= 1.01);
            }
            // A collapsed or diverged trace would fail this: the cloud has
            // to actually occupy its box.
            let spread = pts.iter().map(|p| p[1].abs()).fold(0.0f32, f32::max);
            assert!(spread > 0.3, "trace occupies too little of its box: {spread}");
        }
    }

    /// The w channel carries packed colour. A procedural slot must store
    /// white, because the shader multiplies the palette by it — zero there
    /// unpacks to RGB(0,0,0) and the whole attractor renders black, which
    /// is exactly what happened when the colour channel was added.
    #[test]
    fn procedural_slots_store_white_not_zero() {
        let pts = trace(lorenz_step, [0.1, 0.0, 20.0], 0.005);
        assert!(
            pts.iter().all(|p| p[3] == WHITE),
            "a procedural slot stored a non-white colour, which renders black"
        );
        // And WHITE must actually unpack to full white, the way the shader
        // reads it.
        let bits = WHITE.to_bits();
        assert_eq!(
            [(bits >> 16) & 255, (bits >> 8) & 255, bits & 255],
            [255, 255, 255]
        );
    }

    /// Loaded colours must survive the pack/unpack round trip the shader
    /// performs, or an imported scan comes back the wrong colour.
    #[test]
    fn packed_colour_round_trips() {
        for c in [[0u8, 0, 0], [255, 255, 255], [10, 20, 30], [200, 5, 128]] {
            let bits = pack_color(c).to_bits();
            let back = [
                ((bits >> 16) & 255) as u8,
                ((bits >> 8) & 255) as u8,
                (bits & 255) as u8,
            ];
            assert_eq!(back, c, "colour {c:?} did not survive packing");
        }
    }

    /// Consecutive texels must be consecutive points in time — that is what
    /// makes an index offset read as flow along the trajectory rather than
    /// as noise.
    #[test]
    fn consecutive_points_are_adjacent() {
        let pts = trace(lorenz_step, [0.1, 0.0, 20.0], 0.005);
        let hops: Vec<f32> = pts
            .windows(2)
            .map(|w| {
                let d = [w[1][0] - w[0][0], w[1][1] - w[0][1], w[1][2] - w[0][2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            })
            .collect();
        let max_hop = hops.iter().copied().fold(0.0f32, f32::max);
        let mean_hop = hops.iter().sum::<f32>() / hops.len() as f32;

        // The mean is the discriminating figure. Points in trajectory order
        // step by roughly `dt * speed`; any shuffling of the ordering would
        // put the mean up near the size of the box (2.0), two orders of
        // magnitude away, so this is not a close call.
        assert!(mean_hop < 0.02, "points are not in time order: mean hop {mean_hop}");
        // The max is much larger than the mean because Lorenz genuinely
        // moves fastest crossing between the wings, and a fixed timestep
        // therefore samples those stretches more sparsely. That is correct
        // — the attractor really is dimmer there — so this bound only has
        // to catch a discontinuity, not enforce even spacing.
        assert!(max_hop < 0.2, "trajectory jumps between samples: {max_hop}");
    }
}

#[cfg(test)]
mod slot_sync_tests {
    /// The shader carries its own copy of the slot count, and it folds
    /// anything past it back onto a lower slot. When the bank grew from
    /// four slots to eight, this constant did not: every cloud from the
    /// third loadable slot on quietly showed a different one, with
    /// nothing failing and nothing to see but the wrong picture.
    ///
    /// WGSL cannot read a Rust constant, so the copy is unavoidable. What
    /// is avoidable is the copy going stale, which is what this reads the
    /// shader source to prevent.
    #[test]
    fn the_shader_agrees_with_rust_about_the_slot_bank() {
        let src = include_str!("shaders/particles.wgsl");
        let read = |name: &str| -> usize {
            let needle = format!("const {name}: u32 = ");
            let at = src
                .find(&needle)
                .unwrap_or_else(|| panic!("{name} is not declared in particles.wgsl"));
            let rest = &src[at + needle.len()..];
            let end = rest.find('u').expect("a u32 literal");
            rest[..end].trim().parse().expect("a number")
        };
        assert_eq!(
            read("CLOUD_SLOTS"),
            super::SLOTS,
            "the shader would fold the top slots onto lower ones"
        );
        assert_eq!(
            read("VIDEO_SLOT"),
            super::VIDEO_SLOT,
            "the shader would look for live video in the wrong slot"
        );
    }
}
