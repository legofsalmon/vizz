//! Lock-free parameter store: the control spine of vizz.
//!
//! Every live-controllable value in the app is a [`Param`] registered in a
//! [`ParamRegistry`]. Control threads (OSC, MIDI, UI) write *target* values
//! through atomics; the render thread never takes a lock. Each frame the
//! render thread calls [`ParamSnapshot::advance`], which pulls the targets
//! and applies per-parameter exponential smoothing, so knob jumps become
//! glides and MIDI stair-stepping is filtered out.
//!
//! Topology (the set of parameters) is fixed once [`ParamRegistryBuilder::build`]
//! runs; only values change at runtime. That is what makes the whole thing
//! wait-free on the render path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// Stable index of a parameter within its registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParamId(usize);

impl ParamId {
    /// Position in the registry. Modulation indexes its per-parameter
    /// offset buffer by this.
    pub fn index(self) -> usize {
        self.0
    }
}

/// Static definition of one parameter.
#[derive(Debug, Clone)]
pub struct ParamDef {
    /// OSC-style address, e.g. `/particles/count`. Must be unique.
    pub addr: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Smoothing time constant in seconds. `0.0` means snap instantly.
    /// After `smooth` seconds the value has covered ~63% of the distance
    /// to the target; after `3 * smooth` it is ~95% there.
    pub smooth: f32,
    /// Names for a stepped parameter's positions, indexed by the rounded
    /// value.
    ///
    /// `/shape/mode` reading `5.000` tells you nothing; reading `Lorenz`
    /// tells you what is on screen. Only for parameters whose positions
    /// are genuinely discrete — a swept control has no names to give.
    pub labels: Option<&'static [&'static str]>,
    /// Transport rather than look: fire, blend time, curve, autopilot.
    ///
    /// The single source of truth for that set. Presets exclude it, the
    /// panel's parameter list hides it, and modulation refuses to route to
    /// it — all derived from here rather than from separate lists that
    /// have already drifted once.
    pub transport: bool,
    /// A momentary performance move — a flash, a blackout, a freeze —
    /// rather than part of a look.
    ///
    /// Not transport: transport says *when* things happen and is hidden
    /// from the panel and refused by modulation, while a gesture is a
    /// thing you perform — a strobe under an audio band is a legitimate
    /// patch, and the panel should show it. What a gesture shares with
    /// transport is that no preset may capture it: recalling a look must
    /// never replay somebody's blackout.
    pub gesture: bool,
    /// Whether the parameter list gives this a row of its own.
    ///
    /// True for almost everything. False says the control has a *better*
    /// home elsewhere in the panel — not that it is hidden, which is a
    /// different and much worse thing. `/cloud/a` is the case this
    /// exists for: it is chosen by clicking a cloud's name in the CLOUDS
    /// section, where the names are, rather than by dragging a slider
    /// whose value is a slot index nobody can read.
    ///
    /// Unlike [`ParamDef::transport`] this says nothing about presets: an
    /// unlisted parameter is still part of the look, still captured, and
    /// still restored.
    pub listed: bool,
    /// Moved by the machinery, never offered as a control.
    ///
    /// The cloud morph is the case this exists for. A transition between
    /// two scenes pins `/cloud/a` to the outgoing cloud and `/cloud/b` to
    /// the incoming one and sweeps `/cloud/morph` across — that is the
    /// geometry blend, and it is the only thing that should ever move
    /// those two. A hand on the morph fader mid-transition is fighting
    /// the transition for the same three values.
    ///
    /// So: no row in the parameter list, and refused by modulation, MIDI
    /// and OSC. Still captured by presets, because *which* cloud a scene
    /// shows is part of that scene — and the transition reads it back out
    /// to know what to blend from.
    ///
    /// Implies [`ParamDef::listed`] is false. One flag rather than a
    /// second list, for the reason [`ParamDef::transport`] gives.
    pub driven: bool,
}

impl ParamDef {
    pub fn new(addr: impl Into<String>, min: f32, max: f32, default: f32) -> Self {
        let addr = addr.into();
        assert!(min < max, "param {addr}: min must be < max");
        assert!(
            (min..=max).contains(&default),
            "param {addr}: default out of range"
        );
        Self {
            addr,
            min,
            max,
            default,
            smooth: 0.0,
            labels: None,
            transport: false,
            gesture: false,
            listed: true,
            driven: false,
        }
    }

