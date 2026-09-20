//! Clouds that move on their own: simulations run on a thread and
//! streamed into the live slot, frame by frame, exactly as a network
//! stream is.
//!
//! A generator (`generate.rs`) is made once; a simulation is made sixty
//! times a second. It reaches the screen through [`crate::plystream`],
//! which already solves everything a per-frame source needs — a thread,
//! a slot the renderer takes with `try_lock`, a revision so an unchanged
//! frame costs nothing, reconnection as the normal path — so a
//! simulation is one more [`crate::plystream::Source`] and not a second
//! streaming stack.
//!
//! What it gets back from the app is a [`Drive`]: the four audio bands,
//! the loudness and where the bar is. That is the whole interface, and
//! it is deliberately narrow: a simulation that reads parameters would
//! be a second engine, and this is meant to be a *cloud* — chosen,
//! crossed to, captured and lit like the others — that happens to be
//! alive.
//!
//! Two ship. **Fluid** is Stam's stable solver for the incompressible
//! Navier–Stokes equations (Stam, "Stable Fluids", 1999; "Real-Time
//! Fluid Dynamics for Games", 2003), on a periodic grid with Fedkiw's
//! vorticity confinement to keep the swirls alive, carrying tracer
//! particles that are the cloud. **Reaction** is the Gray–Scott
//! reaction–diffusion system in Pearson's parameterisation, one cell per
//! point, standing up as a relief.

use crate::attractor::POINTS;
use crate::pointcloud::Point;

/// What the app tells a simulation each frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drive {
    /// Post-gain band envelopes, 0..1 — what modulation sees.
    pub bands: [f32; 4],
    /// Broadband loudness, 0..1.
    pub level: f32,
    /// Where the bar is, 0..1.
    pub bar: f32,
    /// Whether an audio input is connected at all. Without one a
    /// simulation drives itself, so a rig with no interface still gets
    /// a picture that moves.
    pub audio: bool,
}

impl Default for Drive {
    fn default() -> Self {
        Self { bands: [0.0; 4], level: 0.0, bar: 0.0, audio: false }
    }
}

/// A cloud that steps.
pub trait Simulation: Send {
    /// Advance by `dt` seconds under `drive`.
    fn step(&mut self, dt: f32, drive: &Drive);
    /// The current cloud, exactly [`POINTS`] points inside the unit box.
    /// `out` is reused between frames.
    fn points(&self, out: &mut Vec<Point>);
}

/// Every simulation this crate can run, by the id `sim:<id>` names it.
/// The catalogue the panel lists is vizz-mod's; a test in vizz-app holds
/// the two to each other.
pub const IDS: &[&str] = &["fluid", "reaction"];

/// Start the simulation `id` names, or `None` for one this crate does
/// not know.
pub fn start(id: &str) -> Option<Box<dyn Simulation>> {
    match id {
        "fluid" => Some(Box::new(Fluid::new())),
        "reaction" => Some(Box::new(Reaction::new())),
        _ => None,
    }
}

// --- Fluid ------------------------------------------------------------

/// Cells along each side of the fluid grid. A power of two, so the
/// periodic wrap is a mask rather than a modulo, in the inner loops.
const N: usize = 128;
const MASK: usize = N - 1;

/// Stam's stable fluid, periodic, with tracers.
///
/// Velocity lives on the grid; the cloud is the tracers riding it. The
/// grid is a torus — no walls — so the sheet is endless and a tracer
/// that leaves on one edge arrives on the other, which is what keeps the
/// cloud uniformly filled without the re-seeding that pops.
pub struct Fluid {
    u: Vec<f32>,
    v: Vec<f32>,
    u0: Vec<f32>,
    v0: Vec<f32>,
    p: Vec<f32>,
    div: Vec<f32>,
    curl: Vec<f32>,
    tracers: Vec<[f32; 2]>,
    time: f32,
    rng: Rng,
    /// Seconds since the kick last burst, so one kick is one burst.
    since_kick: f32,
    since_snare: f32,
}

