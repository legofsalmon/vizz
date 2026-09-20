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
pub const IDS: &[&str] = &["fluid", "reaction", "flock", "wind", "kuramoto", "life"];

/// Start the simulation `id` names, or `None` for one this crate does
/// not know.
pub fn start(id: &str) -> Option<Box<dyn Simulation>> {
    match id {
        "fluid" => Some(Box::new(Fluid::new())),
        "reaction" => Some(Box::new(Reaction::new())),
        "flock" => Some(Box::new(Flock::new())),
        "wind" => Some(Box::new(Wind::new())),
        "kuramoto" => Some(Box::new(Kuramoto::new())),
        "life" => Some(Box::new(Life::new())),
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

// --- Flock ------------------------------------------------------------

/// Boids in the flock, and the length of the streak each one draws.
/// Their product is a slot.
const BOIDS: usize = 4096;
const TRAIL: usize = 16;
/// Cells along each side of the neighbour grid: a boid sees a radius
/// under one cell, so a neighbour is always in the 27 cells around it.
const FG: usize = 10;

/// Reynolds' boids (1987) in a periodic cube: separation, alignment,
/// cohesion, and nothing else. Each boid draws its last sixteen
/// positions, fading, so the flock reads as streaks rather than dust —
/// which is what a flock looks like from a distance.
pub struct Flock {
    pos: Vec<[f32; 3]>,
    vel: Vec<[f32; 3]>,
    acc: Vec<[f32; 3]>,
    trail: Vec<[f32; 3]>,
    head: usize,
    cells: Vec<Vec<u32>>,
    time: f32,
    since_kick: f32,
    since_snare: f32,
    rng: Rng,
}

impl Flock {
    const SEE: f32 = 0.08;
    const TOO_CLOSE: f32 = 0.03;

    pub fn new() -> Self {
        debug_assert_eq!(BOIDS * TRAIL, POINTS);
        let mut rng = Rng::new(0xF10C_1A5E);
        let mut pos = Vec::with_capacity(BOIDS);
        let mut vel = Vec::with_capacity(BOIDS);
        for _ in 0..BOIDS {
            pos.push([rng.f32(), rng.f32(), rng.f32()]);
            let d = rng.on_sphere();
            vel.push([d[0] * 0.2, d[1] * 0.2, d[2] * 0.2]);
        }
        let trail = pos.iter().flat_map(|p| std::iter::repeat_n(*p, TRAIL)).collect();
        Self {
            pos,
            vel,
            acc: vec![[0.0; 3]; BOIDS],
            trail,
            head: 0,
            cells: vec![Vec::new(); FG * FG * FG],
            time: 0.0,
            since_kick: 10.0,
            since_snare: 10.0,
            rng,
        }
    }

    fn cell_of(p: [f32; 3]) -> [usize; 3] {
        [0, 1, 2].map(|i| ((p[i] * FG as f32) as usize).min(FG - 1))
    }
}

impl Default for Flock {
    fn default() -> Self {
        Self::new()
    }
}

fn wrap3(d: [f32; 3]) -> [f32; 3] {
    [wrap(d[0]), wrap(d[1]), wrap(d[2])]
}

impl Simulation for Flock {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        self.since_snare += dt;
        for c in &mut self.cells {
            c.clear();
        }
        for (i, p) in self.pos.iter().enumerate() {
            let [cx, cy, cz] = Self::cell_of(*p);
            self.cells[cx + cy * FG + cz * FG * FG].push(i as u32);
        }
        let see2 = Self::SEE * Self::SEE;
        let close2 = Self::TOO_CLOSE * Self::TOO_CLOSE;
        for i in 0..BOIDS {
            let p = self.pos[i];
            let [cx, cy, cz] = Self::cell_of(p);
            let (mut centre, mut align, mut apart) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
            let mut n = 0.0f32;
            for dz in 0..3 {
                for dy in 0..3 {
                    for dx in 0..3 {
                        let cell = &self.cells[(cx + dx + FG - 1) % FG
                            + ((cy + dy + FG - 1) % FG) * FG
                            + ((cz + dz + FG - 1) % FG) * FG * FG];
                        for &j in cell {
                            let j = j as usize;
                            if j == i {
                                continue;
                            }
                            let d = wrap3([
                                self.pos[j][0] - p[0],
                                self.pos[j][1] - p[1],
                                self.pos[j][2] - p[2],
                            ]);
                            let r2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                            if r2 > see2 {
                                continue;
                            }
                            n += 1.0;
                            for k in 0..3 {
                                centre[k] += d[k];
                                align[k] += self.vel[j][k];
                            }
                            if r2 < close2 {
                                let push = 1.0 / (r2 + 1e-4);
                                for k in 0..3 {
                                    apart[k] -= d[k] * push;
                                }
                            }
                        }
                    }
                }
            }
            let mut a = [0.0f32; 3];
            if n > 0.0 {
                for k in 0..3 {
                    a[k] += centre[k] / n * 4.0;
                    a[k] += (align[k] / n - self.vel[i][k]) * 3.0;
                    a[k] += apart[k] * 0.002;
                }
            }
            self.acc[i] = a;
        }
        // The room: a kick is a predator bursting through a random point,
        // a snare scatters headings, the loudness is the flock's pace.
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.3;
        let snare = drive.audio && drive.bands[2] > 0.5 && self.since_snare > 0.3;
        if kick {
            self.since_kick = 0.0;
            let at = [self.rng.f32(), self.rng.f32(), self.rng.f32()];
            for i in 0..BOIDS {
                let d = wrap3([self.pos[i][0] - at[0], self.pos[i][1] - at[1], self.pos[i][2] - at[2]]);
                let r2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                if r2 < 0.06 {
                    let g = (1.0 - r2 / 0.06) * 12.0 / (r2.sqrt() + 1e-3);
                    for (a, d) in self.acc[i].iter_mut().zip(d) {
                        *a += d * g;
                    }
                }
            }
        }
        if snare {
            self.since_snare = 0.0;
            for i in 0..BOIDS {
                let d = self.rng.on_sphere();
                for (a, d) in self.acc[i].iter_mut().zip(d) {
                    *a += d * 3.0;
                }
            }
        }
        let pace = if drive.audio { 0.18 + 0.32 * drive.level } else { 0.3 };
        for ((v, a), p) in self.vel.iter_mut().zip(&self.acc).zip(self.pos.iter_mut()) {
            for (v, a) in v.iter_mut().zip(a) {
                *v += a * dt;
            }
            let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let want = speed.clamp(pace * 0.5, pace);
            if speed > 1e-6 {
                for v in v.iter_mut() {
                    *v *= want / speed;
                }
            }
            for (p, v) in p.iter_mut().zip(v.iter()) {
                *p = (*p + v * dt).rem_euclid(1.0);
            }
        }
        self.head = (self.head + 1) % TRAIL;
        for i in 0..BOIDS {
            self.trail[i * TRAIL + self.head] = self.pos[i];
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for i in 0..BOIDS {
            for age in 0..TRAIL {
                let slot = (self.head + TRAIL - age) % TRAIL;
                let p = self.trail[i * TRAIL + slot];
                let fade = 1.0 - age as f32 / TRAIL as f32 * 0.85;
                let c = (255.0 * fade) as u8;
                out.push(Point {
                    pos: [(p[0] - 0.5) * 2.0, (p[1] - 0.5) * 2.0, (p[2] - 0.5) * 2.0],
                    normal: [0.0; 3],
                    color: [c, c, c],
                });
            }
        }
    }
}

// --- Wind -------------------------------------------------------------

/// Cells along each side of the curl-noise field.
const WG: usize = 24;

/// Curl noise (Bridson, Hourihan & Nordenstam, 2007): the curl of a
/// smooth noise field is divergence-free, so tracers carried by it
/// flow like a fluid with no solve at all. Sampled onto a grid every
/// few frames and trilinearly interpolated, because sixty-five thousand
/// tracers evaluating six noise gradients each would not make the frame.
pub struct Wind {
    field: Vec<[f32; 3]>,
    tracers: Vec<[f32; 3]>,
    noise: Noise,
    time: f32,
    frame: u32,
    gust: f32,
    since_kick: f32,
}

impl Wind {
    pub fn new() -> Self {
        let mut rng = Rng::new(0x1D_A11E);
        let tracers = (0..POINTS).map(|_| [rng.f32(), rng.f32(), rng.f32()]).collect();
        let mut w = Self {
            field: vec![[0.0; 3]; WG * WG * WG],
            tracers,
            noise: Noise::new(0x5EED),
            time: 0.0,
            frame: 0,
            gust: 0.0,
            since_kick: 10.0,
        };
        w.resample(0.0);
        w
    }

    /// The potential: three noise fields, offset from each other.
    fn potential(&self, p: [f32; 3], t: f32, rough: f32) -> [f32; 3] {
        let f = 2.0;
        let mut out = [0.0f32; 3];
        for (k, offset) in [0.0f32, 31.7, 67.3].into_iter().enumerate() {
            let base = self.noise.at(p[0] * f + offset, p[1] * f + offset, p[2] * f + t);
            let fine = self.noise.at(p[0] * f * 2.7 + offset, p[1] * f * 2.7, p[2] * f * 2.7 + t * 1.6);
            out[k] = base + rough * 0.5 * fine;
        }
        out
    }

    fn resample(&mut self, rough: f32) {
        let t = self.time * 0.15;
        let eps = 0.01;
        for k in 0..WG {
            for j in 0..WG {
                for i in 0..WG {
                    let p = [(i as f32 + 0.5) / WG as f32, (j as f32 + 0.5) / WG as f32, (k as f32 + 0.5) / WG as f32];
                    let d = |axis: usize| {
                        let mut a = p;
                        let mut b = p;
                        a[axis] += eps;
                        b[axis] -= eps;
                        let (pa, pb) = (self.potential(a, t, rough), self.potential(b, t, rough));
                        [(pa[0] - pb[0]) / (2.0 * eps), (pa[1] - pb[1]) / (2.0 * eps), (pa[2] - pb[2]) / (2.0 * eps)]
                    };
                    let (dx, dy, dz) = (d(0), d(1), d(2));
                    // curl ψ = (∂ψz/∂y − ∂ψy/∂z, ∂ψx/∂z − ∂ψz/∂x, ∂ψy/∂x − ∂ψx/∂y)
                    self.field[i + j * WG + k * WG * WG] =
                        [dy[2] - dz[1], dz[0] - dx[2], dx[1] - dy[0]];
                }
            }
        }
    }

    fn velocity_at(&self, p: [f32; 3]) -> [f32; 3] {
        let g = WG as f32;
        let (x, y, z) = (p[0] * g - 0.5, p[1] * g - 0.5, p[2] * g - 0.5);
        let (i0, j0, k0) = (x.floor(), y.floor(), z.floor());
        let (fx, fy, fz) = (x - i0, y - j0, z - k0);
        let idx = |i: f32, j: f32, k: f32| {
            (i.rem_euclid(g) as usize) + (j.rem_euclid(g) as usize) * WG + (k.rem_euclid(g) as usize) * WG * WG
        };
        let mut out = [0.0f32; 3];
        for (di, wx) in [(0.0, 1.0 - fx), (1.0, fx)] {
            for (dj, wy) in [(0.0, 1.0 - fy), (1.0, fy)] {
                for (dk, wz) in [(0.0, 1.0 - fz), (1.0, fz)] {
                    let v = self.field[idx(i0 + di, j0 + dj, k0 + dk)];
                    let w = wx * wy * wz;
                    for c in 0..3 {
                        out[c] += v[c] * w;
                    }
                }
            }
        }
        out
    }
}

impl Default for Wind {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Wind {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.frame += 1;
        self.since_kick += dt;
        self.gust *= (-dt * 2.5).exp();
        // A kick is a gust: the field jumps to a new moment and the
        // tracers run for half a second.
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.3 {
            self.since_kick = 0.0;
            self.time += 3.0;
            self.gust = 1.0;
        }
        let rough = if drive.audio { drive.bands[3] } else { 0.0 };
        if self.frame % 3 == 1 {
            self.resample(rough);
        }
        let pace = (if drive.audio { 0.05 + 0.12 * drive.level } else { 0.09 }) + 0.25 * self.gust;
        for i in 0..self.tracers.len() {
            let p = self.tracers[i];
            let v = self.velocity_at(p);
            self.tracers[i] = [
                (p[0] + v[0] * pace * dt).rem_euclid(1.0),
                (p[1] + v[1] * pace * dt).rem_euclid(1.0),
                (p[2] + v[2] * pace * dt).rem_euclid(1.0),
            ];
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for p in &self.tracers {
            let v = self.velocity_at(*p);
            let s = ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() / 6.0).min(1.0);
            let c = (255.0 * (0.35 + 0.65 * s)) as u8;
            out.push(Point {
                pos: [(p[0] - 0.5) * 2.0, (p[1] - 0.5) * 2.0, (p[2] - 0.5) * 2.0],
                normal: [0.0; 3],
                color: [c, c, c],
            });
        }
    }
}

/// Perlin's improved gradient noise (2002), seeded, in [-1, 1].
struct Noise {
    perm: [u8; 512],
}

impl Noise {
    fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let mut p: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in (1..256).rev() {
            let j = (rng.next() % (i as u64 + 1)) as usize;
            p.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Self { perm }
    }

    fn at(&self, x: f32, y: f32, z: f32) -> f32 {
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let lerp = |a: f32, b: f32, t: f32| a + t * (b - a);
        let grad = |h: u8, x: f32, y: f32, z: f32| {
            let h = h & 15;
            let u = if h < 8 { x } else { y };
            let v = if h < 4 {
                y
            } else if h == 12 || h == 14 {
                x
            } else {
                z
            };
            (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
        };
        let (xi, yi, zi) = (x.floor(), y.floor(), z.floor());
        let (xf, yf, zf) = (x - xi, y - yi, z - zi);
        let (xi, yi, zi) = (xi as i32 as usize & 255, yi as i32 as usize & 255, zi as i32 as usize & 255);
        let (u, v, w) = (fade(xf), fade(yf), fade(zf));
        let p = &self.perm;
        let a = p[xi] as usize + yi;
        let aa = p[a] as usize + zi;
        let ab = p[a + 1] as usize + zi;
        let b = p[xi + 1] as usize + yi;
        let ba = p[b] as usize + zi;
        let bb = p[b + 1] as usize + zi;
        lerp(
            lerp(
                lerp(grad(p[aa], xf, yf, zf), grad(p[ba], xf - 1.0, yf, zf), u),
                lerp(grad(p[ab], xf, yf - 1.0, zf), grad(p[bb], xf - 1.0, yf - 1.0, zf), u),
                v,
            ),
            lerp(
                lerp(grad(p[aa + 1], xf, yf, zf - 1.0), grad(p[ba + 1], xf - 1.0, yf, zf - 1.0), u),
                lerp(
                    grad(p[ab + 1], xf, yf - 1.0, zf - 1.0),
                    grad(p[bb + 1], xf - 1.0, yf - 1.0, zf - 1.0),
                    u,
                ),
                v,
            ),
            w,
        )
    }
}

// --- Kuramoto ---------------------------------------------------------

/// Oscillators on the ring, and how many past phases each one draws.
/// Their product is a slot.
const OSC: usize = 256;
const HIST: usize = 256;

/// Kuramoto's coupled oscillators (1975): every oscillator has its own
/// pace, every one pulls every other towards the crowd's phase, and
/// above a critical coupling they lock. Drawn as a torus — the ring is
/// which oscillator, the tube is its phase, the trail is its recent
/// past — so a locked crowd is a thin ribbon and a free one is the
/// whole tube. The loudness is the coupling: a loud passage locks them.
pub struct Kuramoto {
    theta: Vec<f32>,
    omega: Vec<f32>,
    hist: Vec<f32>,
    head: usize,
    time: f32,
    order: f32,
    since_kick: f32,
    rng: Rng,
}

impl Kuramoto {
    pub fn new() -> Self {
        debug_assert_eq!(OSC * HIST, POINTS);
        let mut rng = Rng::new(0xC0_A11E);
        let theta: Vec<f32> = (0..OSC).map(|_| rng.f32() * std::f32::consts::TAU).collect();
        // Paces drawn from a Cauchy distribution, the textbook choice:
        // it has the heavy tails that keep a few oscillators free
        // however hard the crowd pulls.
        let omega = (0..OSC)
            .map(|_| 1.6 + 0.4 * ((rng.f32() - 0.5) * std::f32::consts::PI).tan().clamp(-6.0, 6.0))
            .collect();
        let hist = theta.iter().flat_map(|t| std::iter::repeat_n(*t, HIST)).collect();
        Self { theta, omega, hist, head: 0, time: 0.0, order: 0.0, since_kick: 10.0, rng }
    }

    /// The order parameter: how locked the crowd is, 0 to 1, and where
    /// it is.
    fn mean_field(&self) -> (f32, f32) {
        let (mut c, mut s) = (0.0f32, 0.0f32);
        for t in &self.theta {
            c += t.cos();
            s += t.sin();
        }
        let n = OSC as f32;
        ((c * c + s * s).sqrt() / n, s.atan2(c))
    }
}

impl Default for Kuramoto {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Kuramoto {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        // Without audio the coupling breathes across the threshold on
        // its own, so the crowd locks and frees over a minute or so.
        let coupling = if drive.audio {
            0.3 + 4.0 * drive.level
        } else {
            1.2 + 1.2 * (self.time * 0.1).sin()
        };
        // A kick scatters half the crowd.
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.4 {
            self.since_kick = 0.0;
            for i in 0..OSC {
                if self.rng.f32() < 0.5 {
                    self.theta[i] += (self.rng.f32() - 0.5) * 2.0;
                }
            }
        }
        let (r, psi) = self.mean_field();
        self.order = r;
        for i in 0..OSC {
            let d = self.omega[i] + coupling * r * (psi - self.theta[i]).sin();
            self.theta[i] = (self.theta[i] + d * dt).rem_euclid(std::f32::consts::TAU);
        }
        self.head = (self.head + 1) % HIST;
        for i in 0..OSC {
            self.hist[i * HIST + self.head] = self.theta[i];
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        const R: f32 = 0.62;
        let bright = 0.4 + 0.6 * self.order;
        for i in 0..OSC {
            let phi = i as f32 / OSC as f32 * std::f32::consts::TAU;
            for age in 0..HIST {
                let slot = (self.head + HIST - age) % HIST;
                let theta = self.hist[i * HIST + slot];
                let fade = 1.0 - age as f32 / HIST as f32;
                let r = 0.32 * (0.25 + 0.75 * fade);
                let ring = R + r * theta.cos();
                let c = (255.0 * bright * (0.3 + 0.7 * fade)) as u8;
                out.push(Point {
                    pos: [ring * phi.cos(), r * theta.sin(), ring * phi.sin()],
                    normal: [0.0; 3],
                    color: [c, c, c],
                });
            }
        }
    }
}

// --- Life -------------------------------------------------------------

/// Cells along each side of the automaton's lattice.
const LG: usize = 48;

/// A three-dimensional cellular automaton — Bays' generalisation of
/// Life to a cubic lattice with the twenty-six-cell neighbourhood, in
/// the "Clouds" rule (survive on 13–26 neighbours, born on 13, 14 or
/// 17–19): a seed grows into slow, cloud-like masses that keep
/// reshaping. A generation every three frames; the kick drops a new
/// seed.
pub struct Life {
    cells: Vec<u8>,
    next: Vec<u8>,
    frame: u32,
    since_kick: f32,
    rng: Rng,
}

impl Life {
    pub fn new() -> Self {
        let mut l = Self {
            cells: vec![0; LG * LG * LG],
            next: vec![0; LG * LG * LG],
            frame: 0,
            since_kick: 10.0,
            rng: Rng::new(0x11FE),
        };
        l.seed(LG / 2, LG / 2, LG / 2, 14);
        l
    }

    /// A random block, about half full, centred on a cell.
    fn seed(&mut self, cx: usize, cy: usize, cz: usize, side: usize) {
        for dz in 0..side {
            for dy in 0..side {
                for dx in 0..side {
                    let (x, y, z) = (
                        (cx + dx + LG - side / 2) % LG,
                        (cy + dy + LG - side / 2) % LG,
                        (cz + dz + LG - side / 2) % LG,
                    );
                    if self.rng.f32() < 0.55 {
                        self.cells[x + y * LG + z * LG * LG] = 1;
                    }
                }
            }
        }
    }

    fn generation(&mut self) {
        let at = |x: usize, y: usize, z: usize| (x % LG) + (y % LG) * LG + (z % LG) * LG * LG;
        for z in 0..LG {
            for y in 0..LG {
                for x in 0..LG {
                    let mut n = 0u8;
                    for dz in 0..3 {
                        for dy in 0..3 {
                            for dx in 0..3 {
                                if dx == 1 && dy == 1 && dz == 1 {
                                    continue;
                                }
                                n += self.cells[at(x + dx + LG - 1, y + dy + LG - 1, z + dz + LG - 1)];
                            }
                        }
                    }
                    let alive = self.cells[at(x, y, z)] == 1;
                    let born = matches!(n, 13 | 14 | 17..=19);
                    let survives = n >= 13;
                    self.next[at(x, y, z)] = u8::from(if alive { survives } else { born });
                }
            }
        }
        std::mem::swap(&mut self.cells, &mut self.next);
    }

    fn alive(&self) -> usize {
        self.cells.iter().filter(|c| **c == 1).count()
    }
}

impl Default for Life {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Life {
    fn step(&mut self, dt: f32, drive: &Drive) {
        self.frame += 1;
        self.since_kick += dt;
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.5 {
            self.since_kick = 0.0;
            let (x, y, z) = (
                (self.rng.f32() * LG as f32) as usize,
                (self.rng.f32() * LG as f32) as usize,
                (self.rng.f32() * LG as f32) as usize,
            );
            self.seed(x, y, z, 8);
        }
        if self.frame.is_multiple_of(3) {
            self.generation();
            // Died out, or filled the lattice: start again from a seed.
            // A dead automaton is a blank slot with a name on it.
            let alive = self.alive();
            if !(64..=LG * LG * LG * 9 / 10).contains(&alive) {
                self.cells.iter_mut().for_each(|c| *c = 0);
                let (x, y, z) = (
                    (self.rng.f32() * LG as f32) as usize,
                    (self.rng.f32() * LG as f32) as usize,
                    (self.rng.f32() * LG as f32) as usize,
                );
                self.seed(x, y, z, 14);
            }
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        let alive: Vec<usize> = (0..self.cells.len()).filter(|i| self.cells[*i] == 1).collect();
        if alive.is_empty() {
            return;
        }
        // A slot's worth, however many are alive: a stride across them
        // when there are more, a jittered repeat when there are fewer —
        // the loader's own policy, applied here so the count is right
        // before the fit sees it.
        let mut rng = Rng::new(0x11FE_5EED);
        for i in 0..POINTS {
            let pick = if alive.len() >= POINTS {
                alive[(i as u64 * alive.len() as u64 / POINTS as u64) as usize]
            } else {
                alive[i % alive.len()]
            };
            let (x, y, z) = (pick % LG, (pick / LG) % LG, pick / (LG * LG));
            let j = if alive.len() >= POINTS { [0.0; 3] } else { [rng.f32() - 0.5, rng.f32() - 0.5, rng.f32() - 0.5] };
            out.push(Point {
                pos: [
                    ((x as f32 + 0.5 + j[0] * 0.8) / LG as f32 - 0.5) * 2.0,
                    ((y as f32 + 0.5 + j[1] * 0.8) / LG as f32 - 0.5) * 2.0,
                    ((z as f32 + 0.5 + j[2] * 0.8) / LG as f32 - 0.5) * 2.0,
                ],
                normal: [0.0; 3],
                color: [255, 255, 255],
            });
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

    fn on_sphere(&mut self) -> [f32; 3] {
        let u = self.f32() * 2.0 - 1.0;
        let a = self.f32() * std::f32::consts::TAU;
        let s = (1.0 - u * u).sqrt();
        [s * a.cos(), s * a.sin(), u]
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

    /// The flock stays in its cube, keeps moving at its pace, and draws a
    /// full slot of streaks.
    #[test]
    fn the_flock_stays_in_the_cube_and_moves() {
        let mut f = Flock::new();
        let before = f.pos.clone();
        let loud = Drive { bands: [1.0, 0.0, 1.0, 0.0], level: 1.0, bar: 0.0, audio: true };
        let quiet = Drive::default();
        for i in 0..120 {
            f.step(1.0 / 60.0, if i % 30 == 0 { &loud } else { &quiet });
        }
        for (p, v) in f.pos.iter().zip(&f.vel) {
            assert!(p.iter().all(|c| (0.0..1.0).contains(c)), "{p:?}");
            assert!(v.iter().all(|c| c.is_finite()), "{v:?}");
        }
        let moved = f.pos.iter().zip(&before).filter(|(a, b)| a != b).count();
        assert_eq!(moved, BOIDS, "some boids never moved");
        let mut pts = Vec::new();
        f.points(&mut pts);
        box_ok(&pts);
    }

    /// The wind carries every tracer and keeps it in the cube.
    #[test]
    fn the_wind_carries_the_tracers() {
        let mut w = Wind::new();
        let before = w.tracers.clone();
        for _ in 0..60 {
            w.step(1.0 / 60.0, &Drive::default());
        }
        let moved = w.tracers.iter().zip(&before).filter(|(a, b)| a != b).count();
        assert!(moved > POINTS * 9 / 10, "only {moved} tracers moved");
        assert!(w.tracers.iter().all(|p| p.iter().all(|c| (0.0..1.0).contains(c))));
        let mut pts = Vec::new();
        w.points(&mut pts);
        box_ok(&pts);
        // And the noise is noise: bounded, and not constant.
        let n = Noise::new(1);
        let samples: Vec<f32> = (0..100).map(|i| n.at(i as f32 * 0.37, 0.5, i as f32 * 0.11)).collect();
        assert!(samples.iter().all(|v| v.abs() <= 1.0));
        assert!(samples.iter().any(|v| v.abs() > 0.1));
    }

    /// Strong coupling locks the crowd and weak coupling leaves it free —
    /// the whole point of the model, and what the loudness plays.
    #[test]
    fn the_crowd_locks_under_coupling_and_not_without() {
        let mut k = Kuramoto::new();
        let loud = Drive { bands: [0.0; 4], level: 1.0, bar: 0.0, audio: true };
        for _ in 0..900 {
            k.step(1.0 / 60.0, &loud);
        }
        assert!(k.order > 0.8, "a loud room did not lock the crowd: r = {}", k.order);
        let mut k = Kuramoto::new();
        let quiet = Drive { bands: [0.0; 4], level: 0.0, bar: 0.0, audio: true };
        for _ in 0..900 {
            k.step(1.0 / 60.0, &quiet);
        }
        assert!(k.order < 0.5, "a quiet room locked the crowd: r = {}", k.order);
        let mut pts = Vec::new();
        k.points(&mut pts);
        box_ok(&pts);
    }

    /// The automaton neither dies nor floods over a run, and always hands
    /// over a full slot.
    #[test]
    fn life_keeps_living() {
        let mut l = Life::new();
        for _ in 0..300 {
            l.step(1.0 / 60.0, &Drive::default());
        }
        let alive = l.alive();
        let cells = LG * LG * LG;
        assert!(alive > cells / 200 && alive < cells * 9 / 10, "{alive} of {cells} alive");
        let mut pts = Vec::new();
        l.points(&mut pts);
        box_ok(&pts);
    }

    /// Every simulation is reachable by id, and nothing else is.
    #[test]
    fn simulations_start_by_id() {
        for id in IDS {
            assert!(start(id).is_some(), "{id}");
        }
        assert!(start("weather").is_none());
    }
}
