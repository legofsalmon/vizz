//! Canned camera paths, as an offset on top of what the faders say.
//!
//! A camera move is the one gesture a VJ cannot make by hand. Orbiting
//! smoothly for eight bars while also firing pads is two jobs and one
//! pair of hands, and the LFOs can drive `/camera/orbit` but only one
//! parameter at a time — a crane is elevation *and* distance moving
//! together on a shape neither of them knows about.
//!
//! # An offset, not a write
//!
//! Every move here returns a delta which the engine adds to the camera it
//! was already building. Nothing is written back into the parameter
//! store. That is the same rule modulation follows and for the same
//! reason: a fader you set stays where you set it, so switching a move
//! off returns you exactly to the framing you had, and a move running is
//! never a reason you cannot still steer.
//!
//! # Phase comes from the bar clock
//!
//! Rate is in bars rather than seconds, because a move that arrives back
//! where it started halfway through a phrase reads as a mistake. The
//! caller passes the beat count, so a move locks to the same clock the
//! sequencer and the LFOs are on.

use glam::Vec3;

/// The moves, in the order `/camera/move` selects them. Index 0 is off.
///
/// Ordered roughly by how much they disturb the framing: the first few
/// are things you can leave running under a whole song, the last few
/// take the camera somewhere you have to mean.
pub const MOVES: &[&str] = &[
    "off",
    "orbit",
    "sway",
    "push",
    "pull",
    "crane",
    "spiral",
    "look around",
    "fly through",
    "walkthrough",
    "drift",
];

/// What a move asks of the camera this frame. All additive.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Move {
    pub orbit: f32,
    pub elevation: f32,
    pub distance: f32,
    /// World-space offset of the point the camera is aimed at.
    pub at: Vec3,
}

impl Move {
    /// The same move, scaled — used by the engine's fade so switching a
    /// move on, off, or across to another one is a hand-off rather than a
    /// cut.
    pub fn scaled(self, k: f32) -> Self {
        Self {
            orbit: self.orbit * k,
            elevation: self.elevation * k,
            distance: self.distance * k,
            at: self.at * k,
        }
    }
}

/// A smooth 0→1→0 over one cycle, without the corner a triangle has.
fn there_and_back(phase: f32) -> f32 {
    0.5 - 0.5 * (phase * std::f32::consts::TAU).cos()
}

/// Cheap deterministic wander on one axis, for the drift move.
///
/// Deterministic rather than random because a move has to be the same
/// move on the second night: a drift that wanders somewhere different
/// each launch is one you cannot rehearse against, and the whole point of
/// a canned path is that you know where it goes.
///
/// The three harmonics are **whole numbers** of the cycle, which is what
/// makes drift periodic in phase and therefore continuous across the
/// wrap. Written first with incommensurate periods — 0.37, 0.19, 0.11 —
/// which reads better on paper and snaps the camera every time the phase
/// crosses 1, because `wander(1)` and `wander(0)` are different numbers.
/// One and seven are far enough apart to still not read as a cycle inside
/// a song at any sensible bar length.
fn wander(phase: f32, seed: f32) -> f32 {
    let t = phase * std::f32::consts::TAU;
    (t + seed).sin() * 0.6 + (t * 3.0 + seed * 2.3).sin() * 0.3
        + (t * 7.0 + seed * 5.1).sin() * 0.1
}

/// Roughly how big the world is.
///
/// Every imported cloud is run through `pointcloud::normalize`, which
/// centres it and scales it to about a unit across, and the procedural
/// shapes are built at the same scale. So a move that travels three units
/// leaves the subject behind entirely — which is exactly what the first
/// version of the walkthrough did: it rendered a black frame, because the
/// numbers were picked for a room-sized scan and every scan here is
/// normalised to the size of a grapefruit.
///
/// Named, so the paths below are written in multiples of "the thing you
/// are looking at" rather than in numbers that happen to work.
const WORLD: f32 = 1.2;

/// How close the camera gets when a move puts you inside the field. Just
/// outside the 0.05 near plane, so points do not clip through the lens.
const INSIDE: f32 = 0.5;

