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
//! Nineteen ship. Some are *fields on a grid*: **fluid** is Stam's
//! stable solver for the incompressible Navier–Stokes equations
//! (Stam, "Stable Fluids", 1999; "Real-Time Fluid Dynamics for Games",
//! 2003) on a periodic sheet, with Fedkiw's vorticity confinement to
//! keep the swirls alive; **smoke** is the same solver in a box, with
//! heat to lift it; **reaction** is the Gray–Scott system in Pearson's
//! parameterisation; **wind** is curl noise, a fluid with no solve at
//! all; **life** is a cellular automaton on a cubic lattice;
//! **cyclic** is another, whose states chase each other round a
//! ring until the lattice fills with scroll waves; **sand** is the
//! Abelian sandpile; and **spiral** is the Belousov–Zhabotinsky
//! reaction as a cellular model.
//!
//! Some are *many bodies*: **flock** is Reynolds' boids, **orbits** is
//! gravity by direct summation, **liquid** is position-based fluids,
//! **pendulum** is four thousand double pendulums hung in a sheet,
//! **slime** is Physarum, **swarm** is swarmalators, **cloth** is a
//! mass-spring sheet in a wind, **vortex** is the thin cores smoke
//! rings are made of, moving each other by Biot–Savart, **tangle** is
//! one long elastic rod tying itself in knots.
//!
//! And **crystal** is grown: Reiter's snowflake on a hexagonal
//! lattice. **kuramoto** is a crowd of coupled oscillators, which is
//! none of the above.
//!//! They cost between half a millisecond and twenty per frame, on a
//! thread of their own, and a slow one loses frames rather than the
//! picture: the renderer takes whatever the slot holds.

use std::f32::consts::SQRT_2;

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
    &["fluid", "reaction", "flock", "wind", "kuramoto", "life", "orbits", "pendulum", "smoke", "liquid", "slime", "swarm", "cloth", "sand", "spiral", "cyclic", "tangle", "crystal", "vortex"];

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
        "life" => Some(Box::new(Life::with_rule(&text("rule", Life::DEFAULT)))),
        "orbits" => Some(Box::new(Orbits::new())),
        "pendulum" => Some(Box::new(Pendulum::new())),
        "smoke" => Some(Box::new(Smoke::new())),
        "liquid" => Some(Box::new(Liquid::new())),
        "slime" => Some(Box::new(Slime::new())),
        "swarm" => Some(Box::new(Swarm::new())),
        "cloth" => Some(Box::new(Cloth::new())),
        "sand" => Some(Box::new(Sand::new())),
        "spiral" => Some(Box::new(Spiral::new())),
        "cyclic" => Some(Box::new(Cyclic::new())),
        "tangle" => Some(Box::new(Tangle::new())),
        "crystal" => Some(Box::new(Crystal::new())),
        "vortex" => Some(Box::new(Vortex::new())),
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
    /// Generations in a row that have changed almost nothing.
    stalled: u32,
    since_kick: f32,
    rng: Rng,
}

impl Life {
    /// The rule this runs unless told otherwise: Pyroclastic, which
    /// boils. A live slot has to keep moving, and most of the
    /// catalogued three-dimensional rules do not — Clouds
    /// (`13-26/13-14,17-19`) grows into lovely masses and then stops
    /// dead, changing sixty cells a generation out of a hundred and
    /// ten thousand, which on screen is a still image with a name on
    /// it. This one turns over a third of the lattice every
    /// generation, for ever.
    pub const DEFAULT: &'static str = "4-7/6-8/10";

    pub fn new() -> Self {
        Self::with_rule(Self::DEFAULT)
    }