    /// Name this parameter's discrete positions. See [`ParamDef::labels`].
    pub fn labels(mut self, labels: &'static [&'static str]) -> Self {
        self.labels = Some(labels);
        self
    }

    /// The name for a value, if this parameter has names. Out-of-range
    /// values yield `None` rather than panicking: the value is clamped
    /// elsewhere, and a label is never worth a crash mid-set.
    pub fn label_for(&self, value: f32) -> Option<&'static str> {
        let labels = self.labels?;
        labels.get(value.round().max(0.0) as usize).copied()
    }

    /// Set the smoothing time constant (seconds).
    /// Mark this as transport: it says *when* something happens, not what
    /// anything looks like.
    ///
    /// One flag rather than a hand-maintained list, because there were two
    /// such lists — `preset::EXCLUDED` and the panel's `is_transport` —
    /// and adding the gravity layer updated one and not the other. The
    /// result was that dragging `/gravity/fire` in the parameter list
    /// fired every gravity scene it glided over, which is precisely the
    /// failure the second list existed to prevent.
    pub fn transport(mut self) -> Self {
        self.transport = true;
        self
    }

    /// Keep this out of the parameter list; it is reached somewhere
    /// better. See [`ParamDef::listed`].
    pub fn unlisted(mut self) -> Self {
        self.listed = false;
        self
    }

    /// Mark this as driven by the machinery rather than by hand: no row,
    /// and no route in from modulation, MIDI or OSC. See
    /// [`ParamDef::driven`].
    pub fn driven(mut self) -> Self {
        self.driven = true;
        self.listed = false;
        self
    }

    pub fn smooth(mut self, seconds: f32) -> Self {
        self.smooth = seconds.max(0.0);
        self
    }

    /// Mark this as a gesture. See [`ParamDef::gesture`]. Gestures are
    /// never smoothed — a flash that fades in is not a flash — so this
    /// asserts the smoothing was left at zero rather than quietly
    /// overriding a conflicting call.
    pub fn gesture(mut self) -> Self {
        assert!(
            self.smooth == 0.0,
            "param {}: a gesture cannot be smoothed",
            self.addr
        );
        self.gesture = true;
        self
    }
}

#[derive(Default)]
pub struct ParamRegistryBuilder {
    defs: Vec<ParamDef>,
}

impl ParamRegistryBuilder {
    /// Register a parameter. Panics on duplicate address (programmer error:
    /// the parameter set is app code, not user input).
    pub fn add(&mut self, def: ParamDef) -> ParamId {
        assert!(
            !self.defs.iter().any(|d| d.addr == def.addr),
            "duplicate param address: {}",
            def.addr
        );
        self.defs.push(def);
        ParamId(self.defs.len() - 1)
    }

    pub fn build(self) -> ParamRegistry {
        let by_addr = self
            .defs
            .iter()
            .enumerate()
            .map(|(i, d)| (d.addr.clone(), ParamId(i)))
            .collect();
        let targets = self
            .defs
            .iter()
            .map(|d| AtomicU32::new(d.default.to_bits()))
            .collect();
        ParamRegistry {
            defs: self.defs,
            by_addr,
            targets,
        }
    }
}

/// Shared parameter store. `Sync`: hand it out via `Arc` to every control
/// thread. Writers clamp to the parameter's range.
pub struct ParamRegistry {
    defs: Vec<ParamDef>,
    by_addr: HashMap<String, ParamId>,
    targets: Vec<AtomicU32>,
}

impl ParamRegistry {
    pub fn builder() -> ParamRegistryBuilder {
        ParamRegistryBuilder::default()
    }

