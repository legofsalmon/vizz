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
//! Ten ship, and they are all one of three kinds. Some are *fields on
//! a grid*: **fluid** is Stam's stable solver for the incompressible
//! Navier–Stokes equations (Stam, "Stable Fluids", 1999; "Real-Time
//! Fluid Dynamics for Games", 2003) on a periodic sheet, with Fedkiw's
//! vorticity confinement to keep the swirls alive; **smoke** is the
//! same solver in a box, with heat to lift it; **reaction** is the
//! Gray–Scott system in Pearson's parameterisation; **wind** is curl
//! noise, a fluid with no solve at all; **life** is a cellular
//! automaton on a cubic lattice. Some are *many bodies*: **flock** is
//! Reynolds' boids, **orbits** is gravity by direct summation,
//! **liquid** is position-based fluids, **pendulum** is four thousand
//! double pendulums hung in a sheet. And **kuramoto** is a crowd of
//! coupled oscillators, which is neither.
//!
//! They cost between half a millisecond and twenty per frame, on a
//! thread of their own, and a slow one loses frames rather than the
//! picture: the renderer takes whatever the slot holds.

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
pub const IDS: &[&str] =
    &["fluid", "reaction", "flock", "wind", "kuramoto", "life", "orbits", "pendulum", "smoke", "liquid"];

/// Start the simulation `id` names, or `None` for one this crate does
/// not know.
pub fn start(spec: &str) -> Option<Box<dyn Simulation>> {
    let (id, settings) = match spec.split_once('?') {
        None => (spec, Vec::new()),
        Some((id, rest)) => (
            id,
            rest.split(';')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.trim(), v.trim()))
                .collect::<Vec<_>>(),
        ),
    };
    let text = |key: &str, default: &str| -> String {
        settings.iter().find(|(k, _)| *k == key).map_or(default, |(_, v)| *v).to_string()
    };
    match id {
        "fluid" => Some(Box::new(Fluid::new())),
        "reaction" => Some(Box::new(Reaction::new())),
        "flock" => Some(Box::new(Flock::new())),
        "wind" => Some(Box::new(Wind::new())),
        "kuramoto" => Some(Box::new(Kuramoto::new())),
        "life" => Some(Box::new(Life::with_rule(&text("rule", Life::CLOUDS)))),
        "orbits" => Some(Box::new(Orbits::new())),
        "pendulum" => Some(Box::new(Pendulum::new())),
        "smoke" => Some(Box::new(Smoke::new())),
        "liquid" => Some(Box::new(Liquid::new())),
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
/// Life to a cubic lattice with the twenty-six-cell neighbourhood, and
/// the family of rules that grew out of it.
///
/// A rule is `survive/born/states`. The first two are neighbour counts,
/// as Life's are. The third is the one that makes this family worth
/// having: with two states a cell is alive or dead, but with more, a
/// cell that fails to survive does not die at once — it counts down
/// through the intermediate states, one a generation, and only the top
/// state counts as a live neighbour to anybody. Those counting-down
/// cells are the ash a growing front leaves behind it, and they turn a
/// flat rule into a solid that grows, hollows and crusts. `4/4/5` is
/// Bays' 4-4-5, a crystal that spikes outward; `4-7/6-8/10` is
/// Pyroclastic, which boils; `9-26/5-7,12-13,15/5` is Amoeba.
///
/// A generation every three frames; the kick drops a new seed.
pub struct Life {
    /// 0 is dead, `states - 1` is alive, and everything between is a
    /// cell part of the way through dying.
    cells: Vec<u8>,
    next: Vec<u8>,
    /// Indexed by neighbour count: whether a live cell survives, and
    /// whether a dead one is born.
    survive: [bool; 27],
    born: [bool; 27],
    /// How many states, counting dead: 2 is Life's own.
    states: u8,
    frame: u32,
    since_kick: f32,
    rng: Rng,
}

impl Life {
    /// The Clouds rule, as `survive/born` in neighbour counts. Two
    /// states, so the third field can be left off.
    pub const CLOUDS: &'static str = "13-26/13-14,17-19";

    pub fn new() -> Self {
        Self::with_rule(Self::CLOUDS)
    }

    /// An automaton in the rule `survive/born` or `survive/born/states`,
    /// the first two sides lists of neighbour counts and ranges —
    /// `13-26/13-14,17-19`, `4/4/5`. A rule that will not parse is the
    /// Clouds rule, because a blank rule is a blank slot.
    pub fn with_rule(rule: &str) -> Self {
        let (survive, born, states) = parse_rule(rule)
            .unwrap_or_else(|| parse_rule(Self::CLOUDS).expect("the shipped rule parses"));
        let mut l = Self {
            cells: vec![0; LG * LG * LG],
            next: vec![0; LG * LG * LG],
            survive,
            born,
            states,
            frame: 0,
            since_kick: 10.0,
            rng: Rng::new(0x11FE),
        };
        let side = l.seed_side();
        l.seed(LG / 2, LG / 2, LG / 2, side);
        l
    }

    /// How big a starting block to drop. The two-state rules here are
    /// the cloud-like ones, which need a crowd to get going; the
    /// multi-state ones grow outward from almost nothing and fill the
    /// lattice if handed a crowd.
    fn seed_side(&self) -> usize {
        if self.states > 2 { 6 } else { 14 }
    }

    /// A random block, about half full, centred on a cell.
    fn seed(&mut self, cx: usize, cy: usize, cz: usize, side: usize) {
        let alive = self.states - 1;
        for dz in 0..side {
            for dy in 0..side {
                for dx in 0..side {
                    let (x, y, z) = (
                        (cx + dx + LG - side / 2) % LG,
                        (cy + dy + LG - side / 2) % LG,
                        (cz + dz + LG - side / 2) % LG,
                    );
                    if self.rng.f32() < 0.55 {
                        self.cells[x + y * LG + z * LG * LG] = alive;
                    }
                }
            }
        }
    }

    fn generation(&mut self) {
        let at = |x: usize, y: usize, z: usize| (x % LG) + (y % LG) * LG + (z % LG) * LG * LG;
        let top = self.states - 1;
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
                                // Only a cell at the top of the ramp is
                                // a neighbour; the ash is inert.
                                n += u8::from(
                                    self.cells
                                        [at(x + dx + LG - 1, y + dy + LG - 1, z + dz + LG - 1)]
                                        == top,
                                );
                            }
                        }
                    }
                    let cell = self.cells[at(x, y, z)];
                    let n = n as usize;
                    self.next[at(x, y, z)] = match cell {
                        0 => u8::from(self.born[n]) * top,
                        c if c == top => {
                            if self.survive[n] {
                                top
                            } else {
                                top - 1
                            }
                        }
                        // Dying, and nothing brings it back: the ramp
                        // only runs one way.
                        c => c - 1,
                    };
                }
            }
        }
        std::mem::swap(&mut self.cells, &mut self.next);
    }

    /// Every cell that is not dead — the live front and the ash behind
    /// it, which is what the cloud draws.
    fn alive(&self) -> usize {
        self.cells.iter().filter(|c| **c > 0).count()
    }
}

impl Default for Life {
    fn default() -> Self {
        Self::new()
    }
}

/// `survive/born` or `survive/born/states`, the first two each a comma
/// list of counts and `lo-hi` ranges over 0–26, the third a state count
/// from 2 to 32. `None` when a side is missing or holds anything else.
fn parse_rule(rule: &str) -> Option<([bool; 27], [bool; 27], u8)> {
    let mut fields = rule.split('/');
    let (s, b) = (fields.next()?, fields.next()?);
    let states = match fields.next() {
        None => 2u8,
        Some(text) => match text.trim().parse::<u8>().ok()? {
            n @ 2..=32 => n,
            _ => return None,
        },
    };
    if fields.next().is_some() {
        return None;
    }
    let side = |text: &str| -> Option<[bool; 27]> {
        let mut set = [false; 27];
        for item in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let (lo, hi) = match item.split_once('-') {
                Some((lo, hi)) => (lo.trim().parse::<usize>().ok()?, hi.trim().parse::<usize>().ok()?),
                None => {
                    let n = item.parse::<usize>().ok()?;
                    (n, n)
                }
            };
            if lo > hi || hi > 26 {
                return None;
            }
            set[lo..=hi].iter_mut().for_each(|on| *on = true);
        }
        Some(set)
    };
    let (s, b) = (side(s)?, side(b)?);
    (s.iter().any(|x| *x) || b.iter().any(|x| *x)).then_some((s, b, states))
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
            let side = self.seed_side().min(8);
            self.seed(x, y, z, side);
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
                let side = self.seed_side();
                self.seed(x, y, z, side);
            }
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        let alive: Vec<usize> = (0..self.cells.len()).filter(|i| self.cells[*i] > 0).collect();
        if alive.is_empty() {
            return;
        }
        // A slot's worth, however many are alive: a stride across them
        // when there are more, a jittered repeat when there are fewer —
        // the loader's own policy, applied here so the count is right
        // before the fit sees it.
        let mut rng = Rng::new(0x11FE_5EED);
        let top = f32::from(self.states - 1);
        for i in 0..POINTS {
            let pick = if alive.len() >= POINTS {
                alive[(i as u64 * alive.len() as u64 / POINTS as u64) as usize]
            } else {
                alive[i % alive.len()]
            };
            let (x, y, z) = (pick % LG, (pick / LG) % LG, pick / (LG * LG));
            let j = if alive.len() >= POINTS { [0.0; 3] } else { [rng.f32() - 0.5, rng.f32() - 0.5, rng.f32() - 0.5] };
            // The ramp, as brightness: the live front is white and the
            // ash behind it goes down towards grey, so a growing rule
            // reads as a crust with a glow on the leading face.
            let age = f32::from(self.cells[pick]) / top;
            let shade = (96.0 + 159.0 * age) as u8;
            out.push(Point {
                pos: [
                    ((x as f32 + 0.5 + j[0] * 0.8) / LG as f32 - 0.5) * 2.0,
                    ((y as f32 + 0.5 + j[1] * 0.8) / LG as f32 - 0.5) * 2.0,
                    ((z as f32 + 0.5 + j[2] * 0.8) / LG as f32 - 0.5) * 2.0,
                ],
                normal: [0.0; 3],
                color: [shade, shade, shade],
            });
        }
    }
}