impl Fluid {
    pub fn new() -> Self {
        let cells = N * N;
        let mut rng = Rng::new(0xF1_0D);
        // Tracers on a jittered lattice rather than pure random: uniform
        // to begin with, so the first frame is a sheet and not a mottle.
        let side = (POINTS as f64).sqrt() as usize;
        let mut tracers = Vec::with_capacity(POINTS);
        for j in 0..side {
            for i in 0..side {
                tracers.push([
                    (i as f32 + rng.f32()) / side as f32,
                    (j as f32 + rng.f32()) / side as f32,
                ]);
            }
        }
        while tracers.len() < POINTS {
            tracers.push([rng.f32(), rng.f32()]);
        }
        Self {
            u: vec![0.0; cells],
            v: vec![0.0; cells],
            u0: vec![0.0; cells],
            v0: vec![0.0; cells],
            p: vec![0.0; cells],
            div: vec![0.0; cells],
            curl: vec![0.0; cells],
            tracers,
            time: 0.0,
            rng,
            since_kick: 10.0,
            since_snare: 10.0,
        }
    }

    /// Bilinear sample of a periodic grid at a position in cell units.
    fn sample(field: &[f32], x: f32, y: f32) -> f32 {
        let x = x.rem_euclid(N as f32);
        let y = y.rem_euclid(N as f32);
        let (i0, j0) = (x.floor() as usize & MASK, y.floor() as usize & MASK);
        let (i1, j1) = ((i0 + 1) & MASK, (j0 + 1) & MASK);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let a = field[i0 + j0 * N] * (1.0 - fx) + field[i1 + j0 * N] * fx;
        let b = field[i0 + j1 * N] * (1.0 - fx) + field[i1 + j1 * N] * fx;
        a * (1.0 - fy) + b * fy
    }

    /// The forces this frame: two stirrers that orbit on their own, the
    /// bands on top. A kick is a burst from the middle, a snare a vortex
    /// pair somewhere, the highs a fine turbulence, the loudness the
    /// stirrers' reach. Without audio the stirrers alone keep it moving.
    fn add_forces(&mut self, dt: f32, drive: &Drive) {
        let t = self.time;
        let lift = if drive.audio { 0.6 + 1.4 * drive.level } else { 1.0 };
        let stirrers = [
            ([0.5 + 0.3 * (t * 0.23).cos(), 0.5 + 0.3 * (t * 0.31).sin()], 1.0),
            ([0.5 + 0.3 * (t * 0.17 + 2.0).sin(), 0.5 + 0.3 * (t * 0.29 + 1.0).cos()], -1.0),
        ];
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.25;
        let snare = drive.audio && drive.bands[2] > 0.5 && self.since_snare > 0.25;
        if kick {
            self.since_kick = 0.0;
        }
        if snare {
            self.since_snare = 0.0;
        }
        let burst_at = [0.5 + 0.2 * (t * 0.7).sin(), 0.5 + 0.2 * (t * 0.5).cos()];
        let vortex_at = [self.rng.f32(), self.rng.f32()];
        let highs = if drive.audio { drive.bands[3] } else { 0.0 };
        for j in 0..N {
            for i in 0..N {
                let idx = i + j * N;
                let x = (i as f32 + 0.5) / N as f32;
                let y = (j as f32 + 0.5) / N as f32;
                let (mut fx, mut fy) = (0.0, 0.0);
                for (c, spin) in stirrers {
                    let (dx, dy) = (wrap(x - c[0]), wrap(y - c[1]));
                    let g = (-(dx * dx + dy * dy) / 0.02).exp();
                    fx += -dy * spin * g * 3.0 * lift;
                    fy += dx * spin * g * 3.0 * lift;
                }
                if kick {
                    let (dx, dy) = (wrap(x - burst_at[0]), wrap(y - burst_at[1]));
                    let r2 = dx * dx + dy * dy;
                    let g = (-r2 / 0.01).exp() * 60.0 * drive.bands[0];
                    fx += dx * g;
                    fy += dy * g;
                }
                if snare {
                    let (dx, dy) = (wrap(x - vortex_at[0]), wrap(y - vortex_at[1]));
                    let g = (-(dx * dx + dy * dy) / 0.006).exp() * 40.0 * drive.bands[2];
                    fx += -dy * g;
                    fy += dx * g;
                }
                if highs > 0.2 {
                    fx += (self.rng.f32() - 0.5) * highs * 4.0;
                    fy += (self.rng.f32() - 0.5) * highs * 4.0;
                }
                self.u[idx] += fx * dt;
                self.v[idx] += fy * dt;
            }
        }
    }

