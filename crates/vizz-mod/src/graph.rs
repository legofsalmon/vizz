//! Modulation as a directed graph.
//!
//! Sources, operators and parameter sinks are all nodes; every node has one
//! output and zero or more inputs. This replaces the flat
//! source-to-parameter routing, which could not express the two things that
//! actually matter in a patch: an operator *between* a source and its
//! target, and a source feeding another source's parameter.
//!
//! Evaluation is a cached topological order, recomputed only when the
//! structure changes. That matters because this runs on the render thread
//! every frame — sorting a graph 60 times a second to produce the same
//! answer would be pure waste.
//!
//! Cycles are the failure mode a patcher has to survive: it is far too easy
//! to wire an output back into its own chain mid-set. Rather than
//! overflowing the stack or silently producing garbage, offending nodes are
//! excluded from the order and evaluate to zero, and `cycle_nodes` reports
//! them so the UI can mark the loop in red.

use serde::{Deserialize, Serialize};
use vizz_params::ParamRegistry;

use crate::{AudioLevels, Lfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub usize);

/// Shaping applied to a 0..1 or -1..1 signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveShape {
    Linear,
    /// Squared: slow start, fast finish. The usual choice for making an
    /// audio band feel punchy rather than mushy.
    Exp2,
    Exp4,
    /// Fast start, slow finish.
    Log,
    /// Ease in and out — the natural choice for LFO-driven movement.
    SCurve,
}

impl CurveShape {
    pub const ALL: [Self; 5] = [Self::Linear, Self::Exp2, Self::Exp4, Self::Log, Self::SCurve];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Exp2 => "exp²",
            Self::Exp4 => "exp⁴",
            Self::Log => "log",
            Self::SCurve => "S-curve",
        }
    }

    /// Applied to magnitude and sign-preserved, so a bipolar LFO keeps its
    /// shape either side of zero instead of being rectified.
    pub fn apply(&self, x: f32) -> f32 {
        let m = x.abs().min(1.0);
        let shaped = match self {
            Self::Linear => m,
            Self::Exp2 => m * m,
            Self::Exp4 => m * m * m * m,
            Self::Log => 1.0 - (1.0 - m) * (1.0 - m),
            Self::SCurve => m * m * (3.0 - 2.0 * m),
        };
        shaped.copysign(x)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MathOp {
    Add,
    Subtract,
    Multiply,
    Min,
    Max,
}

impl MathOp {
    pub const ALL: [Self; 5] = [Self::Add, Self::Subtract, Self::Multiply, Self::Min, Self::Max];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Add => "a + b",
            Self::Subtract => "a - b",
            Self::Multiply => "a × b",
            Self::Min => "min",
            Self::Max => "max",
        }
    }

    pub fn apply(&self, a: f32, b: f32) -> f32 {
        match self {
            Self::Add => a + b,
            Self::Subtract => a - b,
            Self::Multiply => a * b,
            Self::Min => a.min(b),
            Self::Max => a.max(b),
        }
    }
}

/// What a node does. One output each; input count is [`NodeKind::inputs`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    // --- sources: no inputs ---
    Lfo(Lfo),
    /// Audio band envelope, unipolar 0..1.
    Band(usize),
    /// Broadband level, unipolar.
    Level,
    /// Ramp 0..1 over `beats`, locked to the beat clock. A saw the whole
    /// patch can phase off, rather than each LFO keeping its own clock.
    Phasor { beats: f32 },
    Constant(f32),

    // --- operators ---
    Curve { shape: CurveShape, amount: f32 },
    Math { op: MathOp },
    /// Affine: the workhorse for turning a unipolar source bipolar
    /// (mul 2, add -1) or trimming a range.
    Scale { mul: f32, add: f32 },
    /// Asymmetric one-pole, seconds. Slew-limits anything twitchy.
    Smooth { attack: f32, release: f32 },
    /// Snap to N steps — the thing that turns a smooth sweep into
    /// something that lands on the beat.
    Quantise { steps: f32 },
    /// Latch input when the trigger input crosses 0.5 rising.
    SampleHold,

    // --- sink ---
    /// Adds `depth × input` to a parameter's normalised offset.
    Param { addr: String, depth: f32 },
}