// --- Orbits -----------------------------------------------------------

/// Bodies in the system, and how many past positions each one keeps.
/// Their product is a slot.
const BODIES: usize = 512;
const ARC: usize = 128;

/// Gravity, by direct summation: every body pulls on every other one,
/// all two hundred and sixty thousand pairs of them, every frame. No
/// tree, no multipoles — at five hundred bodies the honest sum is
/// cheaper than the bookkeeping that avoids it, and the honest sum is
/// what makes the clumping real.
///
/// The system starts as a disc around a heavy centre, which is the one
/// arrangement that holds together long enough to watch: circular
/// orbits, a little inclination, and then whatever the bodies do to
/// each other. Each one keeps its last two seconds as an arc, so the
/// cloud is orbits rather than dots.
///
/// Softening (Plummer's, at ε = 0.02) is not a fudge but the physics of
/// a point mass that is really a cloud: without it a close pass gives
/// an infinite force and one body leaves at the speed of arithmetic.
pub struct Orbits {
    pos: Vec<[f32; 3]>,
    vel: Vec<[f32; 3]>,
    acc: Vec<[f32; 3]>,
    arc: Vec<[f32; 3]>,
    head: usize,
    since_kick: f32,
    since_snare: f32,
    rng: Rng,
}

impl Orbits {
    /// The centre's pull, as GM: chosen so a body halfway out goes
    /// round in about five seconds — a tempo, not a year.
    const GM: f32 = 0.2;
    /// One body's pull, as Gm. Five hundred of them come to about a
    /// sixth of the centre, which is roughly a disc galaxy's share:
    /// enough for the disc to clump into arms, not enough for it to
    /// collapse on itself.
    const GBODY: f32 = 3.0e-4;
    const SOFTENING: f32 = 0.02;

    pub fn new() -> Self {
        debug_assert_eq!(BODIES * ARC, POINTS);
        let mut rng = Rng::new(0x0B17_0B17);
        let mut pos = Vec::with_capacity(BODIES);
        let mut vel = Vec::with_capacity(BODIES);
        for _ in 0..BODIES {
            // Radii spread over the disc, thin in the third direction.
            let r = 0.18 + 0.62 * rng.f32().sqrt();
            let a = rng.f32() * std::f32::consts::TAU;
            let z = (rng.f32() - 0.5) * 0.06;
            pos.push([r * a.cos(), z, r * a.sin()]);
            // The circular speed at that radius, give or take: v = √(GM/r).
            let v = (Self::GM / r).sqrt() * (0.92 + 0.16 * rng.f32());
            vel.push([-v * a.sin(), 0.0, v * a.cos()]);
        }
        let arc = pos.iter().flat_map(|p| std::iter::repeat_n(*p, ARC)).collect();
        Self {
            pos,
            vel,
            acc: vec![[0.0; 3]; BODIES],
            arc,
            head: 0,
            since_kick: 10.0,
            since_snare: 10.0,
            rng,
        }
    }
}

impl Default for Orbits {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Orbits {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.since_kick += dt;
        self.since_snare += dt;
        // Loudness is the clock: a quiet passage runs slow, a loud one
        // winds the whole system forward.
        let dt = dt * if drive.audio { 0.6 + 1.6 * drive.level } else { 1.0 };
        let eps2 = Self::SOFTENING * Self::SOFTENING;
        for (a, p) in self.acc.iter_mut().zip(&self.pos) {
            // The centre first, then everyone else.
            let r2 = p[0] * p[0] + p[1] * p[1] + p[2] * p[2] + eps2;
            let pull = -Self::GM / (r2 * r2.sqrt());
            *a = [p[0] * pull, p[1] * pull, p[2] * pull];
        }
        for i in 0..BODIES {
            let pi = self.pos[i];
            let mut ai = [0.0f32; 3];
            for j in (i + 1)..BODIES {
                let pj = self.pos[j];
                let d = [pj[0] - pi[0], pj[1] - pi[1], pj[2] - pi[2]];
                let r2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2] + eps2;
                let g = Self::GBODY / (r2 * r2.sqrt());
                // Newton's third law, so half the pairs do all the work.
                for ((a, b), d) in ai.iter_mut().zip(self.acc[j].iter_mut()).zip(d) {
                    *a += d * g;
                    *b -= d * g;
                }
            }
            for (a, add) in self.acc[i].iter_mut().zip(ai) {
                *a += add;
            }
        }
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.4;
        if kick {
            self.since_kick = 0.0;
        }
        let snare = drive.audio && drive.bands[1] > 0.5 && self.since_snare > 0.4;
        if snare {
            self.since_snare = 0.0;
        }
        for (p, (v, a)) in self.pos.iter_mut().zip(self.vel.iter_mut().zip(&self.acc)) {
            for (v, a) in v.iter_mut().zip(a) {
                *v += a * dt;
            }
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-4);
            if kick {
                // A shockwave out of the centre: the disc puffs into a
                // shell and settles back over the next few bars.
                for (v, p) in v.iter_mut().zip(p.iter()) {
                    *v += p / r * 0.06;
                }
            }
            if snare {
                // A shear, along one axis drawn at random: the disc is
                // knocked out of its plane.
                let d = self.rng.on_sphere();
                for (v, d) in v.iter_mut().zip(d) {
                    *v += d * 0.02;
                }
            }
            // The bowl. Nothing outside the box can be seen, and one
            // body thrown out by a close pass would otherwise take the
            // cloud's whole scale with it; past five sixths of the way
            // out, gravity is joined by a stiff spring.
            if r > 0.85 {
                let pull = (r - 0.85) * 30.0;
                for (v, p) in v.iter_mut().zip(p.iter()) {
                    *v -= p / r * pull * dt;
                }
            }
            for (p, v) in p.iter_mut().zip(v.iter()) {
                *p += v * dt;
            }
            // And a wall behind the spring, for the one body that comes
            // out of a close pass fast enough to climb it: put back on
            // the sphere, with whatever speed was taking it outward
            // taken away.
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            if r > 1.0 {
                let radial = (0..3).map(|k| v[k] * p[k] / r).sum::<f32>();
                for (v, p) in v.iter_mut().zip(p.iter()) {
                    *v -= p / r * radial.max(0.0);
                }
                for p in p.iter_mut() {
                    *p /= r;
                }
            }
        }
        self.head = (self.head + 1) % ARC;
        for (i, p) in self.pos.iter().enumerate() {
            self.arc[i * ARC + self.head] = *p;
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for i in 0..BODIES {
            for age in 0..ARC {
                let slot = (self.head + ARC - age) % ARC;
                let p = self.arc[i * ARC + slot];
                let fade = 1.0 - age as f32 / ARC as f32 * 0.9;
                let c = (255.0 * fade) as u8;
                out.push(Point {
                    pos: [p[0].clamp(-1.0, 1.0), p[1].clamp(-1.0, 1.0), p[2].clamp(-1.0, 1.0)],
                    normal: [0.0; 3],
                    color: [c, c, c],
                });
            }
        }
    }
}

// --- Pendulum ---------------------------------------------------------

/// Pendulums across and along the sheet, and points strewn down each
/// one's two arms. Their product is a slot.
const PEND_ACROSS: usize = 64;
const PEND_ALONG: usize = 64;
const PEND_BEADS: usize = 16;