    /// Vorticity confinement (Fedkiw, Stam & Jensen, 2001): push each
    /// cell towards its local vortex centre, so the swirls the
    /// semi-Lagrangian step smears out are put back.
    fn confine(&mut self, dt: f32) {
        let h = 1.0 / N as f32;
        for j in 0..N {
            for i in 0..N {
                let (ip, im) = ((i + 1) & MASK, (i + N - 1) & MASK);
                let (jp, jm) = ((j + 1) & MASK, (j + N - 1) & MASK);
                self.curl[i + j * N] = (self.v[ip + j * N] - self.v[im + j * N]
                    - (self.u[i + jp * N] - self.u[i + jm * N]))
                    * 0.5
                    / h;
            }
        }
        const EPSILON: f32 = 1.5;
        for j in 0..N {
            for i in 0..N {
                let (ip, im) = ((i + 1) & MASK, (i + N - 1) & MASK);
                let (jp, jm) = ((j + 1) & MASK, (j + N - 1) & MASK);
                let gx = (self.curl[ip + j * N].abs() - self.curl[im + j * N].abs()) * 0.5;
                let gy = (self.curl[i + jp * N].abs() - self.curl[i + jm * N].abs()) * 0.5;
                let len = (gx * gx + gy * gy).sqrt() + 1e-5;
                let w = self.curl[i + j * N];
                self.u[i + j * N] += EPSILON * h * (gy / len) * w * dt;
                self.v[i + j * N] -= EPSILON * h * (gx / len) * w * dt;
            }
        }
    }

    /// Make the field divergence-free: solve for a pressure whose
    /// gradient cancels the divergence, Gauss–Seidel, twenty sweeps.
    fn project(&mut self) {
        let h = 1.0 / N as f32;
        for j in 0..N {
            for i in 0..N {
                let (ip, im) = ((i + 1) & MASK, (i + N - 1) & MASK);
                let (jp, jm) = ((j + 1) & MASK, (j + N - 1) & MASK);
                self.div[i + j * N] = -0.5
                    * h
                    * (self.u[ip + j * N] - self.u[im + j * N] + self.v[i + jp * N]
                        - self.v[i + jm * N]);
                self.p[i + j * N] = 0.0;
            }
        }
        for _ in 0..20 {
            for j in 0..N {
                for i in 0..N {
                    let (ip, im) = ((i + 1) & MASK, (i + N - 1) & MASK);
                    let (jp, jm) = ((j + 1) & MASK, (j + N - 1) & MASK);
                    self.p[i + j * N] = (self.div[i + j * N]
                        + self.p[ip + j * N]
                        + self.p[im + j * N]
                        + self.p[i + jp * N]
                        + self.p[i + jm * N])
                        * 0.25;
                }
            }
        }
        for j in 0..N {
            for i in 0..N {
                let (ip, im) = ((i + 1) & MASK, (i + N - 1) & MASK);
                let (jp, jm) = ((j + 1) & MASK, (j + N - 1) & MASK);
                self.u[i + j * N] -= 0.5 * (self.p[ip + j * N] - self.p[im + j * N]) / h;
                self.v[i + j * N] -= 0.5 * (self.p[i + jp * N] - self.p[i + jm * N]) / h;
            }
        }
    }

