//! What a licence status costs, and when that cost may change.
//!
//! Two decisions, both pure:
//!
//! - [`restriction`] — given a status, what an unlicensed copy does. One
//!   function, one table, every case tested.
//! - [`Session`] — when the answer is allowed to change. It is read once
//!   when a session starts; afterwards it may only get *better*. A trial
//!   that runs out at 23:59, a lease that lapses mid-set, a clock that
//!   jumps: none of them may put a mark on a running projector feed or
//!   stop one. Rule 1 of the integration: nothing licence-related stops a
//!   running show.

use crate::verify::Status;

/// What an unlicensed copy of vizz does. **This is the switch.**
///
/// One constant in one place rather than a setting: changing it is a
/// one-line change and a release, never something a user can toggle.
///
/// - `Open` — no licence behaves exactly as vizz always has. The licence
///   section says "Unlicensed" and offers a key or a trial; nothing is
///   restricted.
/// - `Watermark` — without a usable licence the app keeps working fully
///   but its output carries a "VIZZ · UNLICENSED" mark, burned into the
///   master so it reaches Syphon, NDI, recordings and the preview alike.
///   A trial or a key removes it on the spot.
/// - `Lock` — without a usable licence a new session does not start: the
///   app opens on the licence section and publishes nothing (the master
///   is black) until a trial or a key is entered, and then starts at
///   once. A session already running is never stopped.
///
/// `Lock` is the owner's choice — "trial, then lock", as Light does: the
/// trial is the way in, and a finished trial or no licence at all means
/// the next launch waits on the licence screen. Headless runs, which have
/// no screen to wait on, get the mark instead; see [`headless`].
pub const POLICY: Policy = Policy::Lock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Open,
    Watermark,
    Lock,
}

/// What the current session has to do about the licence. Ordered from
/// least to most restrictive, which is what lets [`Session`] say "only
/// ever downwards" as a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Restriction {
    /// Nothing.
    None,
    /// Mark the output.
    Watermark,
    /// Do not start publishing.
    Lock,
}

/// Whether a status is a licence this copy may run as licensed.
///
/// Usable:
///
/// - `Active`.
/// - `UpdateRequired` — a bought licence is permanent; only the update
///   entitlement lapsed. Being outside it costs newer builds, never the
///   build you have.
/// - `CheckInRequired` — a paying customer who has been offline past the
///   lease. The fix is a heartbeat the app makes by itself; the status is
///   a note, never a penalty.
///
/// Not usable: `Invalid` (no licence, or one that does not verify or is
/// for another product), `Expired` (a trial that is over) and
/// `WrongMachine` (a token copied from somewhere else).
pub fn usable(status: Status) -> bool {
    matches!(status, Status::Active | Status::UpdateRequired | Status::CheckInRequired)
}

/// The one place the product decides what a status costs.
///
/// `key_configured` is false for a build with no usable verifying key.
/// Such a build can verify nothing, so every status it computes is
/// `Invalid` — and restricting on that would mark or lock every copy of
/// it, silently, looking exactly like a licensing decision. A build that
/// cannot check never restricts.
pub fn restriction(status: Status, policy: Policy, key_configured: bool) -> Restriction {
    if !key_configured || usable(status) {
        return Restriction::None;
    }
    match policy {
        Policy::Open => Restriction::None,
        Policy::Watermark => Restriction::Watermark,
        Policy::Lock => Restriction::Lock,
    }
}

/// The restriction for a headless run.
///
/// Headless is the benchmark and CI entry point, and it has no panel to
/// type a key into, so `Lock` cannot mean "wait on the licence screen"
/// there. It degrades to the mark instead: a headless Syphon or NDI
/// source is still output, and unlicensed output is still marked.
pub fn headless(restriction: Restriction) -> Restriction {
    restriction.min(Restriction::Watermark)
}

/// The words on the mark: why this copy carries one.
pub fn mark_text(status: Status) -> &'static str {
    match status {
        Status::Expired => "VIZZ · TRIAL ENDED",
        _ => "VIZZ · UNLICENSED",
    }
}

/// The restriction a session is running under.
///
/// Fixed when the session starts; afterwards only ever lifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    current: Restriction,
}

impl Session {
    pub fn begin(restriction: Restriction) -> Self {
        Session { current: restriction }
    }

    pub fn restriction(&self) -> Restriction {
        self.current
    }

    /// Publishing has not started and must not until a licence arrives.
    pub fn locked(&self) -> bool {
        self.current == Restriction::Lock
    }