/// The offset for `move_index` at `beats`, scaled by `size` (0..1).
///
/// `bars` is the length of one cycle. `size` scales how far the move
/// travels, not how fast — the two are separate because "the same move,
/// smaller" is a thing you want during a quiet section and "the same
/// move, slower" is a different thing entirely.
pub fn offset(move_index: usize, beats: f64, bars: f32, size: f32) -> Move {
    let size = size.clamp(0.0, 1.0);
    if move_index == 0 || size <= 0.0 {
        return Move::default();
    }
    let bars = bars.max(0.25);
    // One cycle per `bars` bars, four beats to the bar.
    let phase = ((beats / (bars as f64 * 4.0)) % 1.0) as f32;
    let tau = std::f32::consts::TAU;
    let t = phase * tau;
    // Seconds-ish, for the moves that want to keep going rather than
    // return: a whole number of cycles is still a whole number of turns.
    let ramp = phase;
    let mut m = Move::default();
    match MOVES.get(move_index).copied().unwrap_or("off") {
        // A full turn per cycle. The one move that is genuinely
        // continuous — it ends where it began, so it can run all night.
        "orbit" => m.orbit = ramp * tau * size,
        // Handheld: a partial turn back and forth, with the horizon
        // breathing under it. Not a full turn, because the point is that
        // it looks like somebody holding the camera rather than a motor.
        "sway" => {
            m.orbit = t.sin() * 0.5 * size;
            m.elevation = (t * 0.5).sin() * 0.12 * size;
        }
        // In and back out. Distance is negative because closer is a
        // smaller number, and a "push" that pulled would be a bug nobody
        // would report because they would just stop using it.
        "push" => m.distance = -there_and_back(phase) * 2.2 * size,
        "pull" => m.distance = there_and_back(phase) * 4.0 * size,
        // Rising over the top. Elevation is clamped upstream at ±1.4, so
        // this asks for most of the way and lets the clamp hold it.
        "crane" => {
            m.elevation = there_and_back(phase) * 1.0 * size;
            m.distance = there_and_back(phase) * 0.8 * size;
        }
        // A helix: a full turn while rising and closing. The move that
        // does the most per bar, and the one to reach for on a drop.
        "spiral" => {
            m.orbit = ramp * tau * size;
            m.elevation = (t * 0.5).sin() * 0.7 * size;
            m.distance = -there_and_back(phase) * 1.6 * size;
        }
        // Standing still and turning your head: the camera holds its
        // position and the *target* sweeps, which is the opposite of an
        // orbit and reads completely differently.
        "look around" => {
            // A head turn, so the arc is modest: the first version swung
            // `cos t - 1`, which is two units of travel on a subject one
            // unit across and put it off frame at the far end.
            m.at = Vec3::new(t.sin(), (t * 0.5).sin() * 0.3, (t.cos() - 1.0) * 0.5)
                * (WORLD * 0.7)
                * size;
        }
        // Straight through the middle and out the far side, then round
        // again. The target runs along Z and the camera follows it in.
        "fly through" => {
            // In through the front, out the back, and back again — a
            // cosine rather than a ramp.
            //
            // The ramp version travelled from one side to the other and
            // then teleported back to the start, which the cycle test did
            // not catch because it only compared orbit, elevation and
            // distance. It also spent the last part of every cycle
            // looking at nothing, having left the field behind. A cosine
            // closes exactly and passes through twice.
            let travel = (phase * std::f32::consts::TAU).cos();
            m.at = Vec3::new(0.0, 0.0, travel * WORLD * 1.5 * size);
            m.distance = -(3.5 - INSIDE) * size;
        }
        // Moving through a space and turning corners, like walking
        // through a house: four legs to a cycle, each one a straight run
        // followed by a quarter turn. The turn happens while still
        // moving, or it reads as a robot rather than a person.
        "walkthrough" => {
            let leg = phase * 4.0;
            let which = leg.floor();
            let along = leg - which;
            // Ease the turn into the last third of each leg.
            let turning = ((along - 0.66) / 0.34).clamp(0.0, 1.0);
            let run = WORLD * size;
            // Where you are walking, and where you are looking, are two
            // different headings — which is the whole of turning a
            // corner: your head goes first and your feet follow.
            //
            // Written at first with one heading for both, which meant the
            // last third of every leg travelled in the *next* leg's
            // direction. The path did not close, so the loop teleported
            // once a cycle, and the square was not a square.
            let heading = which * std::f32::consts::FRAC_PI_2;
            let (hs, hc) = heading.sin_cos();
            let base = corner(which as u32, run);
            m.at = base + Vec3::new(hs, 0.0, hc) * (along * run);
            // Facing the way you are going. Half a turn, because the eye
            // sits at `target + back * distance` and `back` points *from*
            // the subject *towards* the camera — so an orbit equal to the
            // heading puts the camera ahead of you looking back over your
            // shoulder, which is what the first version did and what the
            // frame showed: the field pinned to one edge, receding.
            m.orbit = (which + turning) * std::f32::consts::FRAC_PI_2 + std::f32::consts::PI;
            // Down at eye level rather than looking in from above, and
            // close, because the whole idea is being inside it.
            m.elevation = -0.3 * size;
            m.distance = -(3.5 - INSIDE) * size;
        }
        // Very slow, never repeating inside a song, always moving. The
        // move for a long ambient section where a still camera would
        // read as a frozen output.
        "drift" => {
            m.orbit = wander(phase, 0.0) * 0.6 * size;
            m.elevation = wander(phase, 11.0) * 0.25 * size;
            m.distance = wander(phase, 23.0) * 1.2 * size;
            m.at = Vec3::new(
                wander(phase, 31.0),
                wander(phase, 47.0) * 0.4,
                wander(phase, 59.0),
            ) * (WORLD * 0.5)
                * size;
        }
        _ => {}
    }
    m
}