    /// Semi-Lagrangian advection: each cell takes the velocity from
    /// where its fluid came from. Unconditionally stable, which is the
    /// whole reason this solver can run at any frame rate.
    fn advect(&mut self, dt: f32) {
        std::mem::swap(&mut self.u, &mut self.u0);
        std::mem::swap(&mut self.v, &mut self.v0);
        let scale = dt * N as f32;
        for j in 0..N {
            for i in 0..N {
                let idx = i + j * N;
                let x = i as f32 + 0.5 - scale * self.u0[idx];
                let y = j as f32 + 0.5 - scale * self.v0[idx];
                self.u[idx] = Self::sample(&self.u0, x - 0.5, y - 0.5);
                self.v[idx] = Self::sample(&self.v0, x - 0.5, y - 0.5);
            }
        }
    }

    fn velocity_at(&self, p: [f32; 2]) -> [f32; 2] {
        let (x, y) = (p[0] * N as f32 - 0.5, p[1] * N as f32 - 0.5);
        [Self::sample(&self.u, x, y), Self::sample(&self.v, x, y)]
    }
}

impl Default for Fluid {
    fn default() -> Self {
        Self::new()
    }
}

/// Shortest signed distance on a unit torus.
fn wrap(d: f32) -> f32 {
    d - (d + 0.5).floor()
}

impl Simulation for Fluid {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        self.since_snare += dt;
        self.add_forces(dt, drive);
        self.confine(dt);
        // A little damping, because a torus has no walls to lose energy
        // at and the stirrers never stop; and a ceiling, so a burst on a
        // hot input cannot fling the tracers across the sheet in a frame.
        for (u, v) in self.u.iter_mut().zip(self.v.iter_mut()) {
            *u = (*u * 0.995).clamp(-3.0, 3.0);
            *v = (*v * 0.995).clamp(-3.0, 3.0);
        }
        self.project();
        self.advect(dt);
        self.project();
        // Tracers ride the field, midpoint rule, and wrap.
        for i in 0..self.tracers.len() {
            let p = self.tracers[i];
            let k1 = self.velocity_at(p);
            let mid = [p[0] + 0.5 * dt * k1[0], p[1] + 0.5 * dt * k1[1]];
            let k2 = self.velocity_at(mid);
            self.tracers[i] = [
                (p[0] + dt * k2[0]).rem_euclid(1.0),
                (p[1] + dt * k2[1]).rem_euclid(1.0),
            ];
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        // The sheet lies flat; the vorticity stands up as relief and
        // brightens the point, so the eddies read as eddies and not as
        // a faint drift in a plane.
        for p in &self.tracers {
            let w = Self::sample(&self.curl, p[0] * N as f32 - 0.5, p[1] * N as f32 - 0.5);
            let s = (w / 40.0).clamp(-1.0, 1.0);
            let bright = (0.35 + 0.65 * s.abs()).min(1.0);
            let c = (bright * 255.0) as u8;
            out.push(Point {
                pos: [(p[0] - 0.5) * 2.0, s * 0.3, (p[1] - 0.5) * 2.0],
                normal: [0.0; 3],
                color: [c, c, c],
            });
        }
    }
}

// --- Reaction ---------------------------------------------------------

/// Cells along each side of the reaction grid: one cell per point.
const R: usize = 256;
const RMASK: usize = R - 1;

/// Gray–Scott reaction–diffusion, Pearson's parameterisation: two
/// chemicals, one fed and one killed, diffusing at different rates.
/// The kick drops seeds; the pattern grows, splits and heals.
pub struct Reaction {
    u: Vec<f32>,
    v: Vec<f32>,
    u1: Vec<f32>,
    v1: Vec<f32>,
    rng: Rng,
    since_seed: f32,
    since_kick: f32,
}