    /// The output carries the mark.
    pub fn marked(&self) -> bool {
        self.current == Restriction::Watermark
    }

    /// A fresh verdict arrived. It can lift the restriction — a trial
    /// started, a key entered — and never impose one. Returns whether
    /// anything changed, so the caller can say so once.
    pub fn relax(&mut self, restriction: Restriction) -> bool {
        if restriction < self.current {
            self.current = restriction;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole table: every status under every policy, with and without
    /// a key to verify against.
    #[test]
    fn every_status_under_every_policy() {
        use Restriction as R;
        use Status as S;
        #[rustfmt::skip]
        let table = [
            //  status               Open     Watermark     Lock
            (S::Active,          [R::None, R::None,      R::None]),
            (S::UpdateRequired,  [R::None, R::None,      R::None]),
            (S::CheckInRequired, [R::None, R::None,      R::None]),
            (S::Expired,         [R::None, R::Watermark, R::Lock]),
            (S::WrongMachine,    [R::None, R::Watermark, R::Lock]),
            (S::Invalid,         [R::None, R::Watermark, R::Lock]),
        ];
        assert_eq!(table.len(), Status::ALL.len(), "a status is missing from the table");
        for (status, expect) in table {
            for (policy, want) in [Policy::Open, Policy::Watermark, Policy::Lock].into_iter().zip(expect) {
                assert_eq!(restriction(status, policy, true), want, "{status:?} under {policy:?}");
                // Rule 4: a build that cannot verify restricts nothing,
                // whatever the policy says.
                assert_eq!(
                    restriction(status, policy, false),
                    R::None,
                    "{status:?} under {policy:?} restricted a build with no key"
                );
            }
        }
    }

    /// Rule 2, stated on its own because it is the one most likely to be
    /// "tightened" by someone later: an update window that closed and a
    /// lease that lapsed are notes, under every policy.
    #[test]
    fn update_required_and_check_in_required_never_restrict() {
        for policy in [Policy::Open, Policy::Watermark, Policy::Lock] {
            assert_eq!(restriction(Status::UpdateRequired, policy, true), Restriction::None);
            assert_eq!(restriction(Status::CheckInRequired, policy, true), Restriction::None);
        }
    }

    /// Having no licence costs the same as a finished trial. If it cost
    /// less, starting a trial would be strictly worse than never starting
    /// one — the mistake Light made first.
    #[test]
    fn no_licence_is_never_better_than_a_finished_trial() {
        for policy in [Policy::Open, Policy::Watermark, Policy::Lock] {
            assert_eq!(
                restriction(Status::Invalid, policy, true),
                restriction(Status::Expired, policy, true)
            );
        }
    }

    #[test]
    fn the_shipped_policy_is_trial_then_lock() {
        // Flipping this is a product decision; the test makes it a
        // visible one in review rather than a one-character diff.
        assert_eq!(POLICY, Policy::Lock);
    }

    #[test]
    fn headless_never_locks_but_still_marks() {
        assert_eq!(headless(Restriction::Lock), Restriction::Watermark);
        assert_eq!(headless(Restriction::Watermark), Restriction::Watermark);
        assert_eq!(headless(Restriction::None), Restriction::None);
    }

    /// Rule 1: a session's restriction can be lifted and never imposed.
    #[test]
    fn a_running_session_only_ever_gets_less_restricted() {
        // Licensed at launch; the trial runs out mid-set. Nothing changes.
        let mut s = Session::begin(Restriction::None);
        assert!(!s.relax(Restriction::Watermark));
        assert!(!s.relax(Restriction::Lock));
        assert!(!s.marked() && !s.locked());

        // Marked at launch; a key is entered. The mark goes at once, and a
        // later lapse does not bring it back.
        let mut s = Session::begin(Restriction::Watermark);
        assert!(s.marked());
        assert!(s.relax(Restriction::None));
        assert!(!s.marked());
        assert!(!s.relax(Restriction::Watermark));
        assert!(!s.marked());

        // Locked at launch; a trial starts. Publishing begins, and nothing
        // afterwards can stop it.
        let mut s = Session::begin(Restriction::Lock);
        assert!(s.locked());
        assert!(!s.relax(Restriction::Lock), "the same answer is not a change");
        assert!(s.relax(Restriction::None));
        assert!(!s.locked());
        assert!(!s.relax(Restriction::Lock));
        assert!(!s.locked());
    }
}