    pub fn defs(&self) -> &[ParamDef] {
        &self.defs
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn id(&self, addr: &str) -> Option<ParamId> {
        self.by_addr.get(addr).copied()
    }

    /// Every parameter with its id. A UI can build itself from this, so
    /// registering a parameter is all it takes to get a control for it.
    pub fn iter(&self) -> impl Iterator<Item = (ParamId, &ParamDef)> {
        self.defs.iter().enumerate().map(|(i, d)| (ParamId(i), d))
    }

    /// Set a target value (clamped to range).
    ///
    /// Non-finite values are dropped rather than stored. `f32::clamp`
    /// returns NaN for NaN, and the smoothing in `advance_modulated` is
    /// NaN-absorbing — `base + (target - base) * k` stays NaN forever once
    /// poisoned — so a single NaN from an OSC client with a divide in its
    /// expression would kill that parameter for the life of the process,
    /// recoverable only by a restart. It also serialises as JSON `null`,
    /// producing a preset file that saves cleanly and can never be loaded.
    ///
    /// Every writer — OSC, MIDI, presets, the grid, the panel — funnels
    /// through here, which is why one guard is enough.
    pub fn set(&self, id: ParamId, value: f32) {
        if !value.is_finite() {
            return;
        }
        let def = &self.defs[id.0];
        let v = value.clamp(def.min, def.max);
        self.targets[id.0].store(v.to_bits(), Ordering::Relaxed);
    }

    /// Set from a normalized 0..1 value, mapped onto the parameter's range.
    /// This is what MIDI CCs and unipolar controller faders will feed.
    pub fn set_normalized(&self, id: ParamId, t: f32) {
        let def = &self.defs[id.0];
        let t = t.clamp(0.0, 1.0);
        self.set(id, def.min + t * (def.max - def.min));
    }

    /// Set by address. Returns `false` (and logs at debug) for unknown
    /// addresses — unknown OSC traffic must never disturb a running show.
    pub fn set_by_addr(&self, addr: &str, value: f32) -> bool {
        match self.id(addr) {
            Some(id) => {
                self.set(id, value);
                true
            }
            None => {
                log::debug!("ignoring unknown param address: {addr}");
                false
            }
        }
    }

    /// Current target value (not smoothed).
    pub fn target(&self, id: ParamId) -> f32 {
        f32::from_bits(self.targets[id.0].load(Ordering::Relaxed))
    }
}

/// Render-thread-local view of the parameters, with smoothing applied.
/// Owned by exactly one thread; `advance` is the only coupling with the
/// shared registry and it is wait-free.
pub struct ParamSnapshot {
    /// Smoothed values without modulation — what the user set.
    base: Vec<f32>,
    /// What the renderer uses: base plus modulation, clamped.
    current: Vec<f32>,
}

impl ParamSnapshot {
    /// Starts at the registry defaults.
    pub fn new(reg: &ParamRegistry) -> Self {
        let values: Vec<f32> = reg.defs().iter().map(|d| d.default).collect();
        Self { base: values.clone(), current: values }
    }

    /// Pull targets and advance smoothing by `dt` seconds.
    pub fn advance(&mut self, reg: &ParamRegistry, dt: f32) {
        self.advance_modulated(reg, dt, &[]);
    }

    /// As [`Self::advance`], plus per-parameter modulation.
    ///
    /// `offsets` is indexed by parameter position and expressed in
    /// *normalised* units: 0.25 shifts a parameter by a quarter of its
    /// range. Modulation is added after smoothing and never written back
    /// to the store, so the value a user or controller set is preserved
    /// and reappears the moment modulation stops.
    pub fn advance_modulated(&mut self, reg: &ParamRegistry, dt: f32, offsets: &[f32]) {
        for (i, def) in reg.defs().iter().enumerate() {
            let target = reg.target(ParamId(i));
            let base = if def.smooth <= f32::EPSILON {
                target
            } else {
                // Exponential slew: frame-rate independent for any dt.
                let k = 1.0 - (-dt / def.smooth).exp();
                self.base[i] + (target - self.base[i]) * k
            };
            self.base[i] = base;

            let offset = offsets.get(i).copied().unwrap_or(0.0) * (def.max - def.min);
            self.current[i] = (base + offset).clamp(def.min, def.max);
        }
    }

    /// Land every parameter on its target immediately, skipping the
    /// slew.
    ///
    /// The per-parameter smoothing exists to take the staircase out of a
    /// 7-bit MIDI knob, and it is right to have it on all the time — for
    /// knobs. It is wrong when something has deliberately asked for an
    /// instant change: a scene set to a zero-second blend still crossed
    /// over the parameters' own time constants, so "cut" faded. The
    /// control said one thing and the picture did another, which is
    /// worse than not having the control.
    pub fn snap(&mut self, reg: &ParamRegistry) {
        for (i, def) in reg.defs().iter().enumerate() {
            self.base[i] = reg.target(ParamId(i));
            self.current[i] = self.base[i].clamp(def.min, def.max);
        }
    }

    pub fn get(&self, id: ParamId) -> f32 {
        self.current[id.0]
    }