impl Reaction {
    /// Feed and kill: the "mitosis" corner of the map (Sims), where spots
    /// divide like cells. A little more feed on a loud passage moves it
    /// towards coral.
    const FEED: f32 = 0.0367;
    const KILL: f32 = 0.0649;
    const DU: f32 = 0.16;
    const DV: f32 = 0.08;

    pub fn new() -> Self {
        let cells = R * R;
        let mut s = Self {
            u: vec![1.0; cells],
            v: vec![0.0; cells],
            u1: vec![1.0; cells],
            v1: vec![0.0; cells],
            rng: Rng::new(0x6EA7),
            since_seed: 0.0,
            since_kick: 10.0,
        };
        for _ in 0..6 {
            s.seed();
        }
        s
    }

    /// A blob of the second chemical, somewhere. A quarter, not full:
    /// a blob of pure v is eaten before it can grow, and a spot that
    /// takes is the point of a seed.
    fn seed(&mut self) {
        let cx = (self.rng.f32() * R as f32) as usize;
        let cy = (self.rng.f32() * R as f32) as usize;
        for dj in 0..8 {
            for di in 0..8 {
                let (i, j) = ((cx + di) & RMASK, (cy + dj) & RMASK);
                self.v[i + j * R] = 0.25;
                self.u[i + j * R] = 0.5;
            }
        }
    }

    fn total_v(&self) -> f32 {
        self.v.iter().sum()
    }
}

impl Default for Reaction {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Reaction {
    fn step(&mut self, dt: f32, drive: &Drive) {
        self.since_seed += dt;
        self.since_kick += dt;
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.3;
        if kick {
            self.since_kick = 0.0;
            self.seed();
            self.since_seed = 0.0;
        }
        // Left alone, the pattern is seeded every few seconds while it is
        // sparse, so a rig with no audio still has something growing.
        if self.since_seed > 3.0 && self.total_v() < 0.02 * (R * R) as f32 {
            self.seed();
            self.since_seed = 0.0;
        }
        let feed = if drive.audio { Self::FEED + 0.012 * drive.level } else { Self::FEED };
        // Several reaction steps per frame: a unit step is what the
        // parameterisation was fitted at, and one a frame is too slow
        // to see anything happen. Eight a frame is a division every few
        // seconds, which is the pace of something alive.
        for _ in 0..8 {
            for j in 0..R {
                for i in 0..R {
                    let (ip, im) = ((i + 1) & RMASK, (i + R - 1) & RMASK);
                    let (jp, jm) = ((j + 1) & RMASK, (j + R - 1) & RMASK);
                    let idx = i + j * R;
                    let u = self.u[idx];
                    let v = self.v[idx];
                    let lap_u = self.u[ip + j * R] + self.u[im + j * R] + self.u[i + jp * R]
                        + self.u[i + jm * R]
                        - 4.0 * u;
                    let lap_v = self.v[ip + j * R] + self.v[im + j * R] + self.v[i + jp * R]
                        + self.v[i + jm * R]
                        - 4.0 * v;
                    let uvv = u * v * v;
                    self.u1[idx] = (u + Self::DU * lap_u - uvv + feed * (1.0 - u)).clamp(0.0, 1.0);
                    self.v1[idx] =
                        (v + Self::DV * lap_v + uvv - (feed + Self::KILL) * v).clamp(0.0, 1.0);
                }
            }
            std::mem::swap(&mut self.u, &mut self.u1);
            std::mem::swap(&mut self.v, &mut self.v1);
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for j in 0..R {
            for i in 0..R {
                let v = self.v[i + j * R];
                let c = (255.0 * (0.25 + 0.75 * v)) as u8;
                out.push(Point {
                    pos: [
                        (i as f32 + 0.5) / R as f32 * 2.0 - 1.0,
                        v * 0.5,
                        (j as f32 + 0.5) / R as f32 * 2.0 - 1.0,
                    ],
                    normal: [0.0; 3],
                    color: [c, c, c],
                });
            }
        }
    }
}

/// A small deterministic generator (xorshift64), so a simulation's
/// self-driven behaviour is the same on every machine.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_ok(pts: &[Point]) {
        assert_eq!(pts.len(), POINTS);
        for p in pts {
            for v in p.pos {
                assert!(v.is_finite() && v.abs() <= 1.001, "{:?}", p.pos);
            }
        }
    }