/// Where the walkthrough's `n`th leg starts: the corners of a square,
/// walked in order.
///
/// Centred on the origin rather than starting there. A square with one
/// corner at the origin is a loop *beside* the subject, and walking it
/// puts the thing you came to look at outside the square the whole way
/// round — which on screen was the field pinned to the edge of frame
/// while the camera toured the empty space next to it.
fn corner(n: u32, run: f32) -> Vec3 {
    let h = run * 0.5;
    match n % 4 {
        0 => Vec3::new(-h, 0.0, -h),
        1 => Vec3::new(-h, 0.0, h),
        2 => Vec3::new(h, 0.0, h),
        _ => Vec3::new(h, 0.0, -h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;

    /// Off is off, and so is a move at zero size. Both have to be exactly
    /// nothing rather than nearly nothing: this offset is added to the
    /// camera every frame, and "nearly" is a picture that drifts.
    #[test]
    fn nothing_moves_when_nothing_was_asked_for() {
        assert_eq!(offset(0, 123.0, 8.0, 1.0), Move::default());
        for i in 1..MOVES.len() {
            assert_eq!(offset(i, 123.0, 8.0, 0.0), Move::default(), "move {i}");
        }
    }

    /// Every move returns to where it started after one cycle, or it
    /// cannot be left running: a camera that ratchets a little further
    /// out on each pass ends the night pointing at nothing.
    ///
    /// Orbit and spiral are the deliberate exceptions — they turn a whole
    /// turn per cycle, which lands back on the same *picture* even though
    /// the number has grown.
    #[test]
    fn a_cycle_lands_back_where_it_began() {
        let bars = 8.0;
        let cycle = (bars * 4.0) as f64;
        for (i, name) in MOVES.iter().enumerate().skip(1) {
            let start = offset(i, 0.0, bars, 1.0);
            let end = offset(i, cycle - 1e-6, bars, 1.0);
            let turn = std::f32::consts::TAU;
            let orbit_ok = if matches!(*name, "orbit" | "spiral" | "walkthrough") {
                // A whole turn (or, for the walkthrough, four quarters).
                // The walkthrough's constant half-turn — it faces the way
                // it is walking — cancels in the difference.
                (end.orbit - start.orbit - turn).abs() < 0.02
            } else {
                (end.orbit - start.orbit).abs() < 0.02
            };
            assert!(orbit_ok, "{name}: orbit {} → {}", start.orbit, end.orbit);
            assert!(
                (end.elevation - start.elevation).abs() < 0.02,
                "{name}: elevation {} → {}",
                start.elevation,
                end.elevation
            );
            assert!(
                (end.distance - start.distance).abs() < 0.02,
                "{name}: distance {} → {}",
                start.distance,
                end.distance
            );
            // And where it is aimed, which the first version of this test
            // left out — so a fly-through that ramped from one side of
            // the field to the other and teleported back passed it.
            assert!(
                (end.at - start.at).length() < 0.02,
                "{name}: aimed at {:?} → {:?}",
                start.at,
                end.at
            );
        }
    }

    /// No move may leave the subject behind.
    ///
    /// This is the one the walkthrough failed on screen and no test
    /// noticed: the paths were written in units that suited a scanned
    /// room, every cloud is normalised to about a unit across, and the
    /// camera walked three widths away from a one-width object. The
    /// frame was black, and every other test here passed.
    #[test]
    fn no_move_leaves_the_subject_behind() {
        let bars = 8.0;
        for (i, name) in MOVES.iter().enumerate().skip(1) {
            for step in 0..128 {
                let beats = step as f64 * (bars as f64 * 4.0) / 128.0;
                let m = offset(i, beats, bars, 1.0);
                // Aimed no further out than a couple of the thing's own
                // widths: past that the field is off the edge of frame.
                assert!(
                    m.at.length() <= WORLD * 2.0 + 1e-3,
                    "{name} aims {:.2} away at beat {beats:.1}, which is off frame",
                    m.at.length()
                );
                // And still in front of the lens rather than behind it.
                let dist = Camera::default().distance + m.distance;
                assert!(
                    dist > 0.05,
                    "{name} puts the camera at distance {dist:.2} at beat {beats:.1}"
                );
            }
        }
    }

    /// A move has to actually move. A named path that returns almost
    /// nothing would pass every other test here and do nothing on screen,
    /// which is exactly the failure a list of ten shortcuts invites.
    #[test]
    fn every_move_travels_somewhere() {
        let bars = 8.0;
        for (i, name) in MOVES.iter().enumerate().skip(1) {
            let travel = (0..64)
                .map(|s| {
                    let m = offset(i, s as f64 * 0.5, bars, 1.0);
                    m.orbit.abs() + m.elevation.abs() + m.distance.abs() + m.at.length()
                })
                .fold(0.0f32, f32::max);
            assert!(travel > 0.2, "{name} barely moves: {travel}");
        }
    }

    /// Size scales travel without changing the shape of the path.
    /// Size scales travel, and scales it linearly, so half size is
    /// recognisably the same move rather than a different one.
    ///
    /// The walkthrough's *heading* is the deliberate exception, and the
    /// test says so rather than the code bending to it: you turn a corner
    /// by ninety degrees whether the rooms are large or small, and a
    /// half-size walkthrough that only turned forty-five would walk into
    /// the wall.
    #[test]
    fn size_scales_the_move_rather_than_its_speed() {
        for (i, name) in MOVES.iter().enumerate().skip(1) {
            let full = offset(i, 9.0, 8.0, 1.0);
            let half = offset(i, 9.0, 8.0, 0.5);
            if *name != "walkthrough" {
                assert!(
                    (half.orbit * 2.0 - full.orbit).abs() < 1e-4,
                    "{name}: orbit is not linear in size"
                );
            }
            assert!(
                (half.distance * 2.0 - full.distance).abs() < 1e-4,
                "{name}: distance is not linear in size"
            );
            assert!(
                (half.at * 2.0 - full.at).length() < 1e-4,
                "{name}: travel is not linear in size"
            );
        }
    }

    /// Drift has to be continuous across the phase wrap, which is only
    /// true because its harmonics are whole numbers of the cycle. The
    /// readable version — three incommensurate periods — snaps the
    /// camera once per cycle, which on a slow ambient drift is the one
    /// visible thing it does.
    #[test]
    fn drift_does_not_snap_when_the_phase_wraps() {
        let i = MOVES.iter().position(|m| *m == "drift").unwrap();
        let bars = 8.0;
        let cycle = (bars * 4.0) as f64;
        let before = offset(i, cycle - 1e-4, bars, 1.0);
        let after = offset(i, 0.0, bars, 1.0);
        assert!(
            (before.at - after.at).length() < 1e-3
                && (before.orbit - after.orbit).abs() < 1e-3
                && (before.distance - after.distance).abs() < 1e-3,
            "drift jumps at the wrap: {before:?} then {after:?}"
        );
    }

    /// Bars sets the period. Two cycles at four bars must be one cycle at
    /// eight, or the rate control is not a rate control.
    #[test]
    fn bars_sets_the_period() {
        let a = offset(1, 16.0, 4.0, 1.0);
        let b = offset(1, 32.0, 8.0, 1.0);
        assert!((a.orbit - b.orbit).abs() < 1e-4, "{a:?} vs {b:?}");
    }

    /// The walkthrough is the one that has to end up somewhere else: it
    /// is a path through a space, not a wobble around a point.
    #[test]
    fn the_walkthrough_actually_goes_somewhere() {
        let bars = 8.0;
        let cycle = (bars * 4.0) as f64;
        let i = MOVES.iter().position(|m| *m == "walkthrough").unwrap();
        let quarter = offset(i, cycle * 0.25, bars, 1.0);
        assert!(
            quarter.at.length() > WORLD * 0.4,
            "a quarter of the way through it has barely left: {:?}",
            quarter.at
        );
        // And it is facing along the leg it is walking, not backwards.
        let half = offset(i, cycle * 0.5, bars, 1.0);
        // Half way round the square it has turned two of its four
        // corners, on top of the constant half turn that faces it
        // forwards in the first place.
        assert!(
            (half.orbit - std::f32::consts::TAU).abs() < 0.2,
            "half way round it should have turned two corners, got {}",
            half.orbit
        );
    }
}