impl NodeKind {
    pub fn inputs(&self) -> usize {
        match self {
            Self::Lfo(_)
            | Self::Band(_)
            | Self::Level
            | Self::Phasor { .. }
            | Self::Constant(_) => 0,
            Self::Math { .. } | Self::SampleHold => 2,
            _ => 1,
        }
    }

    pub fn input_label(&self, port: usize) -> &'static str {
        match (self, port) {
            (Self::Math { .. }, 0) => "a",
            (Self::Math { .. }, 1) => "b",
            (Self::SampleHold, 0) => "in",
            (Self::SampleHold, 1) => "trig",
            _ => "in",
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Lfo(l) => format!("LFO · {}", l.shape.label()),
            Self::Band(i) => format!("Band {}", i + 1),
            Self::Level => "Level".into(),
            Self::Phasor { .. } => "Phasor".into(),
            Self::Constant(_) => "Constant".into(),
            Self::Curve { .. } => "Curve".into(),
            Self::Math { op } => format!("Math · {}", op.label()),
            Self::Scale { .. } => "Scale".into(),
            Self::Smooth { .. } => "Smooth".into(),
            Self::Quantise { .. } => "Quantise".into(),
            Self::SampleHold => "S&H".into(),
            Self::Param { addr, .. } => addr.clone(),
        }
    }

    /// Sources, operators and sinks are coloured differently on the canvas;
    /// this is the classification behind that.
    pub fn category(&self) -> Category {
        match self {
            Self::Lfo(_)
            | Self::Band(_)
            | Self::Level
            | Self::Phasor { .. }
            | Self::Constant(_) => Category::Source,
            Self::Param { .. } => Category::Sink,
            _ => Category::Operator,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Source,
    Operator,
    Sink,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub kind: NodeKind,
    /// Canvas position. Stored with the patch so a layout survives a
    /// reload — a rearranged patch is a patch you have to re-read.
    pub pos: [f32; 2],
    #[serde(default)]
    pub bypass: bool,
    /// Per-node scratch: S&H latch, smoother state, phasor phase.
    #[serde(skip)]
    state: f32,
    #[serde(skip)]
    prev_trigger: f32,
}

impl Node {
    pub fn new(kind: NodeKind, pos: [f32; 2]) -> Self {
        Self { kind, pos, bypass: false, state: 0.0, prev_trigger: 0.0 }
    }
}

/// A connection from one node's output to another node's numbered input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub port: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Cached topological order; rebuilt only on structural change.
    #[serde(skip)]
    order: Vec<usize>,
    #[serde(skip)]
    values: Vec<f32>,
    #[serde(skip)]
    cycle: Vec<usize>,
    #[serde(skip)]
    dirty: bool,
}

impl NodeGraph {
    pub fn add(&mut self, kind: NodeKind, pos: [f32; 2]) -> NodeId {
        self.nodes.push(Node::new(kind, pos));
        self.dirty = true;
        NodeId(self.nodes.len() - 1)
    }

    /// Connect, replacing whatever fed that input. Inputs take one edge —
    /// summing several into a port silently is how patches become
    /// unreadable; use an explicit Math node.
    pub fn connect(&mut self, from: NodeId, to: NodeId, port: usize) {
        if from.0 >= self.nodes.len() || to.0 >= self.nodes.len() {
            return;
        }
        if port >= self.nodes[to.0].kind.inputs() {
            return;
        }
        self.edges.retain(|e| !(e.to == to && e.port == port));
        self.edges.push(Edge { from, to, port });
        self.dirty = true;
    }