    /// The fluid stays a fluid: after a run under a hot drive the field
    /// is finite and bounded, the projection has left it divergence-free
    /// to solver tolerance, and every tracer is still on the sheet.
    #[test]
    fn the_fluid_stays_bounded_and_incompressible() {
        let mut f = Fluid::new();
        let loud = Drive { bands: [1.0, 0.8, 1.0, 0.9], level: 1.0, bar: 0.0, audio: true };
        for i in 0..120 {
            let drive = if i % 2 == 0 { loud } else { Drive::default() };
            f.step(1.0 / 60.0, &drive);
        }
        let speed = f.u.iter().zip(&f.v).map(|(u, v)| (u * u + v * v).sqrt()).fold(0.0f32, f32::max);
        assert!(speed.is_finite() && speed > 0.01, "the fluid is not moving: {speed}");
        assert!(speed <= 3.0 * 2f32.sqrt() + 1e-3, "the fluid ran away: {speed}");
        // Divergence after the last projection.
        let h = 1.0 / N as f32;
        let mut worst = 0.0f32;
        for j in 0..N {
            for i in 0..N {
                let (ip, im) = ((i + 1) & MASK, (i + N - 1) & MASK);
                let (jp, jm) = ((j + 1) & MASK, (j + N - 1) & MASK);
                let d = 0.5 * h * (f.u[ip + j * N] - f.u[im + j * N] + f.v[i + jp * N] - f.v[i + jm * N]);
                worst = worst.max(d.abs());
            }
        }
        assert!(worst < 0.05, "the field is not divergence-free: {worst}");
        for t in &f.tracers {
            assert!((0.0..1.0).contains(&t[0]) && (0.0..1.0).contains(&t[1]), "{t:?}");
        }
        let mut pts = Vec::new();
        f.points(&mut pts);
        box_ok(&pts);
    }

    /// The tracers move: the cloud is a fluid, not a lattice.
    #[test]
    fn the_tracers_are_carried() {
        let mut f = Fluid::new();
        let before = f.tracers.clone();
        for _ in 0..60 {
            f.step(1.0 / 60.0, &Drive::default());
        }
        let moved = f
            .tracers
            .iter()
            .zip(&before)
            .filter(|(a, b)| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() > 1e-4)
            .count();
        assert!(moved > POINTS / 4, "only {moved} tracers moved in a second");
    }

    /// The reaction grows from its seeds, stays in range, and a kick
    /// plants a new one.
    #[test]
    fn the_reaction_grows_and_a_kick_seeds_it() {
        let mut r = Reaction::new();
        let start = r.total_v();
        // Growth is slow by design — a spot takes a couple of thousand
        // unit steps to establish and divide — so this runs the seconds
        // a performer would wait, not the frames a test would like.
        for _ in 0..400 {
            r.step(1.0 / 60.0, &Drive::default());
        }
        let grown = r.total_v();
        assert!(grown > start, "the pattern did not grow: {start} -> {grown}");
        assert!(r.u.iter().chain(&r.v).all(|x| (0.0..=1.0).contains(x)));
        let kick = Drive { bands: [1.0, 0.0, 0.0, 0.0], level: 0.5, bar: 0.0, audio: true };
        r.step(1.0 / 60.0, &kick);
        assert!(r.total_v() > grown, "a kick did not seed the reaction");
        let mut pts = Vec::new();
        r.points(&mut pts);
        box_ok(&pts);
    }

    /// Both are reachable by id, and nothing else is.
    #[test]
    fn simulations_start_by_id() {
        for id in IDS {
            assert!(start(id).is_some(), "{id}");
        }
        assert!(start("weather").is_none());
    }
}
