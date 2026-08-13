//! Ready-made modulators, attachable to one parameter in one gesture.
//!
//! The node canvas can build anything, which is exactly its problem
//! during a set: "make this breathe over eight beats" is four nodes and
//! three wires, and nobody is doing that between tracks. A shape is that
//! patch, pre-wired, named for what it *sounds like* rather than for the
//! nodes it contains.
//!
//! Built into the graph rather than onto the flat [`crate::Route`] list
//! on purpose. A route's source is a shared LFO from a fixed bank, so
//! "slow sweep" on one fader and "fast wobble" on another would be the
//! same oscillator set two ways — the second choice silently changing
//! the first. Every shape here brings its own source, so faders do not
//! fight, and what it built stays visible and editable on the canvas
//! afterwards instead of being a hidden special case.

use crate::graph::{CurveShape, NodeGraph, NodeId, NodeKind};
use crate::{Lfo, Rate, Shape};

/// One ready-made modulator.
pub struct ModShape {
    /// What it is called in the menu. Short: this is read in a dark room.
    pub name: &'static str,
    /// One line of what it does, for the hover.
    pub about: &'static str,
    /// How far it moves the parameter, as a fraction of its range.
    ///
    /// Part of the shape rather than a separate control, because the
    /// depth that makes a shape feel right is a property of the shape:
    /// a kick envelope wants most of the range, a slow sweep wants a
    /// third of it. Adjustable afterwards on the sink node like any
    /// other patch.
    pub depth: f32,
    /// Whether it swings either side of the set value or only pushes up
    /// from it. Shown in the menu, because it is the difference between
    /// a fader you can still park at the top and one you cannot.
    pub bipolar: bool,
    /// Builds the source chain and returns the node to feed the sink.
    build: fn(&mut NodeGraph, [f32; 2]) -> NodeId,
}

/// Horizontal spacing between a shape's nodes on the canvas, matching
/// the shipped patches — a shape you attached from a fader is a patch
/// you can go and read.
const STEP: f32 = 180.0;