    pub fn disconnect(&mut self, to: NodeId, port: usize) {
        self.edges.retain(|e| !(e.to == to && e.port == port));
        self.dirty = true;
    }

    /// Remove a node and everything wired to it. Indices shift, so edges
    /// are renumbered rather than left dangling.
    pub fn remove(&mut self, id: NodeId) {
        if id.0 >= self.nodes.len() {
            return;
        }
        self.nodes.remove(id.0);
        self.edges.retain(|e| e.from != id && e.to != id);
        for e in &mut self.edges {
            if e.from.0 > id.0 {
                e.from.0 -= 1;
            }
            if e.to.0 > id.0 {
                e.to.0 -= 1;
            }
        }
        self.dirty = true;
    }

    pub fn value(&self, id: NodeId) -> f32 {
        self.values.get(id.0).copied().unwrap_or(0.0)
    }

    /// Nodes excluded from evaluation because they sit in a cycle.
    pub fn cycle_nodes(&self) -> &[usize] {
        &self.cycle
    }

    /// Would connecting these create a cycle? Checked before wiring so the
    /// UI can refuse the drop rather than accept it and then disable it.
    pub fn would_cycle(&self, from: NodeId, to: NodeId) -> bool {
        if from == to {
            return true;
        }
        // Walk forward from `to`: if we can reach `from`, the new edge
        // would close a loop.
        let mut stack = vec![to.0];
        let mut seen = vec![false; self.nodes.len()];
        while let Some(n) = stack.pop() {
            if n == from.0 {
                return true;
            }
            if std::mem::replace(&mut seen[n], true) {
                continue;
            }
            for e in self.edges.iter().filter(|e| e.from.0 == n) {
                stack.push(e.to.0);
            }
        }
        false
    }

    /// Kahn's algorithm. Anything left with a non-zero in-degree is in (or
    /// downstream of) a cycle and is recorded rather than evaluated.
    fn rebuild_order(&mut self) {
        let n = self.nodes.len();
        let mut indegree = vec![0usize; n];
        for e in &self.edges {
            if e.to.0 < n && e.from.0 < n {
                indegree[e.to.0] += 1;
            }
        }
        let mut queue: Vec<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
        // Stable order: a patch must evaluate identically across runs, and
        // pop() from a filtered range is already deterministic, but sorting
        // keeps it independent of how the queue is drained.
        queue.sort_unstable();

        self.order.clear();
        let mut head = 0;
        while head < queue.len() {
            let node = queue[head];
            head += 1;
            self.order.push(node);
            for e in self.edges.iter().filter(|e| e.from.0 == node) {
                if e.to.0 >= n {
                    continue;
                }
                indegree[e.to.0] -= 1;
                if indegree[e.to.0] == 0 {
                    queue.push(e.to.0);
                }
            }
        }
        self.cycle = (0..n).filter(|i| !self.order.contains(i)).collect();
        self.dirty = false;
    }