/// A curtain of double pendulums, each started a hair away from its
/// neighbour.
///
/// One double pendulum is the standard demonstration that a system with
/// four numbers in it can be unpredictable. Four thousand of them, hung
/// in a sheet from initial angles that differ in the fourth decimal
/// place, are the demonstration of *why*: for the first second they
/// swing as one surface, then the surface creases, then it tears, and
/// within ten seconds two pendulums that started indistinguishable are
/// pointing opposite ways. Nothing is random here — the same start
/// gives the same tearing every time.
///
/// The equations are the textbook ones for equal masses and equal arms,
/// integrated by Runge–Kutta in two sub-steps, because a double
/// pendulum integrated carelessly gains energy and ends up spinning.
pub struct Pendulum {
    /// θ₁, θ₂, ω₁, ω₂ per pendulum.
    state: Vec<[f32; 4]>,
    since_kick: f32,
    since_fold: f32,
    rng: Rng,
}

impl Pendulum {
    const COUNT: usize = PEND_ACROSS * PEND_ALONG;
    /// How wide a patch of starting angles the sheet covers. Small, so
    /// the sheet is smooth to begin with and the tearing is the news.
    const SPREAD: f32 = 0.35;

    pub fn new() -> Self {
        debug_assert_eq!(Self::COUNT * PEND_BEADS, POINTS);
        let mut p = Self {
            state: vec![[0.0; 4]; Self::COUNT],
            since_kick: 10.0,
            since_fold: 0.0,
            rng: Rng::new(0x9E4D_0105),
        };
        p.fold(2.0, 2.4);
        p
    }

    /// Hang the sheet again from a fresh patch of angles.
    fn fold(&mut self, centre1: f32, centre2: f32) {
        for row in 0..PEND_ALONG {
            for col in 0..PEND_ACROSS {
                let u = col as f32 / (PEND_ACROSS - 1) as f32 - 0.5;
                let v = row as f32 / (PEND_ALONG - 1) as f32 - 0.5;
                self.state[row * PEND_ACROSS + col] = [
                    centre1 + Self::SPREAD * u,
                    centre2 + Self::SPREAD * v,
                    0.0,
                    0.0,
                ];
            }
        }
        self.since_fold = 0.0;
    }

    /// The angular accelerations, for equal masses and equal arms of
    /// one: the standard Lagrangian result, with `g` left free so the
    /// music can lean on it.
    fn rates([t1, t2, w1, w2]: [f32; 4], g: f32) -> [f32; 4] {
        let d = t1 - t2;
        let (sin_d, cos_d) = d.sin_cos();
        // 2m₁ + m₂ − m₂cos(2θ₁ − 2θ₂), with the masses at one.
        let den = 3.0 - (2.0 * d).cos();
        let a1 = (-3.0 * g * t1.sin() - g * (t1 - 2.0 * t2).sin()
            - 2.0 * sin_d * (w2 * w2 + w1 * w1 * cos_d))
            / den;
        let a2 = (2.0 * sin_d * (2.0 * w1 * w1 + 2.0 * g * t1.cos() + w2 * w2 * cos_d)) / den;
        [w1, w2, a1, a2]
    }

    fn advance(s: [f32; 4], dt: f32, g: f32) -> [f32; 4] {
        let add = |a: [f32; 4], b: [f32; 4], k: f32| std::array::from_fn(|i| a[i] + b[i] * k);
        let k1 = Self::rates(s, g);
        let k2 = Self::rates(add(s, k1, dt * 0.5), g);
        let k3 = Self::rates(add(s, k2, dt * 0.5), g);
        let k4 = Self::rates(add(s, k3, dt), g);
        std::array::from_fn(|i| s[i] + dt / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]))
    }
}

impl Default for Pendulum {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Pendulum {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.since_kick += dt;
        self.since_fold += dt;
        // Gravity is the loudness: a quiet passage is the moon, a loud
        // one is a heavy planet and the curtain tears in seconds.
        let g = if drive.audio { 6.0 + 14.0 * drive.level } else { 9.81 };
        // Once the sheet has torn there is nothing left to watch tear,
        // so a kick hangs it again — from a new place, so the same
        // tearing does not repeat.
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.4;
        if kick {
            self.since_kick = 0.0;
        }
        if self.since_fold > 30.0 || (kick && self.since_fold > 8.0) {
            let (a, b) = (self.rng.f32(), self.rng.f32());
            self.fold(1.2 + 1.6 * a, 1.2 + 1.6 * b);
            return;
        }
        const SUBSTEPS: usize = 2;
        let h = dt / SUBSTEPS as f32;
        for s in &mut self.state {
            for _ in 0..SUBSTEPS {
                *s = Self::advance(*s, h, g);
            }
            // The angles are free to wind round, and after a minute
            // they are large enough that the sines lose their last
            // digits. Bring them back without changing the state.
            for a in &mut s[..2] {
                *a = a.rem_euclid(std::f32::consts::TAU);
            }
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        // Two arms of one, so the tip is two from the pivot: a half
        // fits the whole linkage in the box with the pivot at the top.
        const SCALE: f32 = 0.5;
        for row in 0..PEND_ALONG {
            // The sheet's own axis: fixed, so the curtain hangs in
            // depth and the tearing is across it.
            let z = (row as f32 / (PEND_ALONG - 1) as f32 - 0.5) * 2.0;
            for col in 0..PEND_ACROSS {
                let [t1, t2, _, w2] = self.state[row * PEND_ACROSS + col];
                let (s1, c1) = t1.sin_cos();
                let (s2, c2) = t2.sin_cos();
                let elbow = [s1, -c1];
                // Bright where it is moving fastest, so the tear front
                // lights up as it passes.
                let shade = (96.0 + 159.0 * (w2.abs() / 12.0).min(1.0)) as u8;
                for bead in 0..PEND_BEADS {
                    let t = (bead as f32 + 0.5) / PEND_BEADS as f32 * 2.0;
                    let arm = if t <= 1.0 {
                        [elbow[0] * t, elbow[1] * t]
                    } else {
                        [elbow[0] + s2 * (t - 1.0), elbow[1] - c2 * (t - 1.0)]
                    };
                    out.push(Point {
                        pos: [
                            (arm[0] * SCALE).clamp(-1.0, 1.0),
                            (arm[1] * SCALE + 0.5).clamp(-1.0, 1.0),
                            z,
                        ],
                        normal: [0.0; 3],
                        color: [shade, shade, shade],
                    });
                }
            }
        }
    }
}

// --- Smoke ------------------------------------------------------------

/// Cells along each side of the smoke box, a power of two so the
/// periodic wrap is a mask.
const SG: usize = 32;
const SMASK: usize = SG - 1;
const SPLANE: usize = SG * SG;
const SCELLS: usize = SG * SG * SG;

/// The eight cells around a point, and where in them it falls. Worked
/// out once and then used to interpolate several fields at the same
/// place: advection wants four there, a tracer wants three, and doing
/// the index arithmetic once instead of four times is most of what
/// makes a three-dimensional solver fit in a frame at all.
struct Corners {
    idx: [usize; 8],
    fx: f32,
    fy: f32,
    fz: f32,
}

/// Stam's solver again, in three dimensions, with heat.
///
/// The two-dimensional fluid is a sheet seen from above; this is a box
/// seen from anywhere, and the difference is not only a coordinate. A
/// cube of cells costs thirty-two times what a square of the same side
/// costs, so the grid has to be coarse — thirty-two cells across, where
/// the sheet has a hundred and twenty-eight. That is fine, and the
/// reason is the cloud: sixty-five thousand tracers in thirty-two
/// thousand cells means two particles per cell carrying detail the grid
/// never had, and the filaments they draw are finer than the velocity
/// field drawing them.
///
/// Heat is what makes it smoke rather than stirred dust. A Boussinesq
/// buoyancy — a force proportional to how much hotter a cell is than
/// the box's average, so the net force is zero and a periodic box does
/// not accelerate away — lifts the hot cells, the lift shears, the
/// shear rolls up, and what rises is a plume with a mushroom on it.
pub struct Smoke {
    u: Vec<f32>,
    v: Vec<f32>,
    w: Vec<f32>,
    u0: Vec<f32>,
    v0: Vec<f32>,
    w0: Vec<f32>,
    heat: Vec<f32>,
    heat0: Vec<f32>,
    p: Vec<f32>,
    div: Vec<f32>,
    /// The curl, one component per axis, and its size — wanted by the
    /// confinement, which needs the size at the neighbours too.
    cu: Vec<f32>,
    cv: Vec<f32>,
    cw: Vec<f32>,
    mag: Vec<f32>,
    tracers: Vec<[f32; 3]>,
    time: f32,
    since_kick: f32,
    since_snare: f32,
    rng: Rng,
}

impl Smoke {
    /// Sweeps of Gauss–Seidel in the pressure solve. Twelve, not the
    /// sheet's twenty: a cube has thirty-two times the cells, and by
    /// twelve the swirls have stopped changing at this resolution.
    const SWEEPS: usize = 12;