/// The shipped set.
///
/// Ordered by what they are *for*, not by how they are built: the
/// beat-locked things first, then the ones that follow audio, then the
/// free-running ones. Picking from a menu in a dark room is a scan, and
/// a scan wants like next to like.
pub const SHAPES: &[ModShape] = &[
    ModShape {
        name: "Slow sweep",
        about: "A sine over eight beats. The one that makes a still \
                picture stop being still.",
        depth: 0.35,
        bipolar: true,
        build: |g, at| g.add(lfo(Shape::Sine, Rate::Beats(8.0)), at),
    },
    ModShape {
        name: "Bar sweep",
        about: "A sine over four beats — one bar in four-four.",
        depth: 0.35,
        bipolar: true,
        build: |g, at| g.add(lfo(Shape::Sine, Rate::Beats(4.0)), at),
    },
    ModShape {
        name: "Beat pulse",
        about: "A snap on every beat: hits hard, falls away before the \
                next one.",
        depth: 0.6,
        bipolar: false,
        build: |g, at| {
            let trig = g.add(NodeKind::BeatTrig { beats: 1.0 }, at);
            let env = g.add(
                NodeKind::Envelope { attack: 0.005, decay: 0.16 },
                step(at, 1),
            );
            g.connect(trig, env, 0);
            env
        },
    },
    ModShape {
        name: "Bar pulse",
        about: "The same snap, once a bar. For the thing that should \
                land, not chatter.",
        depth: 0.7,
        bipolar: false,
        build: |g, at| {
            let trig = g.add(NodeKind::BeatTrig { beats: 4.0 }, at);
            let env = g.add(
                NodeKind::Envelope { attack: 0.01, decay: 0.45 },
                step(at, 1),
            );
            g.connect(trig, env, 0);
            env
        },
    },
    ModShape {
        name: "Rise",
        about: "Ramps up over four beats, then drops. A build you do not \
                have to ride.",
        depth: 0.5,
        bipolar: false,
        build: |g, at| g.add(NodeKind::Phasor { beats: 4.0 }, at),
    },
    ModShape {
        name: "Fall",
        about: "The same ramp inverted: full at the downbeat, empty by \
                the next.",
        depth: 0.5,
        bipolar: false,
        build: |g, at| {
            let phasor = g.add(NodeKind::Phasor { beats: 4.0 }, at);
            let flip = g.add(NodeKind::Scale { mul: -1.0, add: 1.0 }, step(at, 1));
            g.connect(phasor, flip, 0);
            flip
        },
    },
    ModShape {
        name: "Steps",
        about: "A four-beat ramp snapped to four steps — movement that \
                lands on the beat instead of sliding past it.",
        depth: 0.5,
        bipolar: false,
        build: |g, at| {
            let phasor = g.add(NodeKind::Phasor { beats: 4.0 }, at);
            let steps = g.add(NodeKind::Quantise { steps: 4.0 }, step(at, 1));
            g.connect(phasor, steps, 0);
            steps
        },
    },
    ModShape {
        name: "Kick",
        about: "The low band, gated into a snap envelope. Follows the \
                drum rather than the clock.",
        depth: 0.6,
        bipolar: false,
        build: |g, at| band_env(g, at, 0, 0.005, 0.18),
    },
    ModShape {
        name: "Snare",
        about: "The same, on the mid band.",
        depth: 0.5,
        bipolar: false,
        build: |g, at| band_env(g, at, 2, 0.005, 0.14),
    },
    ModShape {
        name: "Highs",
        about: "The top band, smoothed. Shimmer rather than hits.",
        depth: 0.4,
        bipolar: false,
        build: |g, at| {
            let band = g.add(NodeKind::Band(3), at);
            let smooth = g.add(
                NodeKind::Smooth { attack: 0.02, release: 0.12 },
                step(at, 1),
            );
            g.connect(band, smooth, 0);
            smooth
        },
    },
    ModShape {
        name: "Loudness",
        about: "Broadband level with the corners taken off — the whole \
                track pushing, not one drum.",
        depth: 0.45,
        bipolar: false,
        build: |g, at| {
            let level = g.add(NodeKind::Level, at);
            let curve = g.add(
                NodeKind::Curve { shape: CurveShape::Exp2, amount: 1.0 },
                step(at, 1),
            );
            let smooth = g.add(
                NodeKind::Smooth { attack: 0.05, release: 0.25 },
                step(at, 2),
            );
            g.connect(level, curve, 0);
            g.connect(curve, smooth, 0);
            smooth
        },
    },
    ModShape {
        name: "Wobble",
        about: "Four cycles a second, free-running. Deliberately not \
                beat-locked — this is texture, not rhythm.",
        depth: 0.25,
        bipolar: true,
        build: |g, at| g.add(lfo(Shape::Sine, Rate::Hz(4.0)), at),
    },
    ModShape {
        name: "Random step",
        about: "A new value every beat, held until the next one.",
        depth: 0.5,
        bipolar: true,
        build: |g, at| g.add(lfo(Shape::SampleHold, Rate::Beats(1.0)), at),
    },
    ModShape {
        name: "Flicker",
        about: "Random, twelve times a second. Broken neon.",
        depth: 0.3,
        bipolar: true,
        build: |g, at| g.add(lfo(Shape::SampleHold, Rate::Hz(12.0)), at),
    },
];

fn lfo(shape: Shape, rate: Rate) -> NodeKind {
    NodeKind::Lfo(Lfo { shape, rate, ..Default::default() })
}

fn step(at: [f32; 2], n: usize) -> [f32; 2] {
    [at[0] + STEP * n as f32, at[1]]
}

/// Vertical spacing between shapes on the canvas.
const ROW: f32 = 120.0;