    /// Evaluate every node and accumulate parameter offsets.
    ///
    /// Returns offsets indexed by parameter position, in normalised units,
    /// ready for `ParamSnapshot::advance_modulated`.
    pub fn tick(
        &mut self,
        dt: f32,
        beat_delta: f64,
        beats: f64,
        audio: AudioLevels<'_>,
        registry: &ParamRegistry,
        offsets: &mut Vec<f32>,
    ) {
        offsets.clear();
        offsets.resize(registry.len(), 0.0);

        if self.dirty || self.order.len() + self.cycle.len() != self.nodes.len() {
            self.rebuild_order();
        }
        self.values.clear();
        self.values.resize(self.nodes.len(), 0.0);

        for idx in 0..self.order.len() {
            let i = self.order[idx];
            // Gather inputs before touching the node, so the borrow of the
            // edge list does not overlap the mutable node borrow.
            let mut input = [0.0f32; 2];
            for (port, slot) in input.iter_mut().enumerate() {
                if let Some(e) = self.edges.iter().find(|e| e.to.0 == i && e.port == port) {
                    *slot = self.values.get(e.from.0).copied().unwrap_or(0.0);
                }
            }

            let node = &mut self.nodes[i];
            if node.bypass {
                // Pass-through rather than zero: bypassing an operator
                // should audition the chain without it, not mute the chain.
                self.values[i] = input[0];
                continue;
            }

            let v = match &mut node.kind {
                NodeKind::Lfo(lfo) => lfo.tick(dt, beat_delta),
                NodeKind::Band(b) => audio.bands.get(*b).copied().unwrap_or(0.0),
                NodeKind::Level => audio.level,
                NodeKind::Phasor { beats: per } => {
                    if *per > 0.0 {
                        (beats / *per as f64).rem_euclid(1.0) as f32
                    } else {
                        0.0
                    }
                }
                NodeKind::Constant(c) => *c,
                NodeKind::Curve { shape, amount } => {
                    // `amount` blends between untouched and fully shaped, so
                    // the control is continuous rather than a switch.
                    let shaped = shape.apply(input[0]);
                    input[0] + (shaped - input[0]) * amount.clamp(0.0, 1.0)
                }
                NodeKind::Math { op } => op.apply(input[0], input[1]),
                NodeKind::Scale { mul, add } => input[0] * *mul + *add,
                NodeKind::Smooth { attack, release } => {
                    let tau = if input[0] > node.state { *attack } else { *release };
                    let k = if tau <= f32::EPSILON {
                        1.0
                    } else {
                        1.0 - (-dt / tau).exp()
                    };
                    node.state += (input[0] - node.state) * k;
                    node.state
                }
                NodeKind::Quantise { steps } => {
                    let s = steps.max(1.0);
                    (input[0] * s).round() / s
                }
                NodeKind::SampleHold => {
                    // Rising edge through the midpoint, so a square LFO or a
                    // band envelope both work as triggers.
                    if node.prev_trigger < 0.5 && input[1] >= 0.5 {
                        node.state = input[0];
                    }
                    node.prev_trigger = input[1];
                    node.state
                }
                NodeKind::Param { addr, depth } => {
                    if let Some(id) = registry.id(addr) {
                        // Several Param nodes may target one parameter; they
                        // sum, and the snapshot clamps the total.
                        offsets[id.index()] += input[0] * *depth;
                    }
                    input[0]
                }
            };
            self.values[i] = v;
        }
    }
}