    pub fn new() -> Self {
        let mut rng = Rng::new(0x5_0C01);
        // Tracers on a jittered lattice: 41³ is a little over a slot,
        // so the box starts evenly full.
        let side = 41;
        let mut tracers = Vec::with_capacity(POINTS);
        'fill: for k in 0..side {
            for j in 0..side {
                for i in 0..side {
                    if tracers.len() == POINTS {
                        break 'fill;
                    }
                    tracers.push([
                        (i as f32 + rng.f32()) / side as f32,
                        (j as f32 + rng.f32()) / side as f32,
                        (k as f32 + rng.f32()) / side as f32,
                    ]);
                }
            }
        }
        Self {
            u: vec![0.0; SCELLS],
            v: vec![0.0; SCELLS],
            w: vec![0.0; SCELLS],
            u0: vec![0.0; SCELLS],
            v0: vec![0.0; SCELLS],
            w0: vec![0.0; SCELLS],
            heat: vec![0.0; SCELLS],
            heat0: vec![0.0; SCELLS],
            p: vec![0.0; SCELLS],
            div: vec![0.0; SCELLS],
            cu: vec![0.0; SCELLS],
            cv: vec![0.0; SCELLS],
            cw: vec![0.0; SCELLS],
            mag: vec![0.0; SCELLS],
            tracers,
            time: 0.0,
            since_kick: 10.0,
            since_snare: 10.0,
            rng,
        }
    }

    fn at(i: usize, j: usize, k: usize) -> usize {
        (i & SMASK) + (j & SMASK) * SG + (k & SMASK) * SPLANE
    }

    /// Where a position in cell units lands, wrapped.
    fn corners(x: f32, y: f32, z: f32) -> Corners {
        let x = x.rem_euclid(SG as f32);
        let y = y.rem_euclid(SG as f32);
        let z = z.rem_euclid(SG as f32);
        let (i0, j0, k0) = (x as usize & SMASK, y as usize & SMASK, z as usize & SMASK);
        let (i1, j1, k1) = ((i0 + 1) & SMASK, (j0 + 1) & SMASK, (k0 + 1) & SMASK);
        let (j0, j1) = (j0 * SG, j1 * SG);
        let (k0p, k1p) = (k0 * SPLANE, k1 * SPLANE);
        Corners {
            idx: [
                i0 + j0 + k0p,
                i1 + j0 + k0p,
                i0 + j1 + k0p,
                i1 + j1 + k0p,
                i0 + j0 + k1p,
                i1 + j0 + k1p,
                i0 + j1 + k1p,
                i1 + j1 + k1p,
            ],
            fx: x.fract(),
            fy: y.fract(),
            fz: z.fract(),
        }
    }

    /// One field, trilinearly, at a place already worked out.
    fn trilerp(field: &[f32], c: &Corners) -> f32 {
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let a = lerp(
            lerp(field[c.idx[0]], field[c.idx[1]], c.fx),
            lerp(field[c.idx[2]], field[c.idx[3]], c.fx),
            c.fy,
        );
        let b = lerp(
            lerp(field[c.idx[4]], field[c.idx[5]], c.fx),
            lerp(field[c.idx[6]], field[c.idx[7]], c.fx),
            c.fy,
        );
        lerp(a, b, c.fz)
    }

    /// Walk only the cells a source can reach, wrapping, handing each
    /// one its Gaussian weight. A source has a reach of a few cells and
    /// the box has thirty-two thousand, so this is the difference
    /// between a hundred exponentials a frame and a hundred thousand.
    fn near(centre: [f32; 3], sigma: f32, mut f: impl FnMut(usize, f32)) {
        // Three standard deviations; past that the weight is a
        // thousandth and the cell may as well be cold.
        let reach = 3.0 * sigma;
        let span = (reach * SG as f32).ceil() as isize;
        let c = [centre[0] * SG as f32, centre[1] * SG as f32, centre[2] * SG as f32];
        let base = [c[0].floor() as isize, c[1].floor() as isize, c[2].floor() as isize];
        let inv = 1.0 / (sigma * sigma * SG as f32 * SG as f32);
        for dk in -span..=span {
            for dj in -span..=span {
                for di in -span..=span {
                    let cell = [base[0] + di, base[1] + dj, base[2] + dk];
                    let d: [f32; 3] =
                        std::array::from_fn(|n| cell[n] as f32 + 0.5 - c[n]);
                    let r2 = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) * inv;
                    if r2 > 9.0 {
                        continue;
                    }
                    let idx = Self::at(
                        cell[0].rem_euclid(SG as isize) as usize,
                        cell[1].rem_euclid(SG as isize) as usize,
                        cell[2].rem_euclid(SG as isize) as usize,
                    );
                    f(idx, (-r2).exp());
                }
            }
        }
    }

    /// Heat in, heat out, and what the heat does. Without audio a
    /// single vent wanders the floor; with it, the kick is a blast, the
    /// snare a shove sideways, the loudness the buoyancy.
    fn add_forces(&mut self, dt: f32, drive: &Drive) {
        let t = self.time;
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.25;
        if kick {
            self.since_kick = 0.0;
        }
        let snare = drive.audio && drive.bands[2] > 0.5 && self.since_snare > 0.25;
        if snare {
            self.since_snare = 0.0;
        }
        // The standing vent, on its own slow path across the floor.
        let vent = [0.5 + 0.25 * (t * 0.19).cos(), 0.12, 0.5 + 0.25 * (t * 0.23).sin()];
        let heat = &mut self.heat;
        Self::near(vent, 0.05, |idx, g| heat[idx] += g * 6.0 * dt);
        if kick {
            // A blast: hot, and somewhere else.
            let at = [self.rng.f32(), 0.12, self.rng.f32()];
            let amount = 18.0 * drive.bands[0] * dt;
            let heat = &mut self.heat;
            Self::near(at, 0.06, |idx, g| heat[idx] += g * amount);
        }
        if snare {
            // A shove, along an axis drawn at random.
            let at = [self.rng.f32(), self.rng.f32(), self.rng.f32()];
            let d = self.rng.on_sphere();
            let push = [d[0] * 6.0 * dt, d[1] * 2.0 * dt, d[2] * 6.0 * dt];
            let (u, v, w) = (&mut self.u, &mut self.v, &mut self.w);
            Self::near(at, 0.08, |idx, g| {
                u[idx] += push[0] * g;
                v[idx] += push[1] * g;
                w[idx] += push[2] * g;
            });
        }
        // Cooling, and the average buoyancy is measured against: a
        // periodic box has no sky to lose heat to, so it is cooled by
        // hand, and lifted only by the part of the heat above the
        // average, which keeps the net force at zero.
        let cool = (1.0 - 0.35 * dt).clamp(0.0, 1.0);
        let mut total = 0.0;
        for h in &mut self.heat {
            *h = (*h * cool).min(4.0);
            total += *h;
        }
        let mean = total / SCELLS as f32;
        let lift = (if drive.audio { 4.0 + 14.0 * drive.level } else { 8.0 }) * dt;
        for (v, h) in self.v.iter_mut().zip(&self.heat) {
            *v += (h - mean) * lift;
        }
    }

    /// Vorticity confinement in three dimensions (Fedkiw, Stam &
    /// Jensen, 2001): the curl is a vector here rather than a number,
    /// and the force is ε h (N × ω), with N the direction in which the
    /// vorticity's size grows.
    fn confine(&mut self, dt: f32, epsilon: f32) {
        let h = 1.0 / SG as f32;
        for k in 0..SG {
            let (kb, kp, km) = (k * SPLANE, ((k + 1) & SMASK) * SPLANE, ((k + SG - 1) & SMASK) * SPLANE);
            for j in 0..SG {
                let (jb, jp, jm) = (j * SG, ((j + 1) & SMASK) * SG, ((j + SG - 1) & SMASK) * SG);
                let row = kb + jb;
                for i in 0..SG {
                    let (ip, im) = ((i + 1) & SMASK, (i + SG - 1) & SMASK);
                    let idx = row + i;
                    let cu = 0.5 * ((self.w[kb + jp + i] - self.w[kb + jm + i])
                        - (self.v[kp + jb + i] - self.v[km + jb + i]))
                        / h;
                    let cv = 0.5 * ((self.u[kp + jb + i] - self.u[km + jb + i])
                        - (self.w[row + ip] - self.w[row + im]))
                        / h;
                    let cw = 0.5 * ((self.v[row + ip] - self.v[row + im])
                        - (self.u[kb + jp + i] - self.u[kb + jm + i]))
                        / h;
                    self.cu[idx] = cu;
                    self.cv[idx] = cv;
                    self.cw[idx] = cw;
                    self.mag[idx] = (cu * cu + cv * cv + cw * cw).sqrt();
                }
            }
        }
        let scale = epsilon * h * dt;
        for k in 0..SG {
            let (kb, kp, km) = (k * SPLANE, ((k + 1) & SMASK) * SPLANE, ((k + SG - 1) & SMASK) * SPLANE);
            for j in 0..SG {
                let (jb, jp, jm) = (j * SG, ((j + 1) & SMASK) * SG, ((j + SG - 1) & SMASK) * SG);
                let row = kb + jb;
                for i in 0..SG {
                    let (ip, im) = ((i + 1) & SMASK, (i + SG - 1) & SMASK);
                    let idx = row + i;
                    let n = [
                        self.mag[row + ip] - self.mag[row + im],
                        self.mag[kb + jp + i] - self.mag[kb + jm + i],
                        self.mag[kp + jb + i] - self.mag[km + jb + i],
                    ];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    if len < 1e-5 {
                        continue;
                    }
                    let n = [n[0] / len, n[1] / len, n[2] / len];
                    let o = [self.cu[idx], self.cv[idx], self.cw[idx]];
                    self.u[idx] += scale * (n[1] * o[2] - n[2] * o[1]);
                    self.v[idx] += scale * (n[2] * o[0] - n[0] * o[2]);
                    self.w[idx] += scale * (n[0] * o[1] - n[1] * o[0]);
                }
            }
        }
    }

    /// Make the field divergence-free: solve for a pressure whose
    /// gradient cancels the divergence, Gauss–Seidel, and subtract it.
    fn project(&mut self) {
        let h = 1.0 / SG as f32;
        for k in 0..SG {
            let (kb, kp, km) = (k * SPLANE, ((k + 1) & SMASK) * SPLANE, ((k + SG - 1) & SMASK) * SPLANE);
            for j in 0..SG {
                let (jb, jp, jm) = (j * SG, ((j + 1) & SMASK) * SG, ((j + SG - 1) & SMASK) * SG);
                let row = kb + jb;
                for i in 0..SG {
                    let (ip, im) = ((i + 1) & SMASK, (i + SG - 1) & SMASK);
                    let idx = row + i;
                    self.div[idx] = -0.5
                        * h
                        * (self.u[row + ip] - self.u[row + im] + self.v[kb + jp + i]
                            - self.v[kb + jm + i]
                            + self.w[kp + jb + i]
                            - self.w[km + jb + i]);
                    self.p[idx] = 0.0;
                }
            }
        }
        for _ in 0..Self::SWEEPS {
            for k in 0..SG {
                let (kb, kp, km) = (k * SPLANE, ((k + 1) & SMASK) * SPLANE, ((k + SG - 1) & SMASK) * SPLANE);
                for j in 0..SG {
                    let (jb, jp, jm) = (j * SG, ((j + 1) & SMASK) * SG, ((j + SG - 1) & SMASK) * SG);
                    let row = kb + jb;
                    // The run along x, split so the inside of the row
                    // needs no wrapping at all: this is the innermost
                    // loop of the whole solver, run twelve times a
                    // frame over every cell in the box, and the two
                    // masks it saves are worth the two lines it costs.
                    for i in 1..SG - 1 {
                        let idx = row + i;
                        self.p[idx] = (self.div[idx]
                            + self.p[idx + 1]
                            + self.p[idx - 1]
                            + self.p[kb + jp + i]
                            + self.p[kb + jm + i]
                            + self.p[kp + jb + i]
                            + self.p[km + jb + i])
                            / 6.0;
                    }
                    for i in [0, SG - 1] {
                        let idx = row + i;
                        self.p[idx] = (self.div[idx]
                            + self.p[row + ((i + 1) & SMASK)]
                            + self.p[row + ((i + SG - 1) & SMASK)]
                            + self.p[kb + jp + i]
                            + self.p[kb + jm + i]
                            + self.p[kp + jb + i]
                            + self.p[km + jb + i])
                            / 6.0;
                    }
                }
            }
        }
        for k in 0..SG {
            let (kb, kp, km) = (k * SPLANE, ((k + 1) & SMASK) * SPLANE, ((k + SG - 1) & SMASK) * SPLANE);
            for j in 0..SG {
                let (jb, jp, jm) = (j * SG, ((j + 1) & SMASK) * SG, ((j + SG - 1) & SMASK) * SG);
                let row = kb + jb;
                for i in 0..SG {
                    let (ip, im) = ((i + 1) & SMASK, (i + SG - 1) & SMASK);
                    let idx = row + i;
                    self.u[idx] -= 0.5 * (self.p[row + ip] - self.p[row + im]) / h;
                    self.v[idx] -= 0.5 * (self.p[kb + jp + i] - self.p[kb + jm + i]) / h;
                    self.w[idx] -= 0.5 * (self.p[kp + jb + i] - self.p[km + jb + i]) / h;
                }
            }
        }
    }

    /// Semi-Lagrangian advection of the velocity and the heat together:
    /// each cell takes what was where its fluid came from, all four
    /// fields off one set of corners.
    fn advect(&mut self, dt: f32) {
        let scale = dt * SG as f32;
        std::mem::swap(&mut self.u, &mut self.u0);
        std::mem::swap(&mut self.v, &mut self.v0);
        std::mem::swap(&mut self.w, &mut self.w0);
        std::mem::swap(&mut self.heat, &mut self.heat0);
        for k in 0..SG {
            for j in 0..SG {
                let row = k * SPLANE + j * SG;
                for i in 0..SG {
                    let idx = row + i;
                    let c = Self::corners(
                        i as f32 - self.u0[idx] * scale,
                        j as f32 - self.v0[idx] * scale,
                        k as f32 - self.w0[idx] * scale,
                    );
                    self.u[idx] = Self::trilerp(&self.u0, &c);
                    self.v[idx] = Self::trilerp(&self.v0, &c);
                    self.w[idx] = Self::trilerp(&self.w0, &c);
                    self.heat[idx] = Self::trilerp(&self.heat0, &c);
                }
            }
        }
    }

}