/// Somewhere clear to build, below whatever is already on the canvas.
///
/// Shapes go under the existing patch rather than searching for gaps in
/// it: a shape is a self-contained chain, and a column of them reads as
/// "the things attached from faders" — which is what they are.
fn free_row(g: &NodeGraph) -> [f32; 2] {
    let bottom = g
        .nodes
        .iter()
        .map(|n| n.pos[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let y = if bottom.is_finite() { bottom + ROW } else { 60.0 };
    [40.0, y]
}

/// Band -> gate -> envelope: the idiom that turns a level into a hit.
fn band_env(g: &mut NodeGraph, at: [f32; 2], band: usize, attack: f32, decay: f32) -> NodeId {
    let src = g.add(NodeKind::Band(band), at);
    let gate = g.add(NodeKind::Gate { threshold: 0.5 }, step(at, 1));
    let env = g.add(NodeKind::Envelope { attack, decay }, step(at, 2));
    g.connect(src, gate, 0);
    g.connect(gate, env, 0);
    env
}

/// Attach a shape to a parameter, replacing whatever shape was there.
///
/// Replacing rather than stacking: from a fader this is one control with
/// one current setting, and picking a second shape means "no, that one".
/// Stacking is still available — it is what the canvas is for.
///
/// Returns false for an out-of-range index, which is the only way this
/// can fail.
pub fn attach(g: &mut NodeGraph, shape: usize, addr: &str) -> bool {
    let Some(s) = SHAPES.get(shape) else { return false };
    detach(g, addr);
    let at = free_row(g);
    let source = (s.build)(g, at);
    // Where the built chain ended, so the sink lands to its right rather
    // than on top of it. A shape is three nodes at most, so this is a
    // short walk.
    let end = g.nodes[source.0].pos;
    let sink = g.add(
        NodeKind::Param { addr: addr.to_string(), depth: s.depth },
        step(end, 1),
    );
    g.connect(source, sink, 0);
    true
}

/// Which shape is attached to a parameter, if the graph still looks like
/// one this module built.
///
/// Matched by structure rather than by a stored name: the canvas can
/// edit anything this creates, and a label claiming "Slow sweep" on a
/// chain somebody rewired into a kick envelope would be a lie the UI
/// tells confidently. Once it stops matching, it reports `None` and the
/// fader says "custom" — which is true.
pub fn attached(g: &NodeGraph, addr: &str) -> Option<usize> {
    let sink = sink_for(g, addr)?;
    // Rebuild each shape into a scratch graph and compare the chain that
    // feeds the sink. Fourteen tiny graphs, only while a menu is open.
    let live = chain(g, sink);
    (0..SHAPES.len()).find(|&i| {
        let mut probe = NodeGraph::default();
        if !attach(&mut probe, i, addr) {
            return false;
        }
        sink_for(&probe, addr)
            .map(|s| chain(&probe, s) == live)
            .unwrap_or(false)
    })
}

/// Whether the graph drives this parameter at all, shape or not.
pub fn driven(g: &NodeGraph, addr: &str) -> bool {
    sink_for(g, addr).is_some()
}

/// Remove the parameter's sink and anything upstream that only fed it.
///
/// The "only fed it" test is what keeps this safe to run on a patch
/// somebody built by hand: a source shared with another chain has an
/// edge going somewhere that survives, so it survives too. Detaching a
/// shape must never quietly take a node the rest of the patch was using.
///
/// Returns whether anything was removed.
pub fn detach(g: &mut NodeGraph, addr: &str) -> bool {
    let mut doomed: Vec<usize> = g
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| matches!(&n.kind, NodeKind::Param { addr: a, .. } if a == addr))
        .map(|(i, _)| i)
        .collect();
    if doomed.is_empty() {
        return false;
    }
    // Walk upstream to a fixpoint. A node joins the set only when every
    // edge leaving it lands inside the set — and only when it has one,
    // so a node the user parked unconnected is never swept up.
    loop {
        let mut grew = false;
        for i in 0..g.nodes.len() {
            if doomed.contains(&i) {
                continue;
            }
            let mut out = g.edges.iter().filter(|e| e.from.0 == i).peekable();
            if out.peek().is_none() {
                continue;
            }
            if out.all(|e| doomed.contains(&e.to.0)) {
                doomed.push(i);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    // Highest first: `remove` renumbers everything above the index it
    // takes, so descending order leaves the pending indices valid.
    doomed.sort_unstable();
    for i in doomed.into_iter().rev() {
        g.remove(NodeId(i));
    }
    true
}

fn sink_for(g: &NodeGraph, addr: &str) -> Option<NodeId> {
    g.nodes
        .iter()
        .position(|n| matches!(&n.kind, NodeKind::Param { addr: a, .. } if a == addr))
        .map(NodeId)
}

/// The kinds feeding a sink, deepest first, with the sink's own depth —
/// enough to recognise a shape without caring where it sits on canvas.
fn chain(g: &NodeGraph, sink: NodeId) -> Vec<NodeKind> {
    let mut out = Vec::new();
    let mut at = sink;
    // Bounded by the node count: a cycle cannot make this spin.
    for _ in 0..=g.nodes.len() {
        out.push(settings_only(&g.nodes[at.0].kind));
        match g.edges.iter().find(|e| e.to == at && e.port == 0) {
            Some(e) => at = e.from,
            None => break,
        }
    }
    out.reverse();
    out
}

/// A node kind with its *settings* and none of its running state.
///
/// [`crate::Lfo`] keeps phase, its sample-and-hold value and its random
/// seed inside the node, and all three move every frame. Comparing the
/// kind as it stands would make a shape recognisable only while the
/// patch was stopped — true in a test that never ticks, false in every
/// real second of use.
///
/// The other nodes hold their scratch on [`crate::graph::Node`] rather
/// than in the kind, so they need nothing here.
fn settings_only(kind: &NodeKind) -> NodeKind {
    match kind {
        NodeKind::Lfo(l) => NodeKind::Lfo(Lfo {
            shape: l.shape,
            rate: l.rate,
            ..Default::default()
        }),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shipped shape builds something that actually drives the
    /// parameter, and says which one it is afterwards.
    ///
    /// The round trip is the point: `attached` is what the fader reads to
    /// light the right menu entry, and a shape that builds fine but is
    /// unrecognisable leaves the control lying about its own state.
    #[test]
    fn every_shape_attaches_and_is_recognised_afterwards() {
        for (i, s) in SHAPES.iter().enumerate() {
            let mut g = NodeGraph::default();
            assert!(attach(&mut g, i, "/fx/glow"), "{} did not attach", s.name);
            assert!(driven(&g, "/fx/glow"), "{} drives nothing", s.name);
            assert_eq!(
                attached(&g, "/fx/glow"),
                Some(i),
                "{} was not recognised as itself",
                s.name
            );
            assert!(
                (0.0..=1.0).contains(&s.depth) && s.depth > 0.0,
                "{} has a depth that cannot move anything: {}",
                s.name,
                s.depth
            );
        }
    }

    /// The shapes must be distinguishable from each other, or `attached`
    /// lights whichever one it happens to hit first.
    #[test]
    fn no_two_shapes_build_the_same_thing() {
        for (i, s) in SHAPES.iter().enumerate() {
            let mut g = NodeGraph::default();
            attach(&mut g, i, "/a");
            assert_eq!(attached(&g, "/a"), Some(i), "{} collides with an earlier shape", s.name);
        }
        let mut names: Vec<_> = SHAPES.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "two shapes share a name");
    }

    /// Picking a second shape replaces the first rather than stacking.
    ///
    /// Two sinks on one address both apply, so a stacked "slow sweep"
    /// and "flicker" would be a fader doing both at once — which is not
    /// what picking from a menu means.
    #[test]
    fn attaching_again_replaces_rather_than_stacks() {
        let mut g = NodeGraph::default();
        attach(&mut g, 0, "/fx/glow");
        let after_first = g.nodes.len();
        attach(&mut g, 7, "/fx/glow");

        let sinks = g
            .nodes
            .iter()
            .filter(|n| matches!(&n.kind, NodeKind::Param { addr, .. } if addr == "/fx/glow"))
            .count();
        assert_eq!(sinks, 1, "the second shape stacked on the first");
        assert_eq!(attached(&g, "/fx/glow"), Some(7));
        assert!(
            g.nodes.len() > after_first,
            "the replacement did not build its own chain"
        );
    }

    /// Detaching leaves nothing behind, and leaves nothing of anyone
    /// else's either.
    #[test]
    fn detaching_takes_the_whole_chain_and_only_that_chain() {
        let mut g = NodeGraph::default();
        // Somebody's hand-built patch, plus a shape attached from a fader.
        let hand = g.add(NodeKind::Phasor { beats: 2.0 }, [0.0, 400.0]);
        let other = g.add(
            NodeKind::Param { addr: "/l1/phase".into(), depth: 1.0 },
            [200.0, 400.0],
        );
        g.connect(hand, other, 0);
        // And a node parked on the canvas, wired to nothing.
        g.add(NodeKind::Constant(0.5), [0.0, 600.0]);

        attach(&mut g, 7, "/fx/glow");
        assert!(detach(&mut g, "/fx/glow"));

        assert!(!driven(&g, "/fx/glow"), "the sink survived");
        assert!(
            !g.nodes.iter().any(|n| matches!(n.kind, NodeKind::Gate { .. })),
            "the shape left its operators behind: {:?}",
            g.nodes.iter().map(|n| n.kind.title()).collect::<Vec<_>>()
        );
        assert!(driven(&g, "/l1/phase"), "detaching took another chain's sink");
        assert_eq!(
            g.nodes.len(),
            3,
            "detaching disturbed the hand-built patch: {:?}",
            g.nodes.iter().map(|n| n.kind.title()).collect::<Vec<_>>()
        );
        // Detaching what is not there is a no-op, not a panic.
        assert!(!detach(&mut g, "/fx/glow"));
    }

    /// A source feeding both a shape and something else survives the
    /// detach, because the something else still needs it.
    #[test]
    fn a_shared_source_is_not_swept_up() {
        let mut g = NodeGraph::default();
        attach(&mut g, 0, "/fx/glow");
        let lfo = NodeId(0);
        // Tap the same LFO into a second parameter, as a patch would.
        let second = g.add(
            NodeKind::Param { addr: "/fx/trail".into(), depth: 0.4 },
            [600.0, 200.0],
        );
        g.connect(lfo, second, 0);

        detach(&mut g, "/fx/glow");
        assert!(
            driven(&g, "/fx/trail"),
            "the second parameter lost its sink"
        );
        assert!(
            g.nodes.iter().any(|n| matches!(n.kind, NodeKind::Lfo(_))),
            "the shared LFO was swept up with the shape"
        );
    }

    /// Recognition survives the graph actually running.
    ///
    /// An LFO carries its phase, its sample-and-hold value and its random
    /// seed inside the node, and all three move every frame. Comparing
    /// node kinds naively made `attached` correct for exactly as long as
    /// the patch sat still: one tick later the live LFO no longer equalled
    /// a freshly built one, every LFO shape reported "custom", and the
    /// fader's menu lost the tick beside the thing it was doing. Caught
    /// only because this test ticks; the round-trip test above passes
    /// either way.
    #[test]
    fn a_shape_is_still_recognised_after_the_graph_has_been_running() {
        let mut b = vizz_params::ParamRegistry::builder();
        b.add(vizz_params::ParamDef::new("/fx/glow", 0.0, 1.0, 0.0));
        let reg = b.build();

        for (i, s) in SHAPES.iter().enumerate() {
            let mut g = NodeGraph::default();
            attach(&mut g, i, "/fx/glow");
            let mut out = Vec::new();
            // A couple of seconds of playing, clock running.
            for f in 0..120 {
                g.tick(
                    1.0 / 60.0,
                    1.0 / 60.0,
                    f as f64 / 60.0,
                    crate::AudioLevels { bands: &[0.8, 0.2, 0.6, 0.4], level: 0.5 },
                    &reg,
                    &mut out,
                );
            }
            assert_eq!(
                attached(&g, "/fx/glow"),
                Some(i),
                "{} stopped recognising itself once it had run",
                s.name
            );
        }
    }

    /// A chain the user has rewired stops claiming to be a shape.
    ///
    /// The alternative is a menu with a tick beside "Slow sweep" on a
    /// patch that is now a kick envelope — the UI stating something the
    /// graph plainly contradicts.
    #[test]
    fn an_edited_chain_reports_no_shape_rather_than_the_wrong_one() {
        let mut g = NodeGraph::default();
        attach(&mut g, 0, "/fx/glow");
        assert_eq!(attached(&g, "/fx/glow"), Some(0));

        // Splice an operator in that no shape has.
        let extra = g.add(NodeKind::Math { op: crate::graph::MathOp::Add }, [0.0, 800.0]);
        let sink = sink_for(&g, "/fx/glow").unwrap();
        g.connect(extra, sink, 0);

        assert_eq!(attached(&g, "/fx/glow"), None, "an edited chain still claimed a shape");
        assert!(driven(&g, "/fx/glow"), "it is still modulated, just not by a shape");
    }

    /// Shapes attached one after another do not land on top of each
    /// other on the canvas.
    ///
    /// Not cosmetic: the argument for building these into the graph
    /// rather than hiding them is that you can go and read them, and a
    /// stack of nodes at one position is not readable.
    #[test]
    fn each_shape_gets_its_own_row() {
        let mut g = NodeGraph::default();
        attach(&mut g, 0, "/a");
        attach(&mut g, 2, "/b");
        attach(&mut g, 7, "/c");
        let mut rows: Vec<i32> = g.nodes.iter().map(|n| n.pos[1] as i32).collect();
        rows.sort_unstable();
        rows.dedup();
        assert_eq!(rows.len(), 3, "three shapes shared fewer than three rows: {rows:?}");
    }
}