    /// An automaton in the rule `survive/born` or `survive/born/states`,
    /// the first two sides lists of neighbour counts and ranges —
    /// `13-26/13-14,17-19`, `4/4/5`. A rule that will not parse is the
    /// shipped one, because a blank rule is a blank slot.
    pub fn with_rule(rule: &str) -> Self {
        let (survive, born, states) = parse_rule(rule)
            .unwrap_or_else(|| parse_rule(Self::DEFAULT).expect("the shipped rule parses"));
        let mut l = Self {
            cells: vec![0; LG * LG * LG],
            next: vec![0; LG * LG * LG],
            survive,
            born,
            states,
            frame: 0,
            stalled: 0,
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
        // The two-state rules here want a *crowd*: Clouds asks for
        // thirteen of a cell's twenty-six neighbours before it will
        // keep it alive, so a small block is all edge, the edge all
        // dies, and what is left is a speck that keeps being reseeded.
        // Half the lattice across is a nucleus that can hold itself up.
        if self.states > 2 { 6 } else { 24 }
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

    /// Advance one generation, and say how many cells it changed.
    fn generation(&mut self) -> u32 {
        let mut changed = 0u32;
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
                    let was = cell;
                    let now = match cell {
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
                    self.next[at(x, y, z)] = now;
                    changed += u32::from(now != was);
                }
            }
        }
        std::mem::swap(&mut self.cells, &mut self.next);
        changed
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
            let changed = self.generation();
            // A generation that changes almost nothing has stopped, and
            // a stopped automaton is as dead as an empty one — most of
            // the catalogued rules grow into a shape and then hold it
            // for ever, which in a live slot is a still image. Four of
            // those in a row and it starts again.
            self.stalled = if changed < 64 { self.stalled + 1 } else { 0 };
            // Died out, or filled the lattice: start again from a seed.
            // A dead automaton is a blank slot with a name on it.
            let alive = self.alive();
            if self.stalled >= 4 || !(64..=LG * LG * LG * 9 / 10).contains(&alive) {
                self.cells.iter_mut().for_each(|c| *c = 0);
                let (x, y, z) = (
                    (self.rng.f32() * LG as f32) as usize,
                    (self.rng.f32() * LG as f32) as usize,
                    (self.rng.f32() * LG as f32) as usize,
                );
                let side = self.seed_side();
                self.seed(x, y, z, side);
                self.stalled = 0;
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
        Self::near(vent, 0.05, |idx, g| heat[idx] += g * 16.0 * dt);
        if kick {
            // A blast: hot, and somewhere else.
            let at = [self.rng.f32(), 0.12, self.rng.f32()];
            let amount = 45.0 * drive.bands[0] * dt;
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
        // Cooling fast enough that a parcel fades before it has risen
        // the height of the box. A periodic box has no sky to lose
        // heat to, so a slow cool simply fills it: the hot air leaves
        // the top, comes back in at the bottom, and within a few
        // seconds there is no plume, only a warm room.
        let cool = (1.0 - 1.6 * dt).clamp(0.0, 1.0);
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
            // Cold air is nearly black, not grey. The box is evenly
            // full of tracers so that none of them ever has to be
            // re-seeded, which means most of them are sitting in air
            // that is doing nothing, and at a fifth brightness sixty
            // thousand of those wash the plume out entirely.
            let bright = (0.06 + 0.94 * (h * 2.2).min(1.0)).clamp(0.0, 1.0);
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

// --- Slime ------------------------------------------------------------

/// Cells along each side of the trail grid.
const TG: usize = 64;
const TPLANE: usize = TG * TG;
const TCELLS: usize = TG * TG * TG;

/// Physarum polycephalum, which is a single cell the size of a dinner
/// plate with no nervous system and a habit of solving mazes.
///
/// Jones' model (2010) is three rules and nothing else: leave a trail,
/// look a short way ahead in a few directions, and steer towards
/// whichever has the most trail on it. Nothing in it knows about paths
/// or networks. What emerges anyway is a transport network that keeps
/// rebuilding itself — the same thing the real organism does when it
/// reproduces the Tokyo rail map out of oat flakes.
///
/// One agent per point, so the cloud *is* the colony rather than a
/// rendering of it, and the trail exists only to be followed.
pub struct Slime {
    pos: Vec<[f32; 3]>,
    /// Unit headings.
    dir: Vec<[f32; 3]>,
    trail: Vec<f32>,
    swap: Vec<f32>,
    time: f32,
    since_kick: f32,
    rng: Rng,
}

impl Slime {
    /// How far ahead an agent looks, in box units — about four cells.
    const SENSE: f32 = 0.06;
    /// How much trail one agent leaves a second.
    const DEPOSIT: f32 = 6.0;

    pub fn new() -> Self {
        let mut rng = Rng::new(0x511E_511E);
        let mut pos = Vec::with_capacity(POINTS);
        let mut dir = Vec::with_capacity(POINTS);
        for _ in 0..POINTS {
            // Everywhere, not in a clump: the network has to be built
            // out of a uniform field, and a colony started in a ball is
            // already in the one configuration the dynamics cannot get
            // out of.
            pos.push([rng.f32(), rng.f32(), rng.f32()]);
            dir.push(rng.on_sphere());
        }
        Self {
            pos,
            dir,
            trail: vec![0.0; TCELLS],
            swap: vec![0.0; TCELLS],
            time: 0.0,
            since_kick: 10.0,
            rng,
        }
    }

    fn at(p: [f32; 3]) -> usize {
        let c: [usize; 3] =
            std::array::from_fn(|i| ((p[i].rem_euclid(1.0) * TG as f32) as usize).min(TG - 1));
        c[0] + c[1] * TG + c[2] * TPLANE
    }

    /// Two directions at right angles to a heading, for swinging the
    /// sensors around it.
    fn frame(h: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        // Cross with whichever axis the heading leans on least, so the
        // cross product is never near zero.
        let a = if h[0].abs() < h[1].abs() && h[0].abs() < h[2].abs() {
            [1.0, 0.0, 0.0]
        } else if h[1].abs() < h[2].abs() {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let u = [
            h[1] * a[2] - h[2] * a[1],
            h[2] * a[0] - h[0] * a[2],
            h[0] * a[1] - h[1] * a[0],
        ];
        let n = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt().max(1e-6);
        let u = [u[0] / n, u[1] / n, u[2] / n];
        let v = [
            h[1] * u[2] - h[2] * u[1],
            h[2] * u[0] - h[0] * u[2],
            h[0] * u[1] - h[1] * u[0],
        ];
        (u, v)
    }

    /// Blur and fade the trail: a three-tap pass along each axis, which
    /// is the same as a 27-cell blur at a ninth of the reads.
    fn diffuse(&mut self, decay: f32) {
        for axis in 0..3 {
            let stride = match axis {
                0 => 1,
                1 => TG,
                _ => TPLANE,
            };
            for k in 0..TG {
                for j in 0..TG {
                    for i in 0..TG {
                        let idx = i + j * TG + k * TPLANE;
                        // The neighbours along this axis, wrapped.
                        let c = [i, j, k][axis];
                        let up = idx + stride - if c + 1 == TG { TG * stride } else { 0 };
                        let down = idx + if c == 0 { TG * stride } else { 0 } - stride;
                        self.swap[idx] =
                            0.5 * self.trail[idx] + 0.25 * (self.trail[up] + self.trail[down]);
                    }
                }
            }
            std::mem::swap(&mut self.trail, &mut self.swap);
        }
        for t in &mut self.trail {
            *t *= decay;
        }
    }
}

impl Default for Slime {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Slime {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        // Fast enough to leave the cell it is in. An agent that moves
        // a fraction of a cell a frame deposits into the same cell over
        // and over, which is a positive feedback into a point: the
        // whole colony walks into its own trail and collapses to a
        // blob. Half a box a second is about half a cell a frame.
        let speed = if drive.audio { 0.35 + 0.35 * drive.level } else { 0.5 };
        // The sensor cone: wide agents wander and the network is
        // ragged, narrow ones commit and it is clean.
        let cone = if drive.audio { 0.35 + 0.7 * drive.bands[3] } else { 0.6 };
        let turn = 7.0 * dt;
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.4;
        if kick {
            self.since_kick = 0.0;
        }
        let (sin_cone, cos_cone) = cone.sin_cos();
        for i in 0..POINTS {
            let p = self.pos[i];
            let h = self.dir[i];
            let (u, v) = Self::frame(h);
            // Five sensors: straight on, and four around the cone.
            let mut best = h;
            let mut most = self.trail[Self::at([
                p[0] + h[0] * Self::SENSE,
                p[1] + h[1] * Self::SENSE,
                p[2] + h[2] * Self::SENSE,
            ])];
            for turn_index in 0..4 {
                let a = turn_index as f32 * std::f32::consts::FRAC_PI_2;
                let (sa, ca) = a.sin_cos();
                let d: [f32; 3] = std::array::from_fn(|k| {
                    h[k] * cos_cone + (u[k] * ca + v[k] * sa) * sin_cone
                });
                let value = self.trail[Self::at([
                    p[0] + d[0] * Self::SENSE,
                    p[1] + d[1] * Self::SENSE,
                    p[2] + d[2] * Self::SENSE,
                ])];
                if value > most {
                    most = value;
                    best = d;
                }
            }
            // Steer towards it rather than snapping: the trail an agent
            // leaves behind is only useful if it is smooth.
            let mut h: [f32; 3] =
                std::array::from_fn(|k| self.dir[i][k] + (best[k] - self.dir[i][k]) * turn);
            let n = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
            if n > 1e-6 {
                for c in &mut h {
                    *c /= n;
                }
            } else {
                h = self.rng.on_sphere();
            }
            if kick && self.rng.f32() < 0.15 {
                // A share of the colony thrown somewhere else, which
                // starts a new front rather than moving the old one.
                let d = self.rng.on_sphere();
                let r = 0.3 * self.rng.f32().cbrt();
                self.pos[i] = [0.5 + d[0] * r, 0.5 + d[1] * r, 0.5 + d[2] * r];
                self.dir[i] = self.rng.on_sphere();
                continue;
            }
            self.dir[i] = h;
            let next: [f32; 3] =
                std::array::from_fn(|k| (p[k] + h[k] * speed * dt).rem_euclid(1.0));
            self.pos[i] = next;
            self.trail[Self::at(next)] += Self::DEPOSIT * dt;
        }
        // A tenth of the trail a frame, which is the decay the model
        // is usually run at: slower and the field saturates, faster and
        // nothing is left to follow.
        self.diffuse((1.0 - 6.0 * dt).clamp(0.0, 1.0));
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for p in &self.pos {
            // Bright where the trail is thick, so the veins of the
            // network stand out from the agents still exploring.
            let t = self.trail[Self::at(*p)];
            let shade = (90.0 + 165.0 * (t * 2.5).min(1.0)) as u8;
            out.push(Point {
                pos: [(p[0] - 0.5) * 2.0, (p[1] - 0.5) * 2.0, (p[2] - 0.5) * 2.0],
                normal: [0.0; 3],
                color: [shade, shade, shade],
            });
        }
    }
}

// --- Swarmalators -----------------------------------------------------

/// Swarmalators, and the points each is drawn with.
const MATES: usize = 1024;
const MATE_BLOB: usize = 64;

/// Swarmalators (O'Keeffe, Hong & Strogatz, 2017): particles that both
/// *swarm* and *synchronise*, where each depends on the other.
///
/// Fireflies sync their flashing; starlings swarm; a swarmalator does
/// both at once, and the two are coupled — how strongly two of them are
/// drawn together depends on how close their phases are, and how
/// strongly their phases pull on each other depends on how close they
/// are in space. Sperm do this. So do magnetic colloids, and the
/// Japanese tree frogs that arrange themselves in a pond by call.
///
/// Five states come out of two numbers, and the transitions between
/// them are sharp: a ball where everything is in phase, a ball where
/// phase is spread at random, a disc where phase runs round the rim, a
/// disc that has splintered into blocks of one phase each, and the same
/// disc with the blocks circulating. The audio drives the two numbers,
/// so a set walks through the states.
pub struct Swarm {
    pos: Vec<[f32; 3]>,
    vel: Vec<[f32; 3]>,
    phase: Vec<f32>,
    pace: Vec<f32>,
    step: Vec<f32>,
    blob: Vec<[f32; 3]>,
    time: f32,
    since_kick: f32,
    rng: Rng,
}

impl Swarm {
    pub fn new() -> Self {
        debug_assert_eq!(MATES * MATE_BLOB, POINTS);
        let mut rng = Rng::new(0x5A11_5A11);
        let mut pos = Vec::with_capacity(MATES);
        let mut phase = Vec::with_capacity(MATES);
        let mut pace = Vec::with_capacity(MATES);
        for _ in 0..MATES {
            let d = rng.on_sphere();
            let r = 0.6 * rng.f32().cbrt();
            pos.push([d[0] * r, d[1] * r, d[2] * r]);
            phase.push(rng.f32() * std::f32::consts::TAU);
            // Identical natural paces, as the model is usually studied:
            // everything interesting here comes from the coupling, not
            // from a spread of clocks.
            pace.push(0.6);
        }
        let blob = (0..MATES * MATE_BLOB)
            .map(|_| {
                let d = rng.on_sphere();
                let r = 0.035 * rng.f32().cbrt();
                [d[0] * r, d[1] * r, d[2] * r]
            })
            .collect();
        Self {
            vel: vec![[0.0; 3]; MATES],
            step: vec![0.0; MATES],
            pos,
            phase,
            pace,
            blob,
            time: 0.0,
            since_kick: 10.0,
            rng,
        }
    }
}

impl Default for Swarm {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Swarm {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        // J: how much being in phase draws two of them together.
        // K: how much being close pulls their phases together. The
        // states live at the corners of this square, and a loud room
        // pushes towards the ordered ones.
        let (j, k) = if drive.audio {
            (0.1 + 0.9 * drive.level, -0.75 + 1.6 * drive.bands[2])
        } else {
            // Left alone, it wanders the square on its own so that all
            // five states come round.
            (0.6 + 0.4 * (self.time * 0.07).sin(), -0.35 + 0.65 * (self.time * 0.043).cos())
        };
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.4 {
            self.since_kick = 0.0;
            for p in &mut self.pos {
                let d = self.rng.on_sphere();
                for (c, d) in p.iter_mut().zip(d) {
                    *c += d * 0.25;
                }
            }
        }
        let n = MATES as f32;
        // Both halves of the interaction are antisymmetric — the pull
        // two of them feel is equal and opposite, and so is the pull on
        // their phases — so half the pairs do all the work.
        self.vel.fill([0.0; 3]);
        self.step.fill(0.0);
        for i in 0..MATES {
            let (pi, ti) = (self.pos[i], self.phase[i]);
            for j_index in (i + 1)..MATES {
                let pj = self.pos[j_index];
                let d = [pj[0] - pi[0], pj[1] - pi[1], pj[2] - pi[2]];
                let r2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                let r = r2.sqrt().max(1e-3);
                let gap = self.phase[j_index] - ti;
                // Attraction that phase agreement strengthens, and a
                // harder repulsion that stops them piling up.
                let a = ((1.0 + j * gap.cos()) / r - 1.0 / (r * r)) / r;
                for (k, d) in d.iter().enumerate() {
                    self.vel[i][k] += d * a;
                    self.vel[j_index][k] -= d * a;
                }
                let turn = gap.sin() / r;
                self.step[i] += turn;
                self.step[j_index] -= turn;
            }
        }
        for i in 0..MATES {
            for v in &mut self.vel[i] {
                *v /= n;
            }
            self.step[i] = self.pace[i] + k * self.step[i] / n;
        }
        for i in 0..MATES {
            for (p, v) in self.pos[i].iter_mut().zip(self.vel[i]) {
                // A ceiling on the step: two that pass very close see a
                // large force for one frame, and without this they are
                // fired out of the picture.
                *p += v.clamp(-2.5, 2.5) * dt;
            }
            // And a leash rather than a wall for the one that gets out
            // anyway. A clamp would park it on the boundary, where it
            // stays for good and drags the whole swarm's scale with it;
            // this walks it back in over a quarter of a second.
            let p = self.pos[i];
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            if r > 1.4 {
                let back = (r - 1.4) * (4.0 * dt).min(1.0) / r;
                for c in &mut self.pos[i] {
                    *c -= *c * back;
                }
            }
            self.phase[i] = (self.phase[i] + self.step[i].clamp(-20.0, 20.0) * dt)
                .rem_euclid(std::f32::consts::TAU);
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        // Fitted to the box each frame, because the swarm's own size is
        // one of the things that changes between states.
        let reach = self
            .pos
            .iter()
            .flat_map(|p| p.iter().map(|c| c.abs()))
            .fold(0.3f32, f32::max);
        let scale = 0.94 / reach;
        for (i, p) in self.pos.iter().enumerate() {
            // Phase as brightness, so a synchronised swarm pulses as
            // one and a phase wave is a band running round the rim.
            let shade = (60.0 + 195.0 * (0.5 + 0.5 * self.phase[i].cos())) as u8;
            for b in 0..MATE_BLOB {
                let o = self.blob[i * MATE_BLOB + b];
                out.push(Point {
                    pos: [
                        (p[0] * scale + o[0]).clamp(-1.0, 1.0),
                        (p[1] * scale + o[1]).clamp(-1.0, 1.0),
                        (p[2] * scale + o[2]).clamp(-1.0, 1.0),
                    ],
                    normal: [0.0; 3],
                    color: [shade, shade, shade],
                });
            }
        }
    }
}

// --- Cloth ------------------------------------------------------------

/// Particles along each side of the sheet. Their square is a slot.
const WEAVE: usize = 256;

/// A sheet of cloth, hung from its top edge, in a wind.
///
/// Mass-spring cloth is older than real-time graphics, and the reason
/// it is still here is that nothing else looks like cloth. What makes
/// it behave is not the springs but how they are solved: integrating
/// spring forces at a frame's time step blows a stiff sheet apart, so
/// the links are treated as *constraints* and satisfied by moving the
/// particles directly (Provot, 1995; Jakobsen, 2001). A constraint pass
/// cannot add energy, which is why this is stable at any step.
///
/// The wind is the same Arnold–Beltrami–Childress flow that
/// `/shape/wind` blows through the particle field, sampled at each
/// particle: a closed form, so the sheet needs no fluid behind it.
pub struct Cloth {
    pos: Vec<[f32; 3]>,
    /// Where it was last frame; the velocity is the difference.
    was: Vec<[f32; 3]>,
    time: f32,
    since_kick: f32,
    gust: f32,
}

impl Cloth {
    /// The rest length of a link between neighbours.
    const LINK: f32 = 1.8 / (WEAVE as f32 - 1.0);
    /// Relaxation passes a frame.
    const PASSES: usize = 4;
    /// How heavy the sheet is.
    const WEIGHT: f32 = 0.6;

    pub fn new() -> Self {
        debug_assert_eq!(WEAVE * WEAVE, POINTS);
        let mut pos = Vec::with_capacity(POINTS);
        for row in 0..WEAVE {
            for col in 0..WEAVE {
                pos.push([
                    -0.9 + col as f32 * Self::LINK,
                    0.9 - row as f32 * Self::LINK,
                    0.0,
                ]);
            }
        }
        Self { was: pos.clone(), pos, time: 0.0, since_kick: 10.0, gust: 0.0 }
    }

    /// The ABC flow, as the shader reads it: divergence-free, and
    /// chaotic in its streamlines, which is what makes a sheet in it
    /// fold rather than merely bulge.
    fn wind(p: [f32; 3], t: f32) -> [f32; 3] {
        let one = |q: [f32; 3]| {
            [q[2].sin() + q[1].cos(), q[0].sin() + q[2].cos(), q[1].sin() + q[0].cos()]
        };
        let a = one([p[0] * 2.2 + t, p[1] * 2.2 + t * 0.7, p[2] * 2.2 + t * 1.3]);
        let b = one([p[0] * 5.1 - t * 1.1, p[1] * 5.1 + t * 1.7, p[2] * 5.1 + t * 0.5]);
        std::array::from_fn(|k| a[k] + 0.4 * b[k])
    }

    /// Pull two particles back to the rest length, half from each —
    /// except at the top edge, which is nailed up.
    fn link(&mut self, a: usize, b: usize, rest: f32) {
        let (pa, pb) = (self.pos[a], self.pos[b]);
        let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len < 1e-6 {
            return;
        }
        let pull = (len - rest) / len * 0.5;
        let pinned_a = a < WEAVE;
        let pinned_b = b < WEAVE;
        if pinned_a && pinned_b {
            return;
        }
        let (share_a, share_b) = match (pinned_a, pinned_b) {
            (true, _) => (0.0, 2.0),
            (_, true) => (2.0, 0.0),
            _ => (1.0, 1.0),
        };
        for (k, d) in d.iter().enumerate() {
            self.pos[a][k] += d * pull * share_a;
            self.pos[b][k] -= d * pull * share_b;
        }
    }
}

impl Default for Cloth {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Cloth {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        self.gust *= (-dt * 1.6).exp();
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.3 {
            self.since_kick = 0.0;
            self.gust = 1.0;
        }
        // The flow itself runs to about four; the sheet is one box
        // wide, so anything much above a tenth of that throws it clear
        // out of the picture in a second rather than billowing it.
        let strength = if drive.audio { 0.07 + 0.22 * drive.level } else { 0.15 } + 0.5 * self.gust;
        let t = self.time * 0.5;
        // Verlet: the velocity is where it was, so damping is a number
        // rather than a state.
        for i in 0..POINTS {
            let p = self.pos[i];
            if i < WEAVE {
                self.was[i] = p;
                continue;
            }
            let w = Self::wind(p, t);
            let a = [w[0] * strength, w[1] * strength - Self::WEIGHT, w[2] * strength];
            let next: [f32; 3] = std::array::from_fn(|k| {
                p[k] + (p[k] - self.was[i][k]) * 0.985 + a[k] * dt * dt
            });
            self.was[i] = p;
            self.pos[i] = next;
        }
        for _ in 0..Self::PASSES {
            // Structural links along the weave, then the diagonals that
            // stop it shearing into a parallelogram.
            for row in 0..WEAVE {
                for col in 0..WEAVE {
                    let i = row * WEAVE + col;
                    if col + 1 < WEAVE {
                        self.link(i, i + 1, Self::LINK);
                    }
                    if row + 1 < WEAVE {
                        self.link(i, i + WEAVE, Self::LINK);
                    }
                    if row + 1 < WEAVE && col + 1 < WEAVE {
                        self.link(i, i + WEAVE + 1, Self::LINK * SQRT_2);
                        self.link(i + 1, i + WEAVE, Self::LINK * SQRT_2);
                    }
                }
            }
        }
        // The sheet is the whole picture, so it stays in the frame: a
        // gust that would take it out of the box is stopped at the
        // wall rather than clipped away by the fit.
        for p in &mut self.pos {
            for c in p.iter_mut() {
                *c = c.clamp(-1.0, 1.0);
            }
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for (i, p) in self.pos.iter().enumerate() {
            // Bright where it is moving fastest, so the fold running
            // across the sheet is the thing the eye follows.
            let v = [p[0] - self.was[i][0], p[1] - self.was[i][1], p[2] - self.was[i][2]];
            let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let shade = (95.0 + 160.0 * (speed * 30.0).min(1.0)) as u8;
            out.push(Point {
                pos: [
                    p[0].clamp(-1.0, 1.0),
                    p[1].clamp(-1.0, 1.0),
                    p[2].clamp(-1.0, 1.0),
                ],
                normal: [0.0; 3],
                color: [shade, shade, shade],
            });
        }
    }
}

// --- Sandpile ---------------------------------------------------------

/// Cells along each side of the sandpile. Its square is a slot, so
/// there is one point per cell.
const PILE: usize = 256;

/// The Abelian sandpile (Bak, Tang & Wiesenfeld, 1987), the model that
/// named self-organised criticality.
///
/// Drop grains on a square. Any square holding four or more topples,
/// sending one grain to each neighbour, which may make them topple in
/// turn. That is the whole rule. Two facts about it are surprising.
/// The first is that the order of toppling does not matter — whatever
/// order you use, the final arrangement is the same, which is what
/// "Abelian" means here. The second is what a large pile looks like:
/// not a heap, but a fractal of nested triangles and squares that
/// nobody designed and which is still not fully explained.
///
/// It is also the origin of the idea that a system can drive *itself*
/// to the edge of stability and sit there, which is where avalanches,
/// earthquakes and forest fires get their power laws.
pub struct Sand {
    cells: Vec<u32>,
    /// Cells known to be over the limit, so a pass does not sweep the
    /// whole square looking for them.
    unstable: Vec<u32>,
    next: Vec<u32>,
    dropped: u64,
    since_kick: f32,
    rng: Rng,
}

impl Sand {
    /// Grains a second at the middle, with no audio. The pattern's
    /// radius grows as the square root of the count, so a square this
    /// size wants a few hundred thousand grains before the nested
    /// triangles appear — which is half a minute of watching, and the
    /// half minute is the point.
    const RAIN: f32 = 30_000.0;
    /// The most topplings one frame will do. A pass that ran to
    /// stability would take as long as it takes, and this runs inside a
    /// frame.
    const BUDGET: usize = 400_000;

    pub fn new() -> Self {
        let mut sand = Self {
            cells: vec![0; PILE * PILE],
            unstable: Vec::with_capacity(1 << 16),
            next: Vec::with_capacity(1 << 16),
            dropped: 0,
            since_kick: 10.0,
            rng: Rng::new(0x5A_4D),
        };
        sand.drop_at(PILE / 2, PILE / 2, 60_000);
        sand
    }

    fn drop_at(&mut self, x: usize, y: usize, grains: u32) {
        if grains == 0 {
            return;
        }
        let i = x.min(PILE - 1) + y.min(PILE - 1) * PILE;
        self.cells[i] += grains;
        self.dropped += u64::from(grains);
        if self.cells[i] >= 4 {
            self.unstable.push(i as u32);
        }
    }

    /// Topple until stable or out of budget. Grains that reach the edge
    /// fall off, which is what stops the pile growing for ever and is
    /// how the model is always run.
    fn settle(&mut self) {
        let mut spent = 0;
        while !self.unstable.is_empty() && spent < Self::BUDGET {
            self.next.clear();
            for index in std::mem::take(&mut self.unstable) {
                let i = index as usize;
                if self.cells[i] < 4 {
                    continue;
                }
                let times = self.cells[i] / 4;
                self.cells[i] -= times * 4;
                spent += times as usize;
                let (x, y) = (i % PILE, i / PILE);
                let give = |s: &mut Self, x: usize, y: usize| {
                    let j = x + y * PILE;
                    s.cells[j] += times;
                    if s.cells[j] >= 4 {
                        s.next.push(j as u32);
                    }
                };
                if x > 0 {
                    give(self, x - 1, y);
                }
                if x + 1 < PILE {
                    give(self, x + 1, y);
                }
                if y > 0 {
                    give(self, x, y - 1);
                }
                if y + 1 < PILE {
                    give(self, x, y + 1);
                }
                if self.cells[i] >= 4 {
                    self.next.push(index);
                }
            }
            std::mem::swap(&mut self.unstable, &mut self.next);
        }
    }

    /// How far the pile has spread from the middle, as a share of the
    /// half-width. Once it reaches the edge the pattern stops growing,
    /// so it is swept away and started again.
    fn reach(&self) -> f32 {
        let half = PILE / 2;
        let mut far = 0usize;
        for x in 0..PILE {
            if self.cells[x + half * PILE] > 0 {
                far = far.max(half.abs_diff(x));
            }
            if self.cells[half + x * PILE] > 0 {
                far = far.max(half.abs_diff(x));
            }
        }
        far as f32 / half as f32
    }
}

impl Default for Sand {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Sand {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.since_kick += dt;
        let rain = Self::RAIN * if drive.audio { 0.3 + 2.0 * drive.level } else { 1.0 };
        let grains = (rain * dt) as u32;
        self.drop_at(PILE / 2, PILE / 2, grains);
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.3 {
            self.since_kick = 0.0;
            // A load somewhere else, which sends an avalanche across
            // whatever the pattern had settled into.
            let (x, y) = (
                (self.rng.f32() * PILE as f32) as usize,
                (self.rng.f32() * PILE as f32) as usize,
            );
            self.drop_at(x, y, 4_000);
        }
        self.settle();
        if self.reach() > 0.94 {
            self.cells.iter_mut().for_each(|c| *c = 0);
            self.unstable.clear();
            self.dropped = 0;
            self.drop_at(PILE / 2, PILE / 2, 60_000);
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for (i, grains) in self.cells.iter().enumerate() {
            let (x, y) = (i % PILE, i / PILE);
            let height = (*grains).min(3) as f32 / 3.0;
            // Empty ground sits at the floor and is nearly dark; the
            // three levels above it step up and brighten, so the
            // nested triangles read as terraces.
            let shade = if *grains == 0 { 45.0 } else { 90.0 + 165.0 * height } as u8;
            out.push(Point {
                pos: [
                    (x as f32 + 0.5) / PILE as f32 * 2.0 - 1.0,
                    (height - 0.5) * 0.38,
                    (y as f32 + 0.5) / PILE as f32 * 2.0 - 1.0,
                ],
                normal: [0.0; 3],
                color: [shade, shade, shade],
            });
        }
    }
}

// --- Spirals ----------------------------------------------------------

/// Cells along each side of the reaction. Its square is a slot.
const BZ: usize = 256;

/// The Belousov–Zhabotinsky reaction, as a cellular model.
///
/// Belousov found in the 1950s that a dish of citric acid, bromate and
/// a cerium salt would change colour back and forth rather than
/// settling, and could not get it published: a chemical reaction that
/// oscillates looked to every referee like a violation of the second
/// law. It is not — the system is far from equilibrium and burning
/// fuel to do it — and by the 1970s the rotating spiral waves it makes
/// were understood to be the same excitable dynamics as a heartbeat
/// and a slime mould's signalling.
///
/// Three chemicals chase each other round a cycle, each one made at the
/// expense of the next, with a local average standing in for diffusion.
/// The waves annihilate where they meet, which is why they never
/// interfere: a spiral's arm is a front, not a ripple.
pub struct Spiral {
    a: Vec<f32>,
    b: Vec<f32>,
    c: Vec<f32>,
    a1: Vec<f32>,
    b1: Vec<f32>,
    c1: Vec<f32>,
    since_kick: f32,
    rng: Rng,
}

impl Spiral {
    pub fn new() -> Self {
        debug_assert_eq!(BZ * BZ, POINTS);
        let mut rng = Rng::new(0xB2_5217);
        let cells = BZ * BZ;
        // Started at random, because a spiral needs a defect to wind
        // around and a smooth start has none.
        let a: Vec<f32> = (0..cells).map(|_| rng.f32()).collect();
        let b: Vec<f32> = (0..cells).map(|_| rng.f32()).collect();
        let c: Vec<f32> = (0..cells).map(|_| rng.f32()).collect();
        Self {
            a1: vec![0.0; cells],
            b1: vec![0.0; cells],
            c1: vec![0.0; cells],
            a,
            b,
            c,
            since_kick: 10.0,
            rng,
        }
    }

    /// The mean of the nine cells around one, wrapping at the edges.
    fn around(field: &[f32], x: usize, y: usize) -> f32 {
        let mut sum = 0.0;
        for dy in 0..3 {
            let j = (y + dy + BZ - 1) % BZ;
            for dx in 0..3 {
                let i = (x + dx + BZ - 1) % BZ;
                sum += field[i + j * BZ];
            }
        }
        sum / 9.0
    }
}

impl Default for Spiral {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Spiral {
    fn step(&mut self, dt: f32, drive: &Drive) {
        self.since_kick += dt.clamp(0.0, 1.0);
        // How hard each chemical feeds on the next. Past about 1.4 the
        // waves break up into turbulence, and below 1 they die out.
        let alpha = if drive.audio { 1.0 + 0.5 * drive.level } else { 1.2 };
        const BETA: f32 = 1.0;
        const GAMMA: f32 = 1.0;
        for y in 0..BZ {
            for x in 0..BZ {
                let i = x + y * BZ;
                let (a, b, c) = (
                    Self::around(&self.a, x, y),
                    Self::around(&self.b, x, y),
                    Self::around(&self.c, x, y),
                );
                self.a1[i] = (a + a * (alpha * b - GAMMA * c)).clamp(0.0, 1.0);
                self.b1[i] = (b + b * (BETA * c - alpha * a)).clamp(0.0, 1.0);
                self.c1[i] = (c + c * (GAMMA * a - BETA * b)).clamp(0.0, 1.0);
            }
        }
        std::mem::swap(&mut self.a, &mut self.a1);
        std::mem::swap(&mut self.b, &mut self.b1);
        std::mem::swap(&mut self.c, &mut self.c1);
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.4 {
            self.since_kick = 0.0;
            // A patch of noise, which is a fresh crop of defects and
            // therefore a fresh crop of spirals.
            let (cx, cy) = (
                (self.rng.f32() * BZ as f32) as usize,
                (self.rng.f32() * BZ as f32) as usize,
            );
            for dy in 0..40 {
                for dx in 0..40 {
                    let i = (cx + dx) % BZ + ((cy + dy) % BZ) * BZ;
                    self.a[i] = self.rng.f32();
                    self.b[i] = self.rng.f32();
                    self.c[i] = self.rng.f32();
                }
            }
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for y in 0..BZ {
            for x in 0..BZ {
                let i = x + y * BZ;
                // The front stands up and brightens: one chemical is
                // the height, another the shade, so a wave is a ridge
                // with a lit edge rather than a flat stripe.
                let front = self.a[i];
                let shade = (70.0 + 185.0 * self.c[i].clamp(0.0, 1.0)) as u8;
                out.push(Point {
                    pos: [
                        (x as f32 + 0.5) / BZ as f32 * 2.0 - 1.0,
                        (front - 0.5) * 0.5,
                        (y as f32 + 0.5) / BZ as f32 * 2.0 - 1.0,
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

// --- Cyclic -----------------------------------------------------------

/// Cells along each side of the cyclic lattice.
const CG: usize = 64;
const CPLANE: usize = CG * CG;
const CCELLS: usize = CG * CG * CG;

/// The cyclic cellular automaton (Fisch, Gravner and Griffeath, 1991):
/// states in a ring, each one waiting to be eaten by the next.
///
/// A cell in state *k* becomes *k + 1* as soon as enough of its
/// neighbours already are; the states wrap round, so nothing is ever
/// finished and nothing has an equilibrium to fall into. Started from
/// pure noise it goes through three phases nobody put in it: the noise
/// clears into *debris*, the debris organises into expanding
/// *droplets*, and the droplets are eventually all consumed by
/// *spirals* — self-sustaining cores that, once formed, cannot be
/// destroyed, because a spiral's own wave comes back round to feed it.
/// The lattice ends up tiled with them, turning for ever.
///
/// In three dimensions the spiral core is a line rather than a point
/// and the waves are scrolls: nested shells rolling out of a filament,
/// which is what an arrhythmic heart does and what the
/// Belousov–Zhabotinsky reaction does in a tall jar. Only the crest is
/// drawn — the two or three states behind the front — so the picture is
/// the wave and not the volume it is crossing.
pub struct Cyclic {
    cell: Vec<u8>,
    next: Vec<u8>,
    states: u8,
    /// Neighbours in the next state needed before a cell turns.
    threshold: u8,
    /// Generations owed, so the pace is in generations a second rather
    /// than one a frame whatever the frame rate is.
    owed: f32,
    time: f32,
    since_kick: f32,
    rng: Rng,
}

impl Cyclic {
    /// States in the ring. Twelve is well past the threshold where
    /// spirals form and slow enough to see a wave arrive.
    const STATES: u8 = 12;

    pub fn new() -> Self {
        let mut s = Self {
            cell: vec![0; CCELLS],
            next: vec![0; CCELLS],
            states: Self::STATES,
            threshold: 1,
            owed: 0.0,
            time: 0.0,
            since_kick: 10.0,
            rng: Rng::new(0xC7_C11C),
        };
        s.scatter(0, CG);
        // Run it far enough that the first frame is already past the
        // debris: an empty-looking lattice of noise is not the system.
        for _ in 0..120 {
            s.generation();
        }
        s
    }

    /// Fill a cube of the lattice with noise, which is how a spiral is
    /// started: a spiral needs a defect to wind round, and noise is
    /// nothing but defects.
    fn scatter(&mut self, corner: usize, side: usize) {
        for z in 0..side {
            for y in 0..side {
                for x in 0..side {
                    let i = (corner + x) % CG
                        + ((corner + y) % CG) * CG
                        + ((corner + z) % CG) * CPLANE;
                    self.cell[i] = (self.rng.next() % self.states as u64) as u8;
                }
            }
        }
    }

    /// One turn of the ring over the whole lattice.
    fn generation(&mut self) {
        for z in 0..CG {
            for y in 0..CG {
                for x in 0..CG {
                    let i = x + y * CG + z * CPLANE;
                    let me = self.cell[i];
                    let eats = (me + 1) % self.states;
                    let mut count = 0;
                    for (axis, c) in [x, y, z].into_iter().enumerate() {
                        let stride = [1, CG, CPLANE][axis];
                        let up = i + stride - if c + 1 == CG { CG * stride } else { 0 };
                        let down = i + if c == 0 { CG * stride } else { 0 } - stride;
                        count += u8::from(self.cell[up] == eats);
                        count += u8::from(self.cell[down] == eats);
                    }
                    self.next[i] = if count >= self.threshold { eats } else { me };
                }
            }
        }
        std::mem::swap(&mut self.cell, &mut self.next);
    }
}

impl Default for Cyclic {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Cyclic {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        // How many neighbours it takes. One is the loose rule that
        // fills the lattice with spirals; two is grudging, and the
        // waves come out blockier and slower, with fewer cores.
        self.threshold = if drive.audio && drive.bands[3] > 0.6 { 2 } else { 1 };
        // Generations a second. A wave crossing the lattice in about
        // five seconds is a pace the eye can follow round a spiral.
        let pace = if drive.audio { 6.0 + 16.0 * drive.level } else { 12.0 };
        self.owed += pace * dt;
        // Capped, so a frame that took too long does not then take
        // even longer catching up.
        let generations = (self.owed as usize).min(6);
        self.owed -= generations as f32;
        for _ in 0..generations {
            self.generation();
        }
        // The kick scatters a corner of the lattice, which is a fresh
        // patch of defects — some of them wind up into new cores, and
        // the waves already crossing it roll straight over the rest.
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.5 {
            self.since_kick = 0.0;
            let corner = (self.rng.next() as usize) % CG;
            self.scatter(corner, CG / 4);
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        // The crest: the states just behind the front. Drawing every
        // cell would draw the solid lattice, and the wave is the thing
        // that is moving.
        const CREST: u8 = 3;
        let front: Vec<usize> = (0..CCELLS).filter(|&i| self.cell[i] < CREST).collect();
        if front.is_empty() {
            return;
        }
        // A slot's worth however many are on the crest: a stride across
        // them when there are more, a jittered repeat when there are
        // fewer. The same policy the loader uses, applied here so the
        // count is right before the fit sees it.
        let mut rng = Rng::new(0xC7_5EED);
        for i in 0..POINTS {
            let pick = if front.len() >= POINTS {
                front[(i as u64 * front.len() as u64 / POINTS as u64) as usize]
            } else {
                front[i % front.len()]
            };
            let (x, y, z) = (pick % CG, (pick / CG) % CG, pick / CPLANE);
            let j: [f32; 3] = if front.len() >= POINTS {
                [0.0; 3]
            } else {
                std::array::from_fn(|_| rng.f32() - 0.5)
            };
            // Brightest at the very front, falling off behind it, so a
            // scroll reads as a wave with a direction.
            let shade = (255.0 - 55.0 * self.cell[pick] as f32) as u8;
            out.push(Point {
                pos: [
                    ((x as f32 + 0.5 + j[0] * 0.8) / CG as f32 - 0.5) * 2.0,
                    ((y as f32 + 0.5 + j[1] * 0.8) / CG as f32 - 0.5) * 2.0,
                    ((z as f32 + 0.5 + j[2] * 0.8) / CG as f32 - 0.5) * 2.0,
                ],
                normal: [0.0; 3],
                color: [shade, shade, shade],
            });
        }
    }
}

// --- Tangle -----------------------------------------------------------

/// Beads along the rod. Coarse on purpose, twice over: a rod's
/// thickness has to be a few links, or it can fold tighter than it is
/// wide and no contact solver can talk it out of that; and a rope thin
/// enough to have a thousand beads in this box is also thin enough to
/// pack the whole of itself into a marble, which is what it does.
const BEADS: usize = 256;
/// Points drawn per bead — a length of tube, sixteen round by sixteen
/// along, so the rod is a rope and not a dotted line.
const STRAND: usize = POINTS / BEADS;
const ROUND: usize = 16;
/// Cells along each side of the grid the rod checks itself against.
/// One cell is a clear distance or more, so a bead's own cell and the
/// twenty-six round it hold everything it could be touching.
const KG: usize = 12;

/// One long elastic rod, loose in a flow, tying itself in knots.
///
/// A rod is the one thing in this list with no resolution: it is a
/// single curve, and everything interesting about it is in how it
/// bends rather than in how many pieces it has. Stretching is stiff
/// beyond any use — a rope does not get longer — so the length is held
/// as a *constraint* and satisfied by moving the beads (Jakobsen,
/// 2001), which cannot add energy and so is stable at any step;
/// bending is held the same way, as a constraint on the distance
/// across three beads, which is the discrete rod's curvature
/// (Bergou et al., 2008) written as something the same solver can do.
///
/// What it needs beyond that is to know it is there: a rod with no
/// self-repulsion passes through itself, and then it cannot knot, it
/// can only look as though it has. So every bead is put on a grid each
/// frame and pushed off the ones too close to it that are not its own
/// neighbours along the rod. That is the difference between a tangle
/// and a scribble.
pub struct Tangle {
    pos: Vec<[f32; 3]>,
    /// Where it was last frame; the velocity is the difference.
    was: Vec<[f32; 3]>,
    /// Bead lists per grid cell, rebuilt each frame.
    grid: Vec<Vec<u32>>,
    time: f32,
    since_kick: f32,
    whip: f32,
}

impl Tangle {
    /// The rest length of a link. The rope is about ten box widths
    /// long and a fifteenth of one thick, which are the proportions
    /// that let it fill the box when it coils: a rope's tangle takes
    /// up the square of its thickness times its length, and a thinner
    /// one at this length simply disappears into a knot in the middle.
    const LINK: f32 = 20.0 / BEADS as f32;
    /// How close two beads that are not neighbours may come — the
    /// rope's own thickness.
    const CLEAR: f32 = Self::LINK * 2.0;
    /// How far along the rod a bead has to be before it counts as
    /// something to bump into. It has to be enough further than
    /// [`Self::CLEAR`] that an ordinary bend does not set the two
    /// against each other: at five links apart, two beads are two and
    /// a half times the clear distance apart on a straight run.
    const OWN: usize = 4;
    /// Relaxation passes a frame.
    const PASSES: usize = 10;

    pub fn new() -> Self {
        let mut pos = Vec::with_capacity(BEADS);
        // Laid out as a loose helix rather than a straight line: a
        // straight rod in a symmetric flow has nothing to break its
        // symmetry with, and takes far too long to start moving.
        for i in 0..BEADS {
            let t = i as f32 / (BEADS - 1) as f32;
            let a = t * std::f32::consts::TAU * 6.0;
            let (sa, ca) = a.sin_cos();
            pos.push([ca * 0.5, (t - 0.5) * 1.4, sa * 0.5]);
        }
        // Scaled to the length the links will hold it at. A rod laid
        // out longer than its own rest length spends the first minute
        // being hauled in a bead at a time, because a constraint pass
        // moves a correction one link along the rod and this one is
        // four thousand links.
        let laid: f32 = (0..BEADS - 1)
            .map(|i| {
                let (a, b) = (pos[i], pos[i + 1]);
                ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
            })
            .sum();
        let fit = Self::LINK * (BEADS - 1) as f32 / laid;
        for p in &mut pos {
            for c in p.iter_mut() {
                *c *= fit;
            }
        }
        Self {
            was: pos.clone(),
            pos,
            grid: vec![Vec::new(); KG * KG * KG],
            time: 0.0,
            since_kick: 10.0,
            whip: 0.0,
        }
    }

    fn cell_of(p: [f32; 3]) -> usize {
        let c: [usize; 3] = std::array::from_fn(|k| {
            (((p[k] * 0.5 + 0.5) * KG as f32) as isize).clamp(0, KG as isize - 1) as usize
        });
        c[0] + c[1] * KG + c[2] * KG * KG
    }

    /// Pull two beads to a distance, half of the correction each.
    fn hold_at(&mut self, a: usize, b: usize, rest: f32, share: f32) {
        let (pa, pb) = (self.pos[a], self.pos[b]);
        let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len < 1e-7 {
            return;
        }
        let pull = (len - rest) / len * 0.5 * share;
        for (k, d) in d.iter().enumerate() {
            self.pos[a][k] += d * pull;
            self.pos[b][k] -= d * pull;
        }
    }

    /// Push apart every pair of beads that are too close and are not
    /// each other's neighbours along the rod.
    fn keep_clear(&mut self) {
        for c in &mut self.grid {
            c.clear();
        }
        for (i, p) in self.pos.iter().enumerate() {
            self.grid[Self::cell_of(*p)].push(i as u32);
        }
        // The grid is one clear-distance across a cell or more, so a
        // bead's own cell and the twenty-six round it hold everything
        // that could be touching it.
        let mut shove: Vec<[f32; 3]> = vec![[0.0; 3]; BEADS];
        for (i, away) in shove.iter_mut().enumerate() {
            let p = self.pos[i];
            let base: [isize; 3] = std::array::from_fn(|k| {
                (((p[k] * 0.5 + 0.5) * KG as f32) as isize).clamp(0, KG as isize - 1)
            });
            for dz in -1isize..=1 {
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        let c: [isize; 3] =
                            [base[0] + dx, base[1] + dy, base[2] + dz];
                        if c.iter().any(|&v| v < 0 || v >= KG as isize) {
                            continue;
                        }
                        let cell = c[0] as usize + c[1] as usize * KG + c[2] as usize * KG * KG;
                        for &j in &self.grid[cell] {
                            let j = j as usize;
                            // Its own stretch of rod is *supposed* to
                            // be that close.
                            if j <= i + Self::OWN && i <= j + Self::OWN {
                                continue;
                            }
                            let q = self.pos[j];
                            let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                            let l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                            if !(1e-12..Self::CLEAR * Self::CLEAR).contains(&l2) {
                                continue;
                            }
                            let l = l2.sqrt();
                            let push = (Self::CLEAR - l) / l * 0.5;
                            for (k, d) in d.iter().enumerate() {
                                away[k] += d * push;
                            }
                        }
                    }
                }
            }
        }
        // Capped: in a knot a bead can be crowded by a dozen others at
        // once, and the sum of a dozen shoves is a jump the link pass
        // afterwards cannot undo in the passes it has — which shows up
        // as a rope that gets slightly longer every frame.
        let most = Self::LINK * 0.8;
        for (p, s) in self.pos.iter_mut().zip(&shove) {
            let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
            let scale = if len > most { most / len } else { 1.0 };
            for k in 0..3 {
                p[k] += s[k] * scale;
            }
        }
    }
}

impl Default for Tangle {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Tangle {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        self.whip *= (-dt * 2.5).exp();
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.4 {
            self.since_kick = 0.0;
            self.whip = 1.0;
        }
        // The same Arnold–Beltrami–Childress flow the cloth hangs in:
        // divergence-free, and chaotic in its streamlines, which is
        // what winds a rod round itself rather than merely waving it.
        let strength = if drive.audio { 0.14 + 0.4 * drive.level } else { 0.32 } + 0.7 * self.whip;
        let t = self.time * 0.4;
        for i in 0..BEADS {
            let p = self.pos[i];
            let w = Cloth::wind(p, t);
            // A soft wall rather than a pull towards the middle: a
            // spring to the centre balls the whole rope up at the
            // origin, because eight box-widths of rope will happily
            // pack into a sphere the size of a marble and then nothing
            // can get it out again.
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-6);
            let over = (r - 0.85).max(0.0);
            let next: [f32; 3] = std::array::from_fn(|k| {
                let a = w[k] * strength - p[k] / r * over * 6.0;
                p[k] + (p[k] - self.was[i][k]) * 0.985 + a * dt * dt
            });
            self.was[i] = p;
            self.pos[i] = next;
        }
        // How straight it wants to be. Stiff, it sweeps in long
        // curves; slack, it folds up small and knots.
        let bend = if drive.audio { 0.55 + 0.42 * drive.bands[2] } else { 0.85 };
        // Shoved apart first and pulled back to length after, in that
        // order: a shove is a displacement with nothing restoring the
        // links behind it, so a solve that ends on one ends with a
        // rope slightly longer than it was, every frame, for ever.
        for pass in 0..Self::PASSES {
            // Twice over the run of passes, so a contact found on the
            // first one still has passes left to settle into.
            if pass == 0 || pass == Self::PASSES / 2 {
                self.keep_clear();
            }
            for i in 0..BEADS - 2 {
                // Across three beads: the discrete rod's curvature,
                // held loosely so it can still bend, which is the
                // whole point of a rod.
                self.hold_at(i, i + 2, Self::LINK * 2.0 * bend, 0.2);
            }
            // Alternating direction, because a sweep along a chain
            // carries its corrections the way it is going and leaves
            // the far end for the next one: four thousand links is a
            // long way for a correction to travel one link at a time.
            if pass % 2 == 0 {
                for i in 0..BEADS - 1 {
                    self.hold_at(i, i + 1, Self::LINK, 1.0);
                }
            } else {
                for i in (0..BEADS - 1).rev() {
                    self.hold_at(i, i + 1, Self::LINK, 1.0);
                }
            }
        }
        for p in &mut self.pos {
            for c in p.iter_mut() {
                *c = c.clamp(-1.0, 1.0);
            }
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for i in 0..BEADS {
            let a = self.pos[i];
            let b = self.pos[(i + 1).min(BEADS - 1)];
            let along = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let behind = self.pos[i.saturating_sub(1)];
            let span = [b[0] - behind[0], b[1] - behind[1], b[2] - behind[2]];
            let reach = (span[0] * span[0] + span[1] * span[1] + span[2] * span[2]).sqrt();
            let tangent: [f32; 3] = if reach > 1e-7 {
                std::array::from_fn(|k| span[k] / reach)
            } else {
                [0.0, 1.0, 0.0]
            };
            let (u, v) = Slime::frame(tangent);
            // Bright where it is bent hardest: the knots and the kinks
            // are what there is to look at, and a straight run is not.
            let straight = reach / (2.0 * Self::LINK);
            let shade = (100.0 + 155.0 * (1.0 - straight).clamp(0.0, 1.0).sqrt()) as u8;
            let thick = Self::CLEAR * 0.45;
            for q in 0..STRAND {
                let (step, turn) = (q / ROUND, q % ROUND);
                let t = step as f32 / (STRAND / ROUND) as f32;
                let angle = turn as f32 / ROUND as f32 * std::f32::consts::TAU;
                let (sa, ca) = angle.sin_cos();
                out.push(Point {
                    pos: std::array::from_fn(|k| {
                        (a[k] + along[k] * t + (u[k] * ca + v[k] * sa) * thick).clamp(-1.0, 1.0)
                    }),
                    normal: [0.0; 3],
                    color: [shade, shade, shade],
                });
            }
        }
    }
}

// --- Crystal ----------------------------------------------------------

/// Cells across the hexagonal lattice. Its square is a slot.
const HEX: usize = 256;

/// Reiter's snowflake (2005): a snow crystal grown one cell at a time
/// on a hexagonal lattice.
///
/// It is a cellular automaton with real-valued cells, and it does not
/// know what a snowflake looks like. Each cell holds water. Cells that
/// are already ice, or next to ice, stop taking part in the diffusion
/// and get a constant trickle of vapour added instead; everything else
/// diffuses. That is all of it — and out of it come the plates, the
/// sectored plates, the stellar dendrites and the needles, sorted by
/// the background vapour exactly as they are sorted by humidity in the
/// Nakaya diagram that real crystals obey.
///
/// The six-fold symmetry is not imposed anywhere: it is in the lattice,
/// and everything else follows from the diffusion being screened at the
/// tips in the same way a diffusion-limited aggregate's are. One
/// crystal grows until
/// it reaches the edge of the plate, then it falls and the next one
/// starts in different air.
pub struct Crystal {
    /// Water per cell; at or above one it is ice.
    s: Vec<f32>,
    /// Next iteration's cells.
    next: Vec<f32>,
    /// Cells that are ice or touching it, so out of the diffusion.
    held: Vec<bool>,
    /// The background vapour this crystal is growing in, which is what
    /// decides its habit.
    beta: f32,
    /// Vapour added to a cell on the boundary each iteration.
    gamma: f32,
    /// How far the ice has reached from the middle, in cells.
    reach: f32,
    /// Seconds left of the pause on a finished flake.
    holding: f32,
    time: f32,
    since_kick: f32,
    rng: Rng,
}

impl Crystal {
    /// Diffusion rate. Reiter runs it at one; the interesting habits
    /// are all found by moving the vapour, not this.
    const ALPHA: f32 = 1.0;
    /// How far out the ice is allowed to reach before the crystal is
    /// done, as a share of the plate.
    const FULL: f32 = 0.44;
    /// Seconds a finished flake is left on the plate before the next
    /// one starts. Growing it is the interesting part, but the grown
    /// crystal is the thing it was grown for, and without this the
    /// picture is never the flake, only the flake appearing.
    const HOLD: f32 = 2.5;

    pub fn new() -> Self {
        let mut s = Self {
            s: vec![0.0; HEX * HEX],
            next: vec![0.0; HEX * HEX],
            held: vec![false; HEX * HEX],
            beta: 0.45,
            gamma: 0.0004,
            reach: 1.0,
            holding: 0.0,
            time: 0.0,
            since_kick: 10.0,
            rng: Rng::new(0x50_0F_1A_CE),
        };
        s.nucleate();
        s
    }

    /// Fill the plate with vapour and drop one seed of ice in it.
    fn nucleate(&mut self) {
        self.s.fill(self.beta);
        self.s[HEX / 2 + (HEX / 2) * HEX] = 1.0;
        self.reach = 1.0;
    }

    /// The six neighbours of a cell on a hexagonal lattice stored in
    /// offset rows — which is a square array with every other row
    /// shifted half a cell, so a hexagon's six neighbours are four
    /// across and two along.
    fn neighbours(x: usize, y: usize) -> [Option<usize>; 6] {
        let odd = y & 1 == 1;
        let lean: isize = if odd { 0 } else { -1 };
        let at = |dx: isize, dy: isize| -> Option<usize> {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if nx < 0 || ny < 0 || nx >= HEX as isize || ny >= HEX as isize {
                None
            } else {
                Some(nx as usize + ny as usize * HEX)
            }
        };
        [
            at(-1, 0),
            at(1, 0),
            at(lean, -1),
            at(lean + 1, -1),
            at(lean, 1),
            at(lean + 1, 1),
        ]
    }

    /// One iteration of the automaton.
    fn grow(&mut self) {
        // Which cells are out of the diffusion: ice, or next to it.
        for y in 0..HEX {
            for x in 0..HEX {
                let i = x + y * HEX;
                let mut held = self.s[i] >= 1.0;
                if !held {
                    for n in Self::neighbours(x, y).into_iter().flatten() {
                        if self.s[n] >= 1.0 {
                            held = true;
                            break;
                        }
                    }
                }
                self.held[i] = held;
            }
        }
        // Split each cell's water in two: the part that diffuses, and
        // the part that has been taken out of the diffusion because it
        // is ice or next to it. A held cell holds *no* diffusing water,
        // which is what starves the crevices and lets the tips run
        // away; what it does still do is take in the diffusing water of
        // its free neighbours, and that is how the crystal grows.
        for y in 0..HEX {
            for x in 0..HEX {
                let i = x + y * HEX;
                let free = if self.held[i] { 0.0 } else { self.s[i] };
                let mut sum = 0.0;
                for n in Self::neighbours(x, y) {
                    // Off the plate is more of the same air, so an edge
                    // cell is not starved by its own edge.
                    sum += match n {
                        Some(n) if !self.held[n] => self.s[n],
                        Some(_) => 0.0,
                        None => self.beta,
                    };
                }
                let mean = sum / 6.0;
                let held = if self.held[i] { self.s[i] + self.gamma } else { 0.0 };
                self.next[i] = free + Self::ALPHA * 0.5 * (mean - free) + held;
            }
        }
        std::mem::swap(&mut self.s, &mut self.next);
    }

    /// Where a cell sits on the plate: rows half a cell apart, which is
    /// what makes the lattice hexagonal rather than square.
    fn place(x: usize, y: usize) -> [f32; 2] {
        let step = 2.0 / HEX as f32;
        let shift = if y & 1 == 1 { 0.5 } else { 0.0 };
        [
            (x as f32 + shift) * step - 1.0,
            (y as f32 - HEX as f32 * 0.5) * step * 0.866_025_4,
        ]
    }

    /// How far the ice reaches, in cells from the middle.
    fn measure(&self) -> f32 {
        let mut reach: f32 = 0.0;
        for y in 0..HEX {
            for x in 0..HEX {
                if self.s[x + y * HEX] >= 1.0 {
                    let dx = x as f32 - HEX as f32 * 0.5;
                    let dy = (y as f32 - HEX as f32 * 0.5) * 0.866_025_4;
                    reach = reach.max((dx * dx + dy * dy).sqrt());
                }
            }
        }
        reach
    }
}

impl Default for Crystal {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Crystal {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        // The air this one is growing in. Low vapour grows a plate,
        // high vapour grows a star — the same sorting the Nakaya
        // diagram makes out of humidity.
        if drive.audio {
            self.beta = 0.35 + 0.28 * drive.bands[2];
            self.gamma = 0.0001 + 0.0016 * drive.bands[3];
        }
        let kick = drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.6;
        if kick {
            self.since_kick = 0.0;
        }
        // A finished flake stays a moment, and a kick cuts the pause
        // short: nothing is growing, so there is nothing to interrupt.
        if self.holding > 0.0 {
            self.holding -= dt;
            if kick {
                self.holding = 0.0;
            }
            if self.holding <= 0.0 {
                self.nucleate();
            }
            return;
        }
        let iterations = if drive.audio { 5 + (10.0 * drive.level) as usize } else { 10 };
        for _ in 0..iterations {
            self.grow();
        }
        self.reach = self.measure();
        // Grown out, or knocked off its plate: the next crystal starts
        // in whatever air the music has left behind.
        if self.reach > HEX as f32 * Self::FULL || (kick && self.reach > HEX as f32 * 0.12) {
            if !drive.audio {
                // Left alone it still works through the habits rather
                // than growing the same flake for ever.
                self.beta = 0.34 + 0.26 * self.rng.f32();
                self.gamma = 0.0001 + 0.0014 * self.rng.f32();
            }
            self.holding = Self::HOLD;
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        // Two thirds of the slot to the ice and a third to the air it
        // grew in. Spending a point per cell would spend all of them
        // on vapour — the flake is a few thousand cells of sixty-five
        // thousand — and draw a plate with a mark on it.
        const PLATE: usize = POINTS / 3;
        let mut rng = Rng::new(0x1CE_5EED);
        for i in 0..PLATE {
            let cell = (i as u64 * (HEX * HEX) as u64 / PLATE as u64) as usize;
            let (x, y) = (cell % HEX, cell / HEX);
            let [px, pz] = Self::place(x, y);
            let shade = (22.0 + 64.0 * self.s[cell].min(1.0)) as u8;
            out.push(Point {
                pos: [px.clamp(-1.0, 1.0), 0.0, pz.clamp(-1.0, 1.0)],
                normal: [0.0; 3],
                color: [shade, shade, shade],
            });
        }
        let ice: Vec<usize> = (0..HEX * HEX).filter(|&i| self.s[i] >= 1.0).collect();
        for i in 0..POINTS - PLATE {
            // A stride across the ice when there is more of it than
            // there are points left, a jittered repeat when there is
            // less — the loader's own policy, so the count is right
            // before the fit sees it.
            let want = POINTS - PLATE;
            let pick = if ice.len() >= want {
                ice[(i as u64 * ice.len() as u64 / want as u64) as usize]
            } else {
                ice[i % ice.len()]
            };
            let (x, y) = (pick % HEX, pick / HEX);
            let [px, pz] = Self::place(x, y);
            let thick = self.s[pick] - 1.0;
            let jitter = if ice.len() >= want { 0.0 } else { 1.0 };
            let step = 2.0 / HEX as f32;
            out.push(Point {
                pos: [
                    (px + (rng.f32() - 0.5) * step * jitter).clamp(-1.0, 1.0),
                    0.05 + 0.3 * thick.min(1.5),
                    (pz + (rng.f32() - 0.5) * step * jitter).clamp(-1.0, 1.0),
                ],
                normal: [0.0; 3],
                color: {
                    let shade = (150.0 + 105.0 * thick.min(1.0)) as u8;
                    [shade, shade, shade]
                },
            });
        }
    }
}

// --- Vortex -----------------------------------------------------------

/// Filaments in the box.
const RINGS: usize = 8;
/// Nodes around each filament.
const NODES: usize = 128;
/// Points drawn per node: a short tube, so a filament is a rope and
/// not a dotted line.
const TUBE: usize = POINTS / (RINGS * NODES);

/// Vortex filaments: the thin cores that smoke rings are made of,
/// moving each other about.
///
/// Vorticity in an ideal fluid does not spread, it is carried — a
/// theorem of Helmholtz's from 1858 — so a fluid whose spin is all
/// concentrated in a few thin loops stays that way, and the whole flow
/// can be integrated as those loops alone (Rosenhead, 1930; Leonard,
/// 1980). Each piece of filament moves in the flow every other piece
/// induces, by Biot–Savart, and that is the entire simulation: no grid,
/// no pressure solve, no tracers.
///
/// What it buys is the behaviour no grid fluid at this resolution can
/// show: rings that shrink as they speed up, catch the one in front,
/// thread through it and swap places — leapfrogging, which Helmholtz
/// predicted and which is still startling to watch — and filaments that
/// stretch, wind round each other and go on stretching.
pub struct Vortex {
    /// Node positions, filament by filament.
    node: Vec<[f32; 3]>,
    /// Velocity read off the others, kept between the two half-steps.
    vel: Vec<[f32; 3]>,
    /// Circulation of each filament, signed.
    gamma: [f32; RINGS],
    time: f32,
    since_kick: f32,
    rng: Rng,
}

impl Vortex {
    /// The core radius that keeps Biot–Savart from dividing by zero at
    /// the filament itself. A real core has a thickness too, so this is
    /// not only a numerical dodge.
    const CORE: f32 = 0.045;

    pub fn new() -> Self {
        let mut s = Self {
            node: vec![[0.0; 3]; RINGS * NODES],
            vel: vec![[0.0; 3]; RINGS * NODES],
            gamma: [0.0; RINGS],
            time: 0.0,
            since_kick: 10.0,
            rng: Rng::new(0x1207_7EC5),
        };
        for r in 0..RINGS {
            s.lay(r);
        }
        s
    }

    /// Lay filament `r` out as a ring, somewhere, facing somewhere.
    fn lay(&mut self, r: usize) {
        let axis = self.rng.on_sphere();
        let (u, v) = Slime::frame(axis);
        let centre: [f32; 3] = std::array::from_fn(|_| (self.rng.f32() - 0.5) * 0.9);
        let radius = 0.16 + 0.22 * self.rng.f32();
        for n in 0..NODES {
            let a = n as f32 / NODES as f32 * std::f32::consts::TAU;
            let (sa, ca) = a.sin_cos();
            self.node[r * NODES + n] =
                std::array::from_fn(|k| centre[k] + (u[k] * ca + v[k] * sa) * radius);
            self.vel[r * NODES + n] = [0.0; 3];
        }
        self.gamma[r] = if self.rng.f32() < 0.5 { -1.0 } else { 1.0 } * (0.6 + 0.6 * self.rng.f32());
    }

    /// The velocity every filament induces at every node, by
    /// Biot–Savart with a desingularised kernel (Rosenhead–Moore).
    fn induce(&mut self, strength: f32) {
        let total = RINGS * NODES;
        let a2 = Self::CORE * Self::CORE;
        for i in 0..total {
            let p = self.node[i];
            let mut v = [0.0f32; 3];
            for r in 0..RINGS {
                let g = self.gamma[r] * strength * 0.08;
                for n in 0..NODES {
                    let j = r * NODES + n;
                    let k = r * NODES + (n + 1) % NODES;
                    let (a, b) = (self.node[j], self.node[k]);
                    // The segment, and the vector from its middle.
                    let dl = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let m = [
                        p[0] - (a[0] + b[0]) * 0.5,
                        p[1] - (a[1] + b[1]) * 0.5,
                        p[2] - (a[2] + b[2]) * 0.5,
                    ];
                    let r2 = m[0] * m[0] + m[1] * m[1] + m[2] * m[2] + a2;
                    let scale = g / (r2 * r2.sqrt());
                    v[0] += (dl[1] * m[2] - dl[2] * m[1]) * scale;
                    v[1] += (dl[2] * m[0] - dl[0] * m[2]) * scale;
                    v[2] += (dl[0] * m[1] - dl[1] * m[0]) * scale;
                }
            }
            self.vel[i] = v;
        }
    }

    /// Damp the wiggles finer than the core.
    ///
    /// Biot–Savart on a discretised filament is unstable at wavelengths
    /// shorter than the core radius — they are not physical, the core
    /// is where the model stops resolving, and left alone they grow
    /// until the filament is noise. One light Laplacian pass a frame
    /// takes them out and barely touches the long waves that are the
    /// motion worth watching; every filament method does some version
    /// of this.
    fn smooth(&mut self) {
        const LAMBDA: f32 = 0.08;
        let mut ring = vec![[0.0f32; 3]; NODES];
        for r in 0..RINGS {
            let base = r * NODES;
            for (n, slot) in ring.iter_mut().enumerate() {
                let a = self.node[base + (n + NODES - 1) % NODES];
                let b = self.node[base + n];
                let c = self.node[base + (n + 1) % NODES];
                *slot = std::array::from_fn(|k| b[k] + LAMBDA * ((a[k] + c[k]) * 0.5 - b[k]));
            }
            self.node[base..base + NODES].copy_from_slice(&ring);
        }
    }

    /// Spread the nodes of each filament out along it again.
    ///
    /// A stretching filament drags its nodes into bunches, and a bunch
    /// resolves the curve where nothing is happening while leaving the
    /// part that is stretching with nothing on it. Resampling to equal
    /// arc length every frame is what keeps a filament a filament.
    fn respace(&mut self) {
        let mut arc = vec![0.0f32; NODES + 1];
        let mut ring = vec![[0.0f32; 3]; NODES];
        for r in 0..RINGS {
            let base = r * NODES;
            for n in 0..NODES {
                let a = self.node[base + n];
                let b = self.node[base + (n + 1) % NODES];
                let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                arc[n + 1] = arc[n] + (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            }
            let total = arc[NODES];
            if total < 1e-5 {
                continue;
            }
            let mut seg = 0usize;
            for (n, slot) in ring.iter_mut().enumerate() {
                let want = total * n as f32 / NODES as f32;
                while seg + 1 < NODES && arc[seg + 1] < want {
                    seg += 1;
                }
                let span = (arc[seg + 1] - arc[seg]).max(1e-6);
                let t = ((want - arc[seg]) / span).clamp(0.0, 1.0);
                let a = self.node[base + seg];
                let b = self.node[base + (seg + 1) % NODES];
                *slot = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t);
            }
            self.node[base..base + NODES].copy_from_slice(&ring);
        }
    }
}

impl Default for Vortex {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulation for Vortex {
    fn step(&mut self, dt: f32, drive: &Drive) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        self.time += dt;
        self.since_kick += dt;
        // Circulation is the one number a filament has, and it is the
        // pace of everything: a loud passage runs the rings fast.
        let strength = if drive.audio { 0.55 + 1.1 * drive.level } else { 0.9 };
        self.induce(strength);
        for i in 0..RINGS * NODES {
            for k in 0..3 {
                self.node[i][k] += self.vel[i][k] * dt;
            }
        }
        self.respace();
        self.smooth();
        // A filament driven out of the box is pulled back rather than
        // clipped: a ring cut off at a wall stops being a ring.
        for p in &mut self.node {
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            if r > 0.92 {
                let pull = (r - 0.92) * 2.4 * dt * 60.0;
                for c in p.iter_mut() {
                    *c -= *c / r * pull;
                }
            }
        }
        // The kick throws a new ring in, over the oldest one, so the
        // box keeps being given something to leapfrog with.
        if drive.audio && drive.bands[0] > 0.5 && self.since_kick > 0.5 {
            self.since_kick = 0.0;
            let r = (self.rng.next() as usize) % RINGS;
            self.lay(r);
        }
        // A filament that has wound itself into a knot too fine to
        // draw is laid out again. Real ones reconnect instead; this is
        // the cheap version of the same ending.
        for r in 0..RINGS {
            let base = r * NODES;
            let mut length = 0.0;
            for n in 0..NODES {
                let a = self.node[base + n];
                let b = self.node[base + (n + 1) % NODES];
                let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                length += (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            }
            if !length.is_finite() || !(0.1..=14.0).contains(&length) {
                self.lay(r);
            }
        }
    }

    fn points(&self, out: &mut Vec<Point>) {
        out.clear();
        for r in 0..RINGS {
            let base = r * NODES;
            for n in 0..NODES {
                let a = self.node[base + n];
                let b = self.node[base + (n + 1) % NODES];
                let along = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let len = (along[0] * along[0] + along[1] * along[1] + along[2] * along[2]).sqrt();
                let tangent: [f32; 3] = if len > 1e-6 {
                    std::array::from_fn(|k| along[k] / len)
                } else {
                    [0.0, 1.0, 0.0]
                };
                let (u, v) = Slime::frame(tangent);
                let speed = {
                    let w = self.vel[base + n];
                    (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt()
                };
                let shade = (100.0 + 155.0 * (speed * 0.5).min(1.0)) as u8;
                // A short tube: a few steps along the segment, a few
                // round it, which is exactly TUBE points.
                let around = 8usize;
                let steps = TUBE / around;
                let thick = 0.012;
                for s in 0..steps {
                    let t = s as f32 / steps as f32;
                    let centre: [f32; 3] = std::array::from_fn(|k| a[k] + along[k] * t);
                    for q in 0..around {
                        let ang = q as f32 / around as f32 * std::f32::consts::TAU;
                        let (sa, ca) = ang.sin_cos();
                        out.push(Point {
                            pos: std::array::from_fn(|k| {
                                (centre[k] + (u[k] * ca + v[k] * sa) * thick).clamp(-1.0, 1.0)
                            }),
                            normal: [0.0; 3],
                            color: [shade, shade, shade],
                        });
                    }
                }
            }
        }
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

    /// The automaton neither dies nor floods over a run, always hands
    /// over a full slot — and, the part that matters for a *live*
    /// slot, keeps moving. Most of the catalogued three-dimensional
    /// rules grow into a shape and then hold it for ever; Clouds
    /// settles down to sixty changed cells a generation out of a
    /// hundred and ten thousand, which is a still image. The shipped
    /// rule turns over a good share of the lattice every generation,
    /// and any rule that does stop is reseeded.
    #[test]
    fn life_keeps_living_and_keeps_moving() {
        let mut l = Life::new();
        let mut churn = 0.0f32;
        let mut generations = 0.0f32;
        for frame in 0..900 {
            let before = l.cells.clone();
            l.step(1.0 / 60.0, &Drive::default());
            if frame > 300 {
                churn += before.iter().zip(&l.cells).filter(|(a, b)| a != b).count() as f32;
                generations += 1.0 / 3.0;
            }
        }
        let alive = l.alive();
        let cells = LG * LG * LG;
        assert!(alive > cells / 200 && alive < cells * 9 / 10, "{alive} of {cells} alive");
        let per_generation = churn / generations;
        assert!(
            per_generation > cells as f32 / 100.0,
            "the automaton has stopped: {per_generation:.0} cells a generation"
        );
        let mut pts = Vec::new();
        l.points(&mut pts);
        box_ok(&pts);
    }

    /// A rule that freezes is started again rather than left on screen.
    /// Clouds is the example: it grows into masses and then holds them,
    /// and without this the slot would show one picture all night.
    #[test]
    fn a_frozen_automaton_is_reseeded() {
        let mut l = Life::with_rule("13-26/13-14,17-19");
        let mut reseeds = 0;
        let mut last = l.alive();
        for _ in 0..3_000 {
            l.step(1.0 / 60.0, &Drive::default());
            let now = l.alive();
            // A reseed is the only thing that can change the count by a
            // large fraction in one generation.
            if now.abs_diff(last) > last / 3 {
                reseeds += 1;
            }
            last = now;
        }
        assert!(reseeds > 0, "a rule that stops was left stopped");
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
        assert_eq!(fallback.states, 10, "the shipped rule has a decay ramp");
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

    /// The colony builds a network: the trail stops being spread
    /// evenly and becomes veins with space between them, which is the
    /// whole claim of the model. Measured as the share of the trail
    /// that is in the busiest twentieth of the cells — even spreading
    /// puts a twentieth there, a network puts most of it.
    #[test]
    fn the_slime_builds_a_network() {
        let mut sim = Slime::new();
        for _ in 0..1_200 {
            sim.step(1.0 / 60.0, &Drive::default());
        }
        // Two things have to be true at once, and only one of them is
        // obvious. The trail must concentrate — a network is veins with
        // space between them, not an even wash. But the colony must
        // also still be spread across the box, because the failure this
        // model falls into is the opposite one: every agent steers
        // towards the most trail, the most trail is wherever the agents
        // are, and if they cannot outrun their own deposit the whole
        // colony walks into a single blob. That blob concentrates the
        // trail beautifully and is not a network.
        let mut mean = [0.0f32; 3];
        for p in &sim.pos {
            for (m, c) in mean.iter_mut().zip(p) {
                *m += c / POINTS as f32;
            }
        }
        let mut spread = [0.0f32; 3];
        for p in &sim.pos {
            for (v, (c, m)) in spread.iter_mut().zip(p.iter().zip(mean)) {
                *v += (c - m) * (c - m) / POINTS as f32;
            }
        }
        // A cloud spread evenly through a box of side one has a
        // standard deviation of 1/√12, which is 0.289.
        for (axis, v) in spread.iter().enumerate() {
            assert!(v.sqrt() > 0.2, "the colony collapsed on axis {axis}: {:.3}", v.sqrt());
        }
        let mut cells = sim.trail.clone();
        cells.sort_by(f32::total_cmp);
        let total: f32 = cells.iter().sum();
        assert!(total > 1.0, "nothing was laid down: {total}");
        let busiest: f32 = cells[cells.len() - TCELLS / 20..].iter().sum();
        let share = busiest / total;
        assert!(share > 0.4, "the trail is spread evenly, not built: {share:.2}");
        let mut pts = Vec::new();
        sim.points(&mut pts);
        box_ok(&pts);
        let lit = pts.iter().filter(|p| p.color[0] > 160).count();
        assert!(lit > POINTS / 20, "no agent found a vein: {lit}");
    }

    /// Swarmalators sync when told to and do not when told not to: the
    /// order parameter, which is how aligned the phases are, is high
    /// under a positive phase coupling and low under a negative one.
    #[test]
    fn the_swarm_syncs_under_coupling_and_not_against_it() {
        let order = |sim: &Swarm| {
            let (mut c, mut s) = (0.0f32, 0.0f32);
            for t in &sim.phase {
                c += t.cos();
                s += t.sin();
            }
            (c * c + s * s).sqrt() / MATES as f32
        };
        let run = |drive: Drive| {
            let mut sim = Swarm::new();
            for _ in 0..900 {
                sim.step(1.0 / 60.0, &drive);
            }
            let mut pts = Vec::new();
            sim.points(&mut pts);
            box_ok(&pts);
            for p in &sim.pos {
                for v in p {
                    assert!(v.is_finite(), "a swarmalator left: {p:?}");
                }
            }
            order(&sim)
        };
        // Band three at full sets the phase coupling positive.
        let together =
            run(Drive { bands: [0.0, 0.0, 1.0, 0.0], level: 0.8, bar: 0.0, audio: true });
        // Band three at zero sets it negative.
        let apart = run(Drive { bands: [0.0, 0.0, 0.0, 0.0], level: 0.8, bar: 0.0, audio: true });
        assert!(together > 0.8, "a positive coupling did not sync them: {together:.2}");
        assert!(apart < together - 0.2, "a negative one synced them anyway: {apart:.2}");
    }

    /// The sheet hangs, keeps its weave and blows: the top edge stays
    /// nailed up, no link is stretched far past its rest length, and
    /// the free end moves.
    #[test]
    fn the_cloth_hangs_and_blows() {
        let mut sim = Cloth::new();
        let pinned: Vec<[f32; 3]> = sim.pos[..WEAVE].to_vec();
        let hem = sim.pos[POINTS - 1];
        for _ in 0..600 {
            sim.step(1.0 / 60.0, &Drive::default());
        }
        assert_eq!(&sim.pos[..WEAVE], &pinned[..], "the top edge came off its nail");
        let moved = (0..3).map(|k| (sim.pos[POINTS - 1][k] - hem[k]).abs()).fold(0.0f32, f32::max);
        assert!(moved > 0.02, "the sheet did not move: {moved}");
        // The weave holds. Mass-spring cloth solved in a handful of
        // passes is famously super-elastic (Provot, 1995), so what is
        // checked is the shape of the distribution rather than the
        // single worst link: the sheet should be within a sixth of its
        // rest length nearly everywhere, and nowhere torn.
        let mut lengths: Vec<f32> = Vec::with_capacity(WEAVE * (WEAVE - 1));
        for row in 0..WEAVE {
            for col in 0..WEAVE - 1 {
                let i = row * WEAVE + col;
                let d = [
                    sim.pos[i + 1][0] - sim.pos[i][0],
                    sim.pos[i + 1][1] - sim.pos[i][1],
                    sim.pos[i + 1][2] - sim.pos[i][2],
                ];
                lengths.push((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt());
            }
        }
        lengths.sort_by(f32::total_cmp);
        let n = lengths.len();
        let nearly_all = lengths[n * 999 / 1000] / Cloth::LINK;
        let worst = lengths[n - 1] / Cloth::LINK;
        assert!(nearly_all < 1.35, "the weave is stretched through: {nearly_all:.2}x");
        assert!(worst < 2.0, "a link tore: {worst:.2}x");
        assert!(lengths[n / 2] / Cloth::LINK > 0.6, "the sheet crushed: {:.2}x", lengths[n / 2] / Cloth::LINK);
        let mut pts = Vec::new();
        sim.points(&mut pts);
        box_ok(&pts);
        // A kick is a gust, and a gust moves it more than no gust does.
        let quiet: f32 = sim.pos.iter().zip(&sim.was).map(|(p, w)| (p[2] - w[2]).abs()).sum();
        let loud = Drive { bands: [1.0, 0.0, 0.0, 0.0], level: 0.5, bar: 0.0, audio: true };
        sim.step(1.0 / 60.0, &loud);
        for _ in 0..20 {
            sim.step(1.0 / 60.0, &Drive { audio: true, ..Drive::default() });
        }
        let gusted: f32 = sim.pos.iter().zip(&sim.was).map(|(p, w)| (p[2] - w[2]).abs()).sum();
        assert!(gusted > quiet, "the gust did nothing: {quiet} then {gusted}");
    }

    /// The sandpile is Abelian, which is the whole theorem: the stable
    /// arrangement it settles into does not depend on the order the
    /// grains were added in. Five thousand grains dropped at once and
    /// five hundred dropped ten times, settling in between, give the
    /// same pile cell for cell.
    #[test]
    fn the_sandpile_does_not_care_what_order_it_was_built_in() {
        let settle = |sand: &mut Sand| {
            while !sand.unstable.is_empty() {
                sand.settle();
            }
        };
        let mut all_at_once = Sand::new();
        all_at_once.drop_at(PILE / 2, PILE / 2, 5_000);
        settle(&mut all_at_once);
        let mut bit_by_bit = Sand::new();
        for _ in 0..10 {
            bit_by_bit.drop_at(PILE / 2, PILE / 2, 500);
            settle(&mut bit_by_bit);
        }
        assert_eq!(all_at_once.cells, bit_by_bit.cells, "the pile depends on its history");
        // Settled means nothing over the limit anywhere.
        assert!(all_at_once.cells.iter().all(|c| *c < 4), "a cell was left unstable");
        // And it is a pattern rather than a heap: the four heights are
        // all well represented, which a smooth pile would not be.
        for height in 0..4u32 {
            let share = all_at_once.cells.iter().filter(|c| **c == height).count();
            assert!(share > 200, "height {height} barely appears: {share}");
        }
        let mut pts = Vec::new();
        all_at_once.points(&mut pts);
        box_ok(&pts);
    }

    /// The reaction makes waves: the field is structured rather than
    /// flat, it keeps moving, and every chemical stays in its bounds.
    #[test]
    fn the_spirals_keep_turning() {
        let mut sim = Spiral::new();
        for _ in 0..200 {
            sim.step(1.0 / 60.0, &Drive::default());
        }
        for field in [&sim.a, &sim.b, &sim.c] {
            for v in field.iter() {
                assert!((0.0..=1.0).contains(v), "a chemical left its bounds: {v}");
            }
        }
        // Structure: neighbouring cells agree, which random noise would
        // not, and the field still uses its whole range.
        let mut difference = 0.0f32;
        for y in 0..BZ {
            for x in 0..BZ - 1 {
                difference += (sim.a[x + y * BZ] - sim.a[x + 1 + y * BZ]).abs();
            }
        }
        let roughness = difference / (BZ * (BZ - 1)) as f32;
        assert!(roughness < 0.1, "the field is still noise: {roughness:.3}");
        let lo = sim.a.iter().cloned().fold(f32::MAX, f32::min);
        let hi = sim.a.iter().cloned().fold(0.0f32, f32::max);
        assert!(hi - lo > 0.5, "the field went flat: {lo:.2} to {hi:.2}");
        // And it is still going: a wave has moved over the next second.
        let before = sim.a.clone();
        for _ in 0..60 {
            sim.step(1.0 / 60.0, &Drive::default());
        }
        let moved: f32 = before.iter().zip(&sim.a).map(|(x, y)| (x - y).abs()).sum::<f32>()
            / (BZ * BZ) as f32;
        assert!(moved > 0.05, "the waves stopped: {moved:.3}");
        let mut pts = Vec::new();
        sim.points(&mut pts);
        box_ok(&pts);
    }

    /// The ring only ever turns one way. Every cell either stays where
    /// it is or moves on to exactly the next state — never back, never
    /// two at once — and that is the only rule there is.
    #[test]
    fn the_ring_only_turns_forwards() {
        let mut sim = Cyclic::new();
        let before = sim.cell.clone();
        sim.generation();
        for (&was, &now) in before.iter().zip(&sim.cell) {
            let forward = (was + 1) % Cyclic::STATES;
            assert!(now == was || now == forward, "{was} became {now}");
        }
    }

    /// From noise it organises itself into waves, and then it never
    /// stops.
    ///
    /// A wave is a run of cells one state apart, so the measure is how
    /// often two neighbours are within a step of each other round the
    /// ring. In noise that is three states out of twelve, by chance.
    /// Once the waves have formed it is most of the lattice — and the
    /// lattice keeps turning, because a cyclic automaton has no still
    /// state to fall into, which is what separates it from every rule
    /// that settles.
    #[test]
    fn noise_becomes_waves_that_never_settle() {
        let in_step = |s: &Cyclic| -> f32 {
            let mut near = 0u32;
            for z in 0..CG {
                for y in 0..CG {
                    for x in 0..CG {
                        let a = s.cell[x + y * CG + z * CPLANE];
                        let b = s.cell[(x + 1) % CG + y * CG + z * CPLANE];
                        let step = (a + s.states - b) % s.states;
                        near += u32::from(step <= 1 || step == s.states - 1);
                    }
                }
            }
            near as f32 / CCELLS as f32
        };
        let mut sim = Cyclic::new();
        // A fresh lattice of noise, to measure chance from.
        sim.scatter(0, CG);
        let noise = in_step(&sim);
        assert!(
            (noise - 3.0 / Cyclic::STATES as f32).abs() < 0.02,
            "the scatter was not random: {noise}"
        );
        for _ in 0..400 {
            sim.generation();
        }
        let organised = in_step(&sim);
        assert!(organised > 0.6, "it stayed noise: {noise} then {organised}");
        let before = sim.cell.clone();
        sim.generation();
        let turning = before.iter().zip(&sim.cell).filter(|(a, b)| a != b).count();
        assert!(
            turning > CCELLS / 100,
            "the lattice came to rest, which this rule cannot do: {turning} cells turned"
        );
        let mut out = Vec::new();
        sim.points(&mut out);
        box_ok(&out);
    }

    /// A rope does not get longer, and it does not pass through itself.
    /// Those are the two claims, and both are constraints rather than
    /// forces, so both should hold exactly rather than on average.
    #[test]
    fn the_rope_keeps_its_length_and_its_distance() {
        let mut sim = Tangle::new();
        let drive = Drive { bands: [0.0, 0.0, 0.5, 0.0], level: 0.9, bar: 0.0, audio: true };
        for _ in 0..240 {
            sim.step(1.0 / 60.0, &drive);
        }
        let length: f32 = (0..BEADS - 1)
            .map(|i| {
                let (a, b) = (sim.pos[i], sim.pos[i + 1]);
                ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
            })
            .sum();
        let rest = Tangle::LINK * (BEADS - 1) as f32;
        assert!(
            (length - rest).abs() < rest * 0.05,
            "the rope stretched: {length} against {rest}"
        );
        // Every pair that is not a neighbour along the rod, checked
        // properly rather than through the grid the solver uses.
        let mut closest = f32::MAX;
        for i in 0..BEADS {
            for j in i + 4..BEADS {
                let (a, b) = (sim.pos[i], sim.pos[j]);
                let d2 = (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2);
                closest = closest.min(d2);
            }
        }
        let closest = closest.sqrt();
        assert!(
            closest > Tangle::CLEAR * 0.5,
            "the rope went through itself: two beads {closest} apart"
        );
        let mut out = Vec::new();
        sim.points(&mut out);
        box_ok(&out);
    }

    /// And it is a rod, not a chain: it resists bending, and how much
    /// is a setting. Measured as the rod's own curvature — how far
    /// three beads in a row fall short of lying straight — because
    /// that is the quantity the constraint holds and the thing that
    /// decides whether it sweeps in long curves or folds up small.
    #[test]
    fn a_stiffer_rod_bends_less() {
        let straightness = |mids: f32| -> f32 {
            let mut sim = Tangle::new();
            let drive = Drive { bands: [0.0, 0.0, mids, 0.0], level: 0.9, bar: 0.0, audio: true };
            for _ in 0..300 {
                sim.step(1.0 / 60.0, &drive);
            }
            let across: f32 = (0..BEADS - 2)
                .map(|i| {
                    let (a, b) = (sim.pos[i], sim.pos[i + 2]);
                    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
                })
                .sum();
            across / ((BEADS - 2) as f32 * 2.0 * Tangle::LINK)
        };
        let (slack, stiff) = (straightness(0.0), straightness(1.0));
        assert!(stiff > slack * 1.02, "stiffness did nothing: {slack} then {stiff}");
        assert!(slack < 0.99, "the slack rod did not bend at all: {slack}");
    }

    /// Ice appears, spreads, and when the flake has filled its plate
    /// the next one starts from a seed rather than the run stopping.
    #[test]
    fn the_crystal_grows_and_starts_again() {
        let mut sim = Crystal::new();
        let drive = Drive { bands: [0.0, 0.0, 0.5, 0.4], level: 1.0, bar: 0.0, audio: true };
        let ice = |s: &Crystal| s.s.iter().filter(|&&v| v >= 1.0).count();
        assert_eq!(ice(&sim), 1, "a crystal starts from one seed");
        for _ in 0..60 {
            sim.step(1.0 / 60.0, &drive);
        }
        let grown = ice(&sim);
        assert!(grown > 20, "the crystal did not grow: {grown} cells of ice");
        // It is a crystal, not a blob: the ice reaches out much further
        // than a disc of that many cells would.
        let disc = (grown as f32 / std::f32::consts::PI).sqrt();
        assert!(
            sim.reach > disc * 1.6,
            "the ice is a disc, not a flake: reach {} for {grown} cells",
            sim.reach
        );
        // Run it out to the edge of the plate and past it.
        let mut restarted = false;
        for _ in 0..3000 {
            sim.step(1.0 / 60.0, &drive);
            if ice(&sim) <= 1 {
                restarted = true;
                break;
            }
        }
        assert!(restarted, "the flake filled the plate and the run stopped there");
        let mut out = Vec::new();
        sim.points(&mut out);
        box_ok(&out);
    }

    /// A vortex ring on its own moves along its own axis, at a speed
    /// set by its circulation — Helmholtz's result, and the one thing
    /// every filament method has to get right before any of the rest
    /// of it means anything.
    #[test]
    fn a_ring_moves_along_its_axis() {
        let mut sim = Vortex::new();
        // One ring, flat in the x–z plane, alone in the box.
        for r in 0..RINGS {
            sim.gamma[r] = 0.0;
        }
        sim.gamma[0] = 1.0;
        for n in 0..NODES {
            let a = n as f32 / NODES as f32 * std::f32::consts::TAU;
            let (sa, ca) = a.sin_cos();
            sim.node[n] = [ca * 0.3, 0.0, sa * 0.3];
        }
        let drive = Drive::default();
        for _ in 0..60 {
            sim.step(1.0 / 60.0, &drive);
        }
        let mean = |s: &Vortex| -> [f32; 3] {
            let mut m = [0.0f32; 3];
            for n in 0..NODES {
                for (k, c) in m.iter_mut().enumerate() {
                    *c += s.node[n][k] / NODES as f32;
                }
            }
            m
        };
        let m = mean(&sim);
        assert!(
            m[1].abs() > 0.05,
            "the ring did not travel along its axis: centre at {m:?}"
        );
        assert!(
            m[0].abs() < 0.05 && m[2].abs() < 0.05,
            "the ring drifted sideways, which it has nothing to push against for: {m:?}"
        );
        let mut out = Vec::new();
        sim.points(&mut out);
        box_ok(&out);
        assert_eq!(out.len(), POINTS);
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