impl Default for Smoke {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Smoke {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        self.since_snare += dt;
        self.add_forces(dt, drive);
        let roughness = if drive.audio { 6.0 + 18.0 * drive.bands[3] } else { 12.0 };
        self.confine(dt, roughness);
        // Damping and a ceiling, for the reasons the sheet has them: a
        // box with no walls loses no energy on its own, and a burst on
        // a hot input must not throw a tracer clear across it in one
        // frame.
        for ((u, v), w) in self.u.iter_mut().zip(self.v.iter_mut()).zip(self.w.iter_mut()) {
            *u = (*u * 0.994).clamp(-3.0, 3.0);
            *v = (*v * 0.994).clamp(-3.0, 3.0);
            *w = (*w * 0.994).clamp(-3.0, 3.0);
        }
        self.project();
        self.advect(dt);
        self.project();
        for t in &mut self.tracers {
            let p = *t;
            let c = Self::corners(
                p[0] * SG as f32 - 0.5,
                p[1] * SG as f32 - 0.5,
                p[2] * SG as f32 - 0.5,
            );
            // Midpoint, as the sheet's tracers use: one extra look-up
            // per particle, and a tracer that goes round an eddy rather
            // than spiralling out of it.
            let k1 = [
                Self::trilerp(&self.u, &c),
                Self::trilerp(&self.v, &c),
                Self::trilerp(&self.w, &c),
            ];
            let mid = [
                p[0] + 0.5 * dt * k1[0],
                p[1] + 0.5 * dt * k1[1],
                p[2] + 0.5 * dt * k1[2],
            ];
            let c = Self::corners(
                mid[0] * SG as f32 - 0.5,
                mid[1] * SG as f32 - 0.5,
                mid[2] * SG as f32 - 0.5,
            );
            *t = [
                (p[0] + dt * Self::trilerp(&self.u, &c)).rem_euclid(1.0),
                (p[1] + dt * Self::trilerp(&self.v, &c)).rem_euclid(1.0),
                (p[2] + dt * Self::trilerp(&self.w, &c)).rem_euclid(1.0),
            ];
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for p in &self.tracers {
            // Bright where it is hot: the plume glows and the still air
            // round it is nearly dark, which is what tells one from the
            // other in a box evenly full of tracers.
            let c = Self::corners(
                p[0] * SG as f32 - 0.5,
                p[1] * SG as f32 - 0.5,
                p[2] * SG as f32 - 0.5,
            );
            let h = Self::trilerp(&self.heat, &c);
            let bright = (0.22 + 0.78 * (h * 1.6).min(1.0)).clamp(0.0, 1.0);
            let shade = (bright * 255.0) as u8;
            out.push(Point {
                pos: [(p[0] - 0.5) * 2.0, (p[1] - 0.5) * 2.0, (p[2] - 0.5) * 2.0],
                normal: [0.0; 3],
                color: [shade, shade, shade],
            });
        }
    }
}

// --- Liquid -----------------------------------------------------------

/// Particles of liquid, and the points each one is drawn with. Their
/// product is a slot.
const DROPS: usize = 4096;
const BLOB: usize = 16;
/// The smoothing length: the radius within which particles feel each
/// other. Chosen so a packed particle has about thirty-five neighbours,
/// which is what the kernels below are calibrated for.
const SMOOTH: f32 = 0.1;
/// Cells along each side of the neighbour grid. A cell is exactly one
/// smoothing length, so every neighbour is in the twenty-seven cells
/// around a particle's own.
const LG_SIDE: usize = 10;
/// The most neighbours one particle will consider. A crowd past this is
/// already denser than the solver is trying to allow.
const NEIGHBOURS: usize = 64;

/// Smoothed-particle hydrodynamics, solved by position-based fluids
/// (Macklin & Müller, 2013).
///
/// The other fluids here are fields on a grid; this one is the liquid
/// itself, four thousand particles that carry their own mass about and
/// are told only that they must not crowd. Each particle is drawn as a
/// small cluster of points, so a slot's worth of cloud shows four
/// thousand particles as blobs rather than as dust.
///
/// Why position-based rather than the textbook weakly-compressible
/// form: the textbook form is a spring system, and a spring stiff
/// enough to look like water needs a time step far finer than a frame —
/// a sixtieth of a second would blow it apart in the first bar. The
/// position-based solver works on the positions directly, projecting
/// them a few times a frame onto the constraint that the density is
/// right, and is stable at any step at the price of looking slightly
/// soft. On a screen behind a band, soft is the correct trade.
///
/// The box tilts: with audio, gravity swings round once a bar, so the
/// liquid pours from corner to corner in time. The kick throws it at
/// the ceiling.
pub struct Liquid {
    pos: Vec<[f32; 3]>,
    vel: Vec<[f32; 3]>,
    /// Where the particle would be with no neighbours — the prediction
    /// the solver then pushes back into place.
    guess: Vec<[f32; 3]>,
    lambda: Vec<f32>,
    delta: Vec<[f32; 3]>,
    /// Neighbour lists, found once a frame: `count[i]` entries starting
    /// at `i * NEIGHBOURS`.
    near: Vec<u32>,
    count: Vec<u8>,
    cells: Vec<Vec<u32>>,
    /// A fixed cluster of offsets per particle, so a blob does not
    /// shimmer from frame to frame.
    blob: Vec<[f32; 3]>,
    /// The density a settled liquid has, measured once at rest.
    rest: f32,
    mass: f32,
    time: f32,
    since_kick: f32,
    since_snare: f32,
    rng: Rng,
}

impl Liquid {
    /// Solver passes per frame. Three is where a dropped column stops
    /// visibly compressing.
    const PASSES: usize = 3;
    /// The floor under the constraint's denominator, so an isolated
    /// particle divides by something.
    const RELAX: f32 = 1.0e-3;
    /// Artificial pressure, against the clumping that a plain density
    /// constraint gives at a free surface: a small push apart that
    /// stands in for surface tension.
    const TENSILE: f32 = 1.0e-4;