/// The palette: every node type that can be added, with a sensible starting
/// configuration. Single source of truth for the canvas's add menu and for
/// the library view, so a new node kind appears in both without being
/// registered twice.
pub fn catalog() -> Vec<(Category, &'static str, NodeKind)> {
    use NodeKind as K;
    vec![
        (Category::Source, "LFO", K::Lfo(Lfo::default())),
        (Category::Source, "Audio band", K::Band(0)),
        (Category::Source, "Audio level", K::Level),
        (Category::Source, "Phasor", K::Phasor { beats: 4.0 }),
        (Category::Source, "Constant", K::Constant(1.0)),
        (Category::Operator, "Curve", K::Curve { shape: CurveShape::Exp2, amount: 1.0 }),
        (Category::Operator, "Math", K::Math { op: MathOp::Add }),
        (Category::Operator, "Scale", K::Scale { mul: 1.0, add: 0.0 }),
        (Category::Operator, "Smooth", K::Smooth { attack: 0.02, release: 0.2 }),
        (Category::Operator, "Quantise", K::Quantise { steps: 4.0 }),
        (Category::Operator, "Sample & hold", K::SampleHold),
        (Category::Sink, "Parameter", K::Param { addr: String::new(), depth: 0.5 }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizz_params::ParamDef;

    fn registry() -> ParamRegistry {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/a", 0.0, 1.0, 0.0));
        b.add(ParamDef::new("/b", 0.0, 1.0, 0.0));
        b.build()
    }

    fn run(g: &mut NodeGraph, reg: &ParamRegistry) -> Vec<f32> {
        let mut out = Vec::new();
        g.tick(1.0 / 60.0, 0.0, 0.0, AudioLevels::default(), reg, &mut out);
        out
    }

    fn run_audio(g: &mut NodeGraph, reg: &ParamRegistry, bands: &[f32]) -> Vec<f32> {
        let mut out = Vec::new();
        g.tick(1.0 / 60.0, 0.0, 0.0, AudioLevels { bands, level: 0.0 }, reg, &mut out);
        out
    }

    /// The capability the flat routing could not express: an operator
    /// sitting between a source and its target.
    #[test]
    fn operator_shapes_a_source_before_it_reaches_a_parameter() {
        let reg = registry();
        let mut g = NodeGraph::default();
        let band = g.add(NodeKind::Band(0), [0.0, 0.0]);
        let curve = g.add(NodeKind::Curve { shape: CurveShape::Exp2, amount: 1.0 }, [1.0, 0.0]);
        let param = g.add(NodeKind::Param { addr: "/a".into(), depth: 1.0 }, [2.0, 0.0]);
        g.connect(band, curve, 0);
        g.connect(curve, param, 0);

        let out = run_audio(&mut g, &reg, &[0.5]);
        // 0.5 squared, at full depth.
        assert!((out[0] - 0.25).abs() < 1e-5, "got {}", out[0]);
    }

    /// The other capability: a source driving another source's parameter.
    /// Here an LFO is summed into a band before it reaches the target,
    /// which the old `source -> param` model had no way to say.
    #[test]
    fn sources_can_be_combined_before_reaching_a_target() {
        let reg = registry();
        let mut g = NodeGraph::default();
        let a = g.add(NodeKind::Constant(0.3), [0.0, 0.0]);
        let b = g.add(NodeKind::Band(0), [0.0, 1.0]);
        let sum = g.add(NodeKind::Math { op: MathOp::Add }, [1.0, 0.0]);
        let param = g.add(NodeKind::Param { addr: "/a".into(), depth: 1.0 }, [2.0, 0.0]);
        g.connect(a, sum, 0);
        g.connect(b, sum, 1);
        g.connect(sum, param, 0);

        let out = run_audio(&mut g, &reg, &[0.4]);
        assert!((out[0] - 0.7).abs() < 1e-5, "got {}", out[0]);
    }

    /// Evaluation must follow the wiring, not the insertion order. Building
    /// the chain backwards is the case that catches an unsorted evaluator:
    /// it would read a stale zero and the patch would lag a frame per hop.
    #[test]
    fn evaluation_follows_wiring_not_insertion_order() {
        let reg = registry();
        let mut g = NodeGraph::default();
        // Added sink first, source last.
        let param = g.add(NodeKind::Param { addr: "/a".into(), depth: 1.0 }, [3.0, 0.0]);
        let scale = g.add(NodeKind::Scale { mul: 2.0, add: 0.0 }, [2.0, 0.0]);
        let src = g.add(NodeKind::Constant(0.25), [1.0, 0.0]);
        g.connect(src, scale, 0);
        g.connect(scale, param, 0);

        let out = run(&mut g, &reg);
        assert!((out[0] - 0.5).abs() < 1e-5, "single pass should resolve: {}", out[0]);
    }

    /// Wiring a loop must not hang, overflow or produce nonsense — it is
    /// an easy mistake to make live, so it has to degrade quietly.
    #[test]
    fn cycles_are_excluded_and_reported() {
        let reg = registry();
        let mut g = NodeGraph::default();
        let a = g.add(NodeKind::Scale { mul: 1.0, add: 0.1 }, [0.0, 0.0]);
        let b = g.add(NodeKind::Scale { mul: 1.0, add: 0.1 }, [1.0, 0.0]);
        let good = g.add(NodeKind::Constant(0.4), [0.0, 2.0]);
        let param = g.add(NodeKind::Param { addr: "/a".into(), depth: 1.0 }, [2.0, 2.0]);
        g.connect(a, b, 0);
        g.connect(b, a, 0);
        g.connect(good, param, 0);

        let out = run(&mut g, &reg);
        // The healthy chain still works.
        assert!((out[0] - 0.4).abs() < 1e-5, "unrelated chain broke: {}", out[0]);
        let cycle = g.cycle_nodes();
        assert!(cycle.contains(&a.0) && cycle.contains(&b.0), "cycle not reported: {cycle:?}");
    }

    /// The UI refuses a cycle-forming drop rather than accepting it and
    /// then disabling it, so the check has to be available before wiring.
    #[test]
    fn would_cycle_predicts_the_loop() {
        let mut g = NodeGraph::default();
        let a = g.add(NodeKind::Constant(1.0), [0.0, 0.0]);
        let b = g.add(NodeKind::Scale { mul: 1.0, add: 0.0 }, [1.0, 0.0]);
        let c = g.add(NodeKind::Scale { mul: 1.0, add: 0.0 }, [2.0, 0.0]);
        g.connect(a, b, 0);
        g.connect(b, c, 0);

        assert!(g.would_cycle(c, b), "c -> b closes a loop");
        assert!(g.would_cycle(b, b), "self-connection is a loop");
        assert!(!g.would_cycle(a, c), "a -> c is a legal shortcut");
    }

    /// Deleting a node shifts every later index; edges must follow or the
    /// patch silently rewires itself to the wrong nodes.
    #[test]
    fn removing_a_node_renumbers_edges() {
        let reg = registry();
        let mut g = NodeGraph::default();
        let doomed = g.add(NodeKind::Constant(9.0), [0.0, 0.0]);
        let src = g.add(NodeKind::Constant(0.5), [1.0, 0.0]);
        let param = g.add(NodeKind::Param { addr: "/a".into(), depth: 1.0 }, [2.0, 0.0]);
        g.connect(src, param, 0);

        g.remove(doomed);
        assert_eq!(g.nodes.len(), 2);
        let out = run(&mut g, &reg);
        assert!((out[0] - 0.5).abs() < 1e-5, "edge did not survive renumbering: {}", out[0]);
    }

    /// One input takes one edge. Re-wiring should replace, not accumulate,
    /// or a port ends up summing invisibly.
    #[test]
    fn reconnecting_an_input_replaces_it() {
        let reg = registry();
        let mut g = NodeGraph::default();
        let a = g.add(NodeKind::Constant(0.2), [0.0, 0.0]);
        let b = g.add(NodeKind::Constant(0.7), [0.0, 1.0]);
        let param = g.add(NodeKind::Param { addr: "/a".into(), depth: 1.0 }, [1.0, 0.0]);
        g.connect(a, param, 0);
        g.connect(b, param, 0);

        assert_eq!(g.edges.len(), 1);
        let out = run(&mut g, &reg);
        assert!((out[0] - 0.7).abs() < 1e-5, "got {}", out[0]);
    }

    /// Bypass auditions a chain without an operator, so it must pass its
    /// input through rather than mute.
    #[test]
    fn bypass_passes_through() {
        let reg = registry();
        let mut g = NodeGraph::default();
        let src = g.add(NodeKind::Constant(0.6), [0.0, 0.0]);
        let curve = g.add(NodeKind::Curve { shape: CurveShape::Exp4, amount: 1.0 }, [1.0, 0.0]);
        let param = g.add(NodeKind::Param { addr: "/a".into(), depth: 1.0 }, [2.0, 0.0]);
        g.connect(src, curve, 0);
        g.connect(curve, param, 0);
        g.nodes[curve.0].bypass = true;

        let out = run(&mut g, &reg);
        assert!((out[0] - 0.6).abs() < 1e-5, "bypass should pass through: {}", out[0]);
    }

    #[test]
    fn sample_hold_latches_on_a_rising_trigger() {
        let reg = registry();
        let mut g = NodeGraph::default();
        let sig = g.add(NodeKind::Constant(0.3), [0.0, 0.0]);
        let trig = g.add(NodeKind::Band(0), [0.0, 1.0]);
        let sh = g.add(NodeKind::SampleHold, [1.0, 0.0]);
        let param = g.add(NodeKind::Param { addr: "/a".into(), depth: 1.0 }, [2.0, 0.0]);
        g.connect(sig, sh, 0);
        g.connect(trig, sh, 1);
        g.connect(sh, param, 0);

        // Trigger low: nothing latched yet.
        assert!(run_audio(&mut g, &reg, &[0.0])[0].abs() < 1e-6);
        // Rising edge latches the current signal.
        assert!((run_audio(&mut g, &reg, &[1.0])[0] - 0.3).abs() < 1e-5);
        // Held while the trigger stays high, and after it falls.
        g.nodes[sig.0].kind = NodeKind::Constant(0.9);
        assert!((run_audio(&mut g, &reg, &[1.0])[0] - 0.3).abs() < 1e-5, "re-latched without an edge");
        assert!((run_audio(&mut g, &reg, &[0.0])[0] - 0.3).abs() < 1e-5, "lost the held value");
        // Next rising edge takes the new value.
        assert!((run_audio(&mut g, &reg, &[1.0])[0] - 0.9).abs() < 1e-5);
    }

    /// Curves must not rectify: a bipolar LFO shaped by a curve has to keep
    /// its negative half, or every LFO turns into a one-sided pulse.
    #[test]
    fn curves_preserve_sign() {
        for shape in CurveShape::ALL {
            assert!(shape.apply(-0.5) < 0.0, "{:?} rectified a negative input", shape);
            assert!((shape.apply(0.0)).abs() < 1e-6, "{:?} moved zero", shape);
            assert!((shape.apply(1.0) - 1.0).abs() < 1e-5, "{:?} did not reach unity", shape);
        }
    }

    /// A patch is worth nothing if it cannot be reloaded, and node
    /// positions are part of the patch — a graph that reloads with its
    /// layout scrambled has to be re-read from scratch.
    #[test]
    fn round_trips_through_json_with_layout() {
        let reg = registry();
        let mut g = NodeGraph::default();
        let src = g.add(NodeKind::Lfo(Lfo::default()), [12.0, 34.0]);
        let curve = g.add(NodeKind::Curve { shape: CurveShape::SCurve, amount: 0.5 }, [56.0, 78.0]);
        let param = g.add(NodeKind::Param { addr: "/b".into(), depth: 0.4 }, [90.0, 12.0]);
        g.connect(src, curve, 0);
        g.connect(curve, param, 0);

        let json = serde_json::to_string(&g).unwrap();
        let mut back: NodeGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(back.nodes, g.nodes);
        assert_eq!(back.edges, g.edges);
        assert_eq!(back.nodes[curve.0].pos, [56.0, 78.0]);

        // And it must evaluate without needing an explicit rebuild: the
        // cached order is skipped by serde, so a fresh graph starts empty.
        let out = run(&mut back, &reg);
        assert_eq!(out.len(), reg.len());
    }

    /// The palette drives both the canvas menu and the library view, so
    /// every kind it offers must actually be constructible and evaluate.
    #[test]
    fn every_catalog_entry_evaluates() {
        let reg = registry();
        for (_, name, kind) in catalog() {
            let mut g = NodeGraph::default();
            let inputs = kind.inputs();
            let node = g.add(kind, [0.0, 0.0]);
            for port in 0..inputs {
                let c = g.add(NodeKind::Constant(0.5), [0.0, port as f32]);
                g.connect(c, node, port);
            }
            let out = run(&mut g, &reg);
            assert_eq!(out.len(), reg.len(), "{name} produced the wrong offset length");
            assert!(g.value(node).is_finite(), "{name} produced a non-finite value");
        }
    }
}