    /// The un-modulated value, i.e. where the fader actually sits.
    pub fn base(&self, id: ParamId) -> f32 {
        self.base[id.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cut cuts.
    ///
    /// Per-parameter smoothing exists to take the staircase out of a
    /// 7-bit MIDI knob, and it is right to have on all the time — for
    /// knobs. It was also applied to a scene change set to a zero-second
    /// blend, so "cut" faded: the one control whose whole job is to be
    /// instant was the one that could not be.
    #[test]
    fn snapping_lands_a_value_the_slew_would_have_faded() {
        let mut b = ParamRegistry::builder();
        // A generous time constant, so a single advance covers only a
        // little of the distance and the difference is unmistakable.
        let id = b.add(ParamDef::new("/a", 0.0, 1.0, 0.0).smooth(1.0));
        let reg = b.build();
        let mut values = ParamSnapshot::new(&reg);

        // Smoothed: one 16ms frame gets nowhere near the target.
        reg.set(id, 1.0);
        values.advance(&reg, 0.016);
        let eased = values.get(id);
        assert!(
            eased < 0.1,
            "the premise is wrong — smoothing did not hold the value back ({eased})"
        );

        // Snapped: it is simply there.
        values.snap(&reg);
        assert_eq!(values.get(id), 1.0, "a snap did not land the target");

        // And smoothing still works afterwards, rather than being
        // permanently disabled by one cut.
        reg.set(id, 0.0);
        values.advance(&reg, 0.016);
        let after = values.get(id);
        assert!(
            after > 0.9,
            "smoothing stopped working after a snap ({after})"
        );
    }
    use std::sync::Arc;

    fn reg_one(smooth: f32) -> (ParamRegistry, ParamId) {
        let mut b = ParamRegistry::builder();
        let id = b.add(ParamDef::new("/test/x", 0.0, 10.0, 2.0).smooth(smooth));
        (b.build(), id)
    }

    #[test]
    fn set_and_lookup_by_addr() {
        let (reg, id) = reg_one(0.0);
        assert!(reg.set_by_addr("/test/x", 5.0));
        assert_eq!(reg.target(id), 5.0);
        assert!(!reg.set_by_addr("/nope", 1.0));
    }

    #[test]
    fn values_clamp_to_range() {
        let (reg, id) = reg_one(0.0);
        reg.set(id, 99.0);
        assert_eq!(reg.target(id), 10.0);
        reg.set(id, -3.0);
        assert_eq!(reg.target(id), 0.0);
    }

    #[test]
    fn normalized_maps_range() {
        let mut b = ParamRegistry::builder();
        let id = b.add(ParamDef::new("/test/y", -1.0, 1.0, 0.0));
        let reg = b.build();
        reg.set_normalized(id, 0.75);
        assert!((reg.target(id) - 0.5).abs() < 1e-6);
        reg.set_normalized(id, 2.0); // out-of-range input clamps
        assert_eq!(reg.target(id), 1.0);
    }

    #[test]
    fn zero_smooth_snaps() {
        let (reg, id) = reg_one(0.0);
        let mut snap = ParamSnapshot::new(&reg);
        reg.set(id, 7.0);
        snap.advance(&reg, 1.0 / 60.0);
        assert_eq!(snap.get(id), 7.0);
    }

    #[test]
    fn smoothing_converges_monotonically() {
        let (reg, id) = reg_one(0.1);
        let mut snap = ParamSnapshot::new(&reg);
        reg.set(id, 10.0);
        let mut last = snap.get(id);
        for _ in 0..120 {
            snap.advance(&reg, 1.0 / 60.0);
            let v = snap.get(id);
            assert!(v >= last, "smoothing must be monotonic toward target");
            last = v;
        }
        // 2 seconds at tau=0.1 → converged for all practical purposes.
        assert!((last - 10.0).abs() < 1e-3, "got {last}");
    }

    #[test]
    fn smoothing_is_framerate_independent() {
        let (reg_a, id) = reg_one(0.25);
        let (reg_b, _) = reg_one(0.25);
        let mut at_60 = ParamSnapshot::new(&reg_a);
        let mut at_30 = ParamSnapshot::new(&reg_b);
        reg_a.set(id, 10.0);
        reg_b.set(id, 10.0);
        for _ in 0..60 {
            at_60.advance(&reg_a, 1.0 / 60.0);
        }
        for _ in 0..30 {
            at_30.advance(&reg_b, 1.0 / 30.0);
        }
        // Same wall-clock time elapsed → nearly the same value.
        assert!((at_60.get(id) - at_30.get(id)).abs() < 0.05);
    }

    #[test]
    fn concurrent_writers_do_not_corrupt() {
        let (reg, id) = reg_one(0.0);
        let reg = Arc::new(reg);
        let handles: Vec<_> = (0..4)
            .map(|w| {
                let reg = Arc::clone(&reg);
                std::thread::spawn(move || {
                    for i in 0..10_000 {
                        reg.set(id, (i % 11) as f32 * 0.9 + w as f32 * 0.01);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let v = reg.target(id);
        // Whatever write won, the value must be a valid in-range f32.
        assert!((0.0..=10.0).contains(&v), "got {v}");
    }
}