    pub fn new() -> Self {
        debug_assert_eq!(DROPS * BLOB, POINTS);
        let mut rng = Rng::new(0x11_9C1D);
        // A block filling the lower half, on a jittered lattice so it
        // starts packed rather than clumped.
        let side = 16;
        let mut pos = Vec::with_capacity(DROPS);
        'fill: for k in 0..side {
            for j in 0..side {
                for i in 0..side {
                    if pos.len() == DROPS {
                        break 'fill;
                    }
                    pos.push([
                        0.1 + 0.8 * (i as f32 + 0.2 * rng.f32()) / side as f32,
                        0.05 + 0.45 * (j as f32 + 0.2 * rng.f32()) / side as f32,
                        0.1 + 0.8 * (k as f32 + 0.2 * rng.f32()) / side as f32,
                    ]);
                }
            }
        }
        let blob = (0..DROPS * BLOB)
            .map(|_| {
                let d = rng.on_sphere();
                let r = SMOOTH * 0.34 * rng.f32().cbrt();
                [d[0] * r, d[1] * r, d[2] * r]
            })
            .collect();
        let mut l = Self {
            vel: vec![[0.0; 3]; DROPS],
            guess: pos.clone(),
            lambda: vec![0.0; DROPS],
            delta: vec![[0.0; 3]; DROPS],
            near: vec![0; DROPS * NEIGHBOURS],
            count: vec![0; DROPS],
            cells: vec![Vec::new(); LG_SIDE * LG_SIDE * LG_SIDE],
            pos,
            blob,
            rest: 1.0,
            mass: 1.0,
            time: 0.0,
            since_kick: 10.0,
            since_snare: 10.0,
            rng,
        };
        // The rest density is whatever this packing gives, measured
        // with unit mass and then folded into the mass so the settled
        // liquid sits at a density of one. That keeps every number in
        // the solver near unity whatever the smoothing length is.
        l.find_neighbours();
        let raw = l.densities();
        let mut sorted = raw;
        sorted.sort_by(f32::total_cmp);
        let median = sorted[DROPS / 2].max(1e-6);
        l.mass = 1.0 / median;
        l.rest = 1.0;
        l
    }

    /// Poly6, the density kernel (Müller, Charypar & Gross, 2003).
    fn poly6(r2: f32) -> f32 {
        const H2: f32 = SMOOTH * SMOOTH;
        if r2 >= H2 {
            return 0.0;
        }
        let d = H2 - r2;
        // 315 / (64 π h⁹), as a constant the compiler can fold.
        const K: f32 = 315.0 / (64.0 * std::f32::consts::PI * SMOOTH * SMOOTH * SMOOTH * SMOOTH * SMOOTH * SMOOTH * SMOOTH * SMOOTH * SMOOTH);
        K * d * d * d
    }

    /// The spiky gradient, which is what a pressure wants: it does not
    /// go flat at the centre the way poly6 does, so particles on top of
    /// each other still push apart.
    fn spiky(d: [f32; 3], r: f32) -> [f32; 3] {
        if !(1e-6..SMOOTH).contains(&r) {
            return [0.0; 3];
        }
        // -45 / (π h⁶)
        const K: f32 = -45.0 / (std::f32::consts::PI * SMOOTH * SMOOTH * SMOOTH * SMOOTH * SMOOTH * SMOOTH);
        let g = K * (SMOOTH - r) * (SMOOTH - r) / r;
        [d[0] * g, d[1] * g, d[2] * g]
    }

    fn cell_of(p: [f32; 3]) -> usize {
        let c: [usize; 3] =
            std::array::from_fn(|i| ((p[i] * LG_SIDE as f32) as usize).min(LG_SIDE - 1));
        c[0] + c[1] * LG_SIDE + c[2] * LG_SIDE * LG_SIDE
    }

    /// Bin the guessed positions and collect each particle's
    /// neighbours, once a frame — the solver then passes over the lists
    /// several times without touching the grid again.
    fn find_neighbours(&mut self) {
        for c in &mut self.cells {
            c.clear();
        }
        for (i, p) in self.guess.iter().enumerate() {
            self.cells[Self::cell_of(*p)].push(i as u32);
        }
        let h2 = SMOOTH * SMOOTH;
        for i in 0..DROPS {
            let p = self.guess[i];
            let base: [usize; 3] =
                std::array::from_fn(|k| ((p[k] * LG_SIDE as f32) as usize).min(LG_SIDE - 1));
            let mut found = 0usize;
            'cells: for dz in 0..3 {
                for dy in 0..3 {
                    for dx in 0..3 {
                        let c: [isize; 3] = [
                            base[0] as isize + dx as isize - 1,
                            base[1] as isize + dy as isize - 1,
                            base[2] as isize + dz as isize - 1,
                        ];
                        if c.iter().any(|v| *v < 0 || *v >= LG_SIDE as isize) {
                            continue;
                        }
                        let cell = c[0] as usize
                            + c[1] as usize * LG_SIDE
                            + c[2] as usize * LG_SIDE * LG_SIDE;
                        for j in &self.cells[cell] {
                            if *j as usize == i {
                                continue;
                            }
                            let q = self.guess[*j as usize];
                            let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < h2 {
                                self.near[i * NEIGHBOURS + found] = *j;
                                found += 1;
                                if found == NEIGHBOURS {
                                    break 'cells;
                                }
                            }
                        }
                    }
                }
            }
            self.count[i] = found as u8;
        }
    }

    /// Every particle's density, from the neighbour lists.
    fn densities(&self) -> Vec<f32> {
        (0..DROPS)
            .map(|i| {
                let p = self.guess[i];
                let mut rho = Self::poly6(0.0);
                for n in 0..self.count[i] as usize {
                    let q = self.guess[self.near[i * NEIGHBOURS + n] as usize];
                    let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                    rho += Self::poly6(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
                }
                rho * self.mass
            })
            .collect()
    }

    /// One projection onto "the density is right": work out each
    /// particle's multiplier, then the push it and its neighbours give
    /// each other.
    ///
    /// The constraint is C_i = ρ_i/ρ₀ − 1 and its gradient with respect
    /// to a particle is m∇W/ρ₀, so the mass belongs in both the
    /// multiplier and the push. Leaving it out of one of them is the
    /// classic way to get a solver that compiles, runs, and does
    /// nothing.
    fn solve(&mut self) {
        // The tensile correction is measured against the kernel at a
        // fixed fraction of the smoothing length.
        let w_dq = Self::poly6((0.2 * SMOOTH) * (0.2 * SMOOTH));
        let scale = self.mass / self.rest;
        for i in 0..DROPS {
            let p = self.guess[i];
            let mut rho = Self::poly6(0.0);
            let mut grad_self = [0.0f32; 3];
            let mut sum_sq = 0.0f32;
            for n in 0..self.count[i] as usize {
                let q = self.guess[self.near[i * NEIGHBOURS + n] as usize];
                let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                let r2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                rho += Self::poly6(r2);
                let g = Self::spiky(d, r2.sqrt()).map(|g| g * scale);
                for (a, g) in grad_self.iter_mut().zip(g) {
                    *a += g;
                }
                // The same gradient is the neighbour's own, negated,
                // and it is squared here either way.
                sum_sq += g[0] * g[0] + g[1] * g[1] + g[2] * g[2];
            }
            rho *= self.mass;
            sum_sq += grad_self[0] * grad_self[0]
                + grad_self[1] * grad_self[1]
                + grad_self[2] * grad_self[2];
            // Only crowding is corrected. A particle at a free surface
            // is short of neighbours by definition, and pulling it back
            // to the bulk density would suck the surface into clumps.
            let c = (rho / self.rest - 1.0).max(0.0);
            self.lambda[i] = -c / (sum_sq + Self::RELAX);
        }
        for i in 0..DROPS {
            let p = self.guess[i];
            let mut push = [0.0f32; 3];
            for n in 0..self.count[i] as usize {
                let j = self.near[i * NEIGHBOURS + n] as usize;
                let q = self.guess[j];
                let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                let r2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                let ratio = if w_dq > 0.0 { Self::poly6(r2) / w_dq } else { 0.0 };
                let corr = -Self::TENSILE * ratio * ratio * ratio * ratio;
                let g = Self::spiky(d, r2.sqrt());
                let k = (self.lambda[i] + self.lambda[j] + corr) * scale;
                for (a, g) in push.iter_mut().zip(g) {
                    *a += g * k;
                }
            }
            self.delta[i] = push;
        }
        for (g, d) in self.guess.iter_mut().zip(&self.delta) {
            for (g, d) in g.iter_mut().zip(d) {
                // A ceiling on one pass's correction: a particle that
                // somehow ends up inside another must not be fired out
                // of the box to fix it.
                *g += d.clamp(-0.02, 0.02);
            }
        }
        Self::contain(&mut self.guess);
    }

    /// The box. Positions are put back inside with a hair to spare, so
    /// a wall never holds two particles at exactly the same place.
    fn contain(pos: &mut [[f32; 3]]) {
        for p in pos.iter_mut() {
            for v in p.iter_mut() {
                *v = v.clamp(0.002, 0.998);
            }
        }
    }
}

impl Default for Liquid {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Liquid {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        self.since_snare += dt;
        // Gravity, tilted. With audio it swings once a bar, so the
        // liquid pours from corner to corner in time with the music;
        // without, it leans on a slow cycle of its own.
        let phase = if drive.audio { drive.bar * std::f32::consts::TAU } else { self.time * 0.5 };
        let tilt = if drive.audio { 0.35 + 0.25 * drive.level } else { 0.25 };
        let g = [
            tilt * phase.cos() * 2.0,
            -2.0,
            tilt * phase.sin() * 2.0,
        ];
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.3;
        if kick {
            self.since_kick = 0.0;
        }
        let snare = drive.audio && drive.bands[2] > 0.5 && self.since_snare > 0.3;
        if snare {
            self.since_snare = 0.0;
        }
        let gust = if snare {
            let d = self.rng.on_sphere();
            [d[0] * 0.5, d[1].abs() * 0.3, d[2] * 0.5]
        } else {
            [0.0; 3]
        };
        for (v, p) in self.vel.iter_mut().zip(&self.pos) {
            for (k, v) in v.iter_mut().enumerate() {
                *v += g[k] * dt + gust[k];
            }
            if kick {
                // A thump through the floor: the deeper the particle,
                // the harder it is hit, so the liquid leaves the bottom
                // as a sheet.
                *v = [v[0], v[1] + 1.4 * (1.0 - p[1]).clamp(0.0, 1.0), v[2]];
            }
        }
        for ((gu, p), v) in self.guess.iter_mut().zip(&self.pos).zip(&self.vel) {
            for ((g, p), v) in gu.iter_mut().zip(p).zip(v) {
                *g = p + v * dt;
            }
        }
        Self::contain(&mut self.guess);
        self.find_neighbours();
        for _ in 0..Self::PASSES {
            self.solve();
        }
        // The velocity is whatever the solver's answer implies, which is
        // what makes this stable: no force was ever integrated.
        for ((v, p), g) in self.vel.iter_mut().zip(&self.pos).zip(&self.guess) {
            for ((v, p), g) in v.iter_mut().zip(p).zip(g) {
                *v = ((g - p) / dt).clamp(-6.0, 6.0);
            }
        }
        // XSPH: each particle takes on a little of its neighbours'
        // velocity, which is the viscosity this solver has.
        for i in 0..DROPS {
            let p = self.guess[i];
            let mut blend = [0.0f32; 3];
            for n in 0..self.count[i] as usize {
                let j = self.near[i * NEIGHBOURS + n] as usize;
                let q = self.guess[j];
                let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                let w = Self::poly6(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) * self.mass;
                for (b, (a, c)) in blend.iter_mut().zip(self.vel[j].iter().zip(&self.vel[i])) {
                    *b += (a - c) * w;
                }
            }
            self.delta[i] = blend;
        }
        for (v, b) in self.vel.iter_mut().zip(&self.delta) {
            for (v, b) in v.iter_mut().zip(b) {
                *v += b * 0.06;
            }
        }
        std::mem::swap(&mut self.pos, &mut self.guess);
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for (i, p) in self.pos.iter().enumerate() {
            let v = self.vel[i];
            let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            // Bright where it is moving: a splash lights up and a
            // settled pool goes quiet.
            let shade = (110.0 + 145.0 * (speed / 2.5).min(1.0)) as u8;
            for k in 0..BLOB {
                let o = self.blob[i * BLOB + k];
                out.push(Point {
                    pos: [
                        ((p[0] + o[0] - 0.5) * 2.0).clamp(-1.0, 1.0),
                        ((p[1] + o[1] - 0.5) * 2.0).clamp(-1.0, 1.0),
                        ((p[2] + o[2] - 0.5) * 2.0).clamp(-1.0, 1.0),
                    ],
                    normal: [0.0; 3],
                    color: [shade, shade, shade],
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

    /// A rule is read as counts, ranges and a state count, and a rule
    /// that will not read is the shipped one rather than a blank
    /// lattice.
    #[test]
    fn automaton_rules_parse_and_bad_ones_fall_back() {
        let (s, b, states) = parse_rule("13-26/13-14,17-19").unwrap();
        assert!(s[13] && s[26] && !s[12]);
        assert!(b[13] && b[14] && !b[15] && b[17] && b[19] && !b[20]);
        assert_eq!(states, 2, "a rule with no third field is alive or dead");
        let (s, b, states) = parse_rule("4/4").unwrap();
        assert!(s[4] && !s[5] && b[4]);
        assert_eq!(states, 2);
        assert_eq!(parse_rule("4/4/5").unwrap().2, 5);
        assert_eq!(parse_rule("4-7/6-8/10").unwrap().2, 10);
        assert!(parse_rule("4").is_none());
        assert!(parse_rule("27/4").is_none());
        assert!(parse_rule("a/b").is_none());
        assert!(parse_rule("4/4/1").is_none(), "one state is no automaton");
        assert!(parse_rule("4/4/99").is_none());
        assert!(parse_rule("4/4/5/6").is_none());
        let fallback = Life::with_rule("nonsense");
        assert_eq!(fallback.survive, Life::new().survive);
        assert_eq!(fallback.states, 2);
        assert!(start("life?rule=4/4").is_some());
    }

    /// A rule with states behaves like one: cells count down through
    /// the middle of the ramp instead of dying at once, only the top of
    /// it counts as a neighbour, and the ash is drawn dimmer than the
    /// front. Run for all four of the shipped multi-state rules, each
    /// of which has to still be going at the end.
    #[test]
    fn multi_state_rules_leave_ash_behind_them() {
        for rule in ["4/4/5", "9-26/5-7,12-13,15/5", "2,6,9/4,6,8-9/10", "4-7/6-8/10"] {
            let mut l = Life::with_rule(rule);
            let states = l.states;
            assert!(states > 2, "{rule} should have states");
            for _ in 0..300 {
                l.step(1.0 / 60.0, &Drive::default());
            }
            let cells = LG * LG * LG;
            let alive = l.alive();
            assert!(alive > cells / 500 && alive < cells * 9 / 10, "{rule}: {alive} of {cells}");
            let dying = l.cells.iter().filter(|c| **c > 0 && **c < states - 1).count();
            assert!(dying > 0, "{rule} never left any ash");
            let mut pts = Vec::new();
            l.points(&mut pts);
            box_ok(&pts);
            let dim = pts.iter().filter(|p| p.color[0] < 255).count();
            assert!(dim > 0, "{rule} drew the ash as bright as the front");
        }
    }

    /// The system holds together: bodies stay in the box, keep moving,
    /// and the disc does not collapse onto the centre or evaporate off
    /// the edge over half a minute.
    #[test]
    fn the_orbits_hold_together() {
        let mut o = Orbits::new();
        let first = o.pos[7];
        for _ in 0..1_800 {
            o.step(1.0 / 60.0, &Drive::default());
        }
        let moved = (0..3).map(|k| (o.pos[7][k] - first[k]).abs()).fold(0.0f32, f32::max);
        assert!(moved > 0.05, "nothing went round: {moved}");
        let radii: Vec<f32> = o
            .pos
            .iter()
            .map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt())
            .collect();
        for r in &radii {
            assert!(r.is_finite() && *r < 1.2, "a body left: {r}");
        }
        let spread = radii.iter().filter(|r| **r > 0.1).count();
        assert!(spread > BODIES / 2, "the disc fell in: {spread} of {BODIES} still out");
        let mut pts = Vec::new();
        o.points(&mut pts);
        box_ok(&pts);
        // A kick puffs the disc outward: the mean speed away from the
        // centre jumps by about the size of the impulse.
        let outward = |o: &Orbits| {
            o.pos
                .iter()
                .zip(&o.vel)
                .map(|(p, v)| {
                    let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-4);
                    (0..3).map(|k| v[k] * p[k] / r).sum::<f32>()
                })
                .sum::<f32>()
                / BODIES as f32
        };
        let before = outward(&o);
        let loud = Drive { bands: [1.0, 0.0, 0.0, 0.0], level: 0.5, bar: 0.0, audio: true };
        o.step(1.0 / 60.0, &loud);
        let after = outward(&o);
        assert!(after - before > 0.04, "the kick did not push: {before} to {after}");
    }

    /// The curtain tears: pendulums that started next to each other are
    /// far apart later, which is the whole point of the thing, and none
    /// of them blows up while doing it.
    #[test]
    fn the_pendulum_curtain_tears() {
        let mut p = Pendulum::new();
        let start = p.state.clone();
        let neighbours = |s: &[[f32; 4]]| {
            let mut worst = 0.0f32;
            for row in 0..PEND_ALONG {
                for col in 1..PEND_ACROSS {
                    let a = s[row * PEND_ACROSS + col][0];
                    let b = s[row * PEND_ACROSS + col - 1][0];
                    worst = worst.max((a - b).abs().min(std::f32::consts::TAU - (a - b).abs()));
                }
            }
            worst
        };
        assert!(neighbours(&start) < 0.02, "the sheet did not start smooth");
        for _ in 0..600 {
            p.step(1.0 / 60.0, &Drive::default());
        }
        assert!(neighbours(&p.state) > 0.5, "the sheet never tore");
        for s in &p.state {
            for v in s {
                assert!(v.is_finite(), "a pendulum blew up: {s:?}");
            }
            // Runge–Kutta at this step keeps the energy near enough that
            // nothing ends up spinning like a propeller.
            assert!(s[2].abs() < 40.0 && s[3].abs() < 40.0, "gained energy: {s:?}");
        }
        let mut pts = Vec::new();
        p.points(&mut pts);
        box_ok(&pts);
        // A kick, once the sheet has had time to tear, hangs it again.
        let loud = Drive { bands: [1.0, 0.0, 0.0, 0.0], level: 0.5, bar: 0.0, audio: true };
        p.step(1.0 / 60.0, &loud);
        assert!(neighbours(&p.state) < 0.02, "the kick did not re-hang the sheet");
    }

    /// The box holds a plume: the field stays bounded and very nearly
    /// divergence-free, the heat rises rather than sinking, and the
    /// tracers are carried by it.
    #[test]
    fn the_smoke_rises_and_stays_incompressible() {
        let mut sim = Smoke::new();
        let first = sim.tracers[POINTS / 3];
        for _ in 0..240 {
            sim.step(1.0 / 60.0, &Drive::default());
        }
        for ((u, v), w) in sim.u.iter().zip(&sim.v).zip(&sim.w) {
            assert!(u.is_finite() && v.is_finite() && w.is_finite(), "the field blew up");
            assert!(u.abs() <= 3.001 && v.abs() <= 3.001 && w.abs() <= 3.001);
        }
        // The same measure the sheet's test uses: the residual in the
        // units the solve itself works in, which is the divergence
        // times half the cell size.
        let h = 1.0 / SG as f32;
        let worst = (0..SG)
            .flat_map(|k| (0..SG).flat_map(move |j| (0..SG).map(move |i| (i, j, k))))
            .map(|(i, j, k)| {
                (0.5
                    * h
                    * ((sim.u[Smoke::at(i + 1, j, k)] - sim.u[Smoke::at(i + SG - 1, j, k)])
                        + (sim.v[Smoke::at(i, j + 1, k)] - sim.v[Smoke::at(i, j + SG - 1, k)])
                        + (sim.w[Smoke::at(i, j, k + 1)] - sim.w[Smoke::at(i, j, k + SG - 1)])))
                    .abs()
            })
            .fold(0.0f32, f32::max);
        assert!(worst < 0.05, "the solve left divergence behind: {worst}");
        // The hot cells are above the cold ones: the mean height of the
        // heat is above the middle of the box.
        let (mut weight, mut height) = (0.0f32, 0.0f32);
        for k in 0..SG {
            for j in 0..SG {
                for i in 0..SG {
                    let hot = sim.heat[Smoke::at(i, j, k)];
                    weight += hot;
                    height += hot * (j as f32 + 0.5) / SG as f32;
                }
            }
        }
        assert!(weight > 0.5, "there is no heat in the box: {weight}");
        let centre = height / weight;
        assert!(centre > 0.25, "the heat sank to the floor: {centre}");
        let moved = (0..3)
            .map(|k| (sim.tracers[POINTS / 3][k] - first[k]).abs())
            .fold(0.0f32, f32::max);
        assert!(moved > 1e-3, "the tracers were not carried: {moved}");
        let mut pts = Vec::new();
        sim.points(&mut pts);
        box_ok(&pts);
        let bright = pts.iter().map(|p| p.color[0]).max().unwrap();
        let dim = pts.iter().map(|p| p.color[0]).min().unwrap();
        assert!(bright > dim + 40, "the plume does not stand out: {dim} to {bright}");
    }

    /// The liquid behaves like one: it stays in the box, it settles
    /// downhill, it does not crush itself, and a kick throws it up.
    #[test]
    fn the_liquid_settles_without_crushing_itself() {
        let mut l = Liquid::new();
        for _ in 0..600 {
            l.step(1.0 / 60.0, &Drive::default());
        }
        for p in &l.pos {
            for v in p {
                assert!(v.is_finite() && (0.0..=1.0).contains(v), "a particle left the box: {p:?}");
            }
        }
        // Density near the rest value: crowded enough to be a liquid
        // rather than a gas, not crushed into a point.
        l.find_neighbours();
        let mut rho = l.densities();
        rho.sort_by(f32::total_cmp);
        let median = rho[DROPS / 2];
        assert!((0.75..1.35).contains(&median), "the density drifted to {median}");
        // Settled: most of it is in the lower half, because gravity
        // here leans but still points down.
        let low = l.pos.iter().filter(|p| p[1] < 0.5).count();
        assert!(low > DROPS * 2 / 3, "the liquid did not settle: {low} of {DROPS} low");
        let mut pts = Vec::new();
        l.points(&mut pts);
        box_ok(&pts);
        // A kick throws it at the ceiling.
        let before = l.pos.iter().map(|p| p[1]).sum::<f32>() / DROPS as f32;
        let loud = Drive { bands: [1.0, 0.0, 0.0, 0.0], level: 0.6, bar: 0.0, audio: true };
        l.step(1.0 / 60.0, &loud);
        for _ in 0..30 {
            l.step(1.0 / 60.0, &Drive { audio: true, ..Drive::default() });
        }
        let after = l.pos.iter().map(|p| p[1]).sum::<f32>() / DROPS as f32;
        assert!(after > before + 0.02, "the kick did not lift it: {before} to {after}");
    }

    /// Every simulation is reachable by id, and nothing else is.
    #[test]
    fn simulations_start_by_id() {
        for id in IDS {
            let mut sim = start(id).unwrap_or_else(|| panic!("{id} would not start"));
            // The contract every live slot relies on, whatever the
            // simulation is: a full slot of finite points inside the
            // box, on the first frame and on the hundredth, whether or
            // not anything is plugged in.
            let loud = Drive { bands: [1.0, 0.7, 0.9, 0.6], level: 0.8, bar: 0.5, audio: true };
            let quiet = Drive::default();
            let mut out = Vec::new();
            sim.points(&mut out);
            if !out.is_empty() {
                box_ok(&out);
            }
            for i in 0..100 {
                sim.step(1.0 / 60.0, if i % 7 == 0 { &loud } else { &quiet });
            }
            sim.points(&mut out);
            box_ok(&out);
        }
        assert!(start("weather").is_none());
    }
}
