//! Getting the picture back after the GPU device is lost.
//!
//! A driver reset, a GPU switch on a dual-GPU laptop, an external GPU
//! pulled: wgpu reports it once, through the device-lost callback, and
//! every call on that device fails from then on. Until this existed the
//! loss was logged and recorded and nothing else happened, so every frame
//! after it failed, was caught and was skipped, and the projector held the
//! last good picture until somebody relaunched.
//!
//! The rebuild itself is in `windowed.rs`: a new instance, adapter,
//! device and surface on the same window, then the render state built on
//! them from what the app already knows. This is only its bookkeeping —
//! whether a rebuild is due and how far apart the attempts are — kept
//! apart so it can be tested without a GPU.
//!
//! The one thing it has to prevent is a rebuild loop. A GPU that is gone
//! for good fails every attempt, and one that is dying may hand out a
//! device that is lost again straight away. Either would otherwise rebuild
//! as fast as the event loop turns, which starves the panel and fills the
//! log. So attempts back off, doubling from a second to half a minute,
//! and a rebuilt device only resets that once it has stayed up for a
//! while.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The longest wait between two attempts. Long enough that a GPU gone for
/// good costs nothing noticeable; short enough that one which comes back
/// — a driver that finishes resetting, a cable pushed back in — is picked
/// up within the same song.
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// How long a rebuilt device has to last before its rebuild counts as
/// having worked. One lost sooner carries on the streak, and so waits out
/// a longer back-off, instead of starting again at the shortest.
pub const STABLE: Duration = Duration::from_secs(60);

/// Whether a lost device is one to rebuild.
///
/// Every device reports `Destroyed` on its way out, including the ones
/// this module replaces and the one dropped at quit. Rebuilding on that
/// would be rebuilding on every rebuild.
pub fn needs_rebuild(reason: wgpu::DeviceLostReason) -> bool {
    reason != wgpu::DeviceLostReason::Destroyed
}

/// The wait after attempt `streak` before the next one may start.
pub fn backoff(streak: u32) -> Duration {
    let secs = 1u64 << streak.saturating_sub(1).min(16);
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

#[derive(Default)]
pub struct DeviceRecovery {
    /// Raised by the device-lost callback, on whatever thread wgpu calls
    /// it from, and by a failed attempt.
    lost: Arc<AtomicBool>,
    /// Attempts in the current run of losses.
    streak: u32,
    /// No attempt before this.
    not_before: Option<Instant>,
    /// When the last attempt succeeded, until the next loss.
    rebuilt_at: Option<Instant>,
}

impl DeviceRecovery {
    /// The flag for the device-lost callback to raise.
    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.lost)
    }

    /// The device is gone and has not been replaced yet.
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// Start an attempt if one is due, returning its number in the streak.
    ///
    /// The flag is lowered as the attempt starts, so the new device being
    /// lost raises it again, and the next attempt is scheduled whether or
    /// not this one works.
    pub fn begin(&mut self, now: Instant) -> Option<u32> {
        if !self.lost() || self.not_before.is_some_and(|t| now < t) {
            return None;
        }
        // A device that stayed up ends the streak; one that did not is
        // part of it. Taken either way, so a later failure is not
        // measured against a success from before it.
        if self
            .rebuilt_at
            .take()
            .is_some_and(|t| now.saturating_duration_since(t) >= STABLE)
        {
            self.streak = 0;
        }
        self.streak = self.streak.saturating_add(1);
        self.not_before = Some(now + backoff(self.streak));
        self.lost.store(false, Ordering::Release);
        Some(self.streak)
    }

    /// The attempt did not produce a device. Tried again once the
    /// back-off `begin` set has passed.
    pub fn failed(&mut self) {
        self.lost.store(true, Ordering::Release);
    }

    /// The attempt produced a working renderer.
    pub fn succeeded(&mut self, now: Instant) {
        self.rebuilt_at = Some(now);
    }

    /// When the next attempt may start, while one is waiting.
    pub fn next_attempt(&self) -> Option<Instant> {
        self.not_before.filter(|_| self.lost())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lose(r: &DeviceRecovery) {
        r.flag().store(true, Ordering::Release);
    }

    #[test]
    fn nothing_is_rebuilt_until_the_device_is_lost() {
        let mut r = DeviceRecovery::default();
        assert!(!r.lost());
        assert_eq!(r.begin(Instant::now()), None);
    }

    /// Only a loss the app did not ask for is rebuilt from. Every device
    /// says `Destroyed` when dropped — including each one a rebuild
    /// replaces — so treating that as a loss would rebuild forever.
    #[test]
    fn a_device_dropped_on_purpose_is_not_rebuilt() {
        assert!(!needs_rebuild(wgpu::DeviceLostReason::Destroyed));
        assert!(needs_rebuild(wgpu::DeviceLostReason::Unknown));
    }

    #[test]
    fn the_first_attempt_is_immediate_and_lowers_the_flag() {
        let mut r = DeviceRecovery::default();
        lose(&r);
        let now = Instant::now();
        assert_eq!(r.begin(now), Some(1));
        assert!(!r.lost(), "the attempt left the flag up");
        assert_eq!(r.begin(now), None, "a second attempt started with nothing lost");
    }

    /// A GPU that stays gone is retried further and further apart, and
    /// never faster than once a second.
    #[test]
    fn failed_attempts_back_off_up_to_a_ceiling() {
        let mut r = DeviceRecovery::default();
        lose(&r);
        let mut now = Instant::now();
        let mut gaps = Vec::new();
        for n in 1..=8 {
            assert_eq!(r.begin(now), Some(n), "attempt {n} did not start when due");
            r.failed();
            assert!(r.lost(), "a failed attempt was forgotten");
            let next = r.next_attempt().expect("a failed attempt schedules the next");
            let gap = next - now;
            // Not a moment before.
            assert_eq!(r.begin(next - Duration::from_millis(1)), None);
            gaps.push(gap.as_secs());
            now = next;
        }
        assert_eq!(gaps, [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    /// A rebuilt device that is lost again at once must not be rebuilt at
    /// once: that is the loop. It carries on the streak and waits.
    #[test]
    fn a_device_lost_again_soon_after_a_rebuild_waits_its_turn() {
        let mut r = DeviceRecovery::default();
        let start = Instant::now();
        let mut now = start;
        let mut attempts = Vec::new();
        // Each rebuild works, and each new device dies a moment later.
        while now < start + Duration::from_secs(60) {
            lose(&r);
            match r.begin(now) {
                Some(n) => {
                    attempts.push(n);
                    r.succeeded(now);
                }
                None => {
                    // Still waiting; the loss stays raised.
                }
            }
            now += Duration::from_millis(100);
        }
        // A doubling back-off from one second: attempts at 0, 1, 3, 7,
        // 15 and 31 seconds in a minute, not six hundred.
        assert_eq!(attempts, [1, 2, 3, 4, 5, 6]);
    }

    /// A device that lasted is a recovery that worked: the next loss,
    /// whenever it comes, is treated as a first.
    #[test]
    fn a_device_that_stayed_up_starts_the_next_loss_afresh() {
        let mut r = DeviceRecovery::default();
        let mut now = Instant::now();
        for _ in 0..4 {
            lose(&r);
            r.begin(now).expect("due");
            r.failed();
            now = r.next_attempt().expect("scheduled");
        }
        assert_eq!(r.begin(now), Some(5));
        r.succeeded(now);

        now += STABLE;
        lose(&r);
        assert_eq!(r.begin(now), Some(1), "a device that lasted did not end the streak");
    }

    /// The streak is judged against the last success only once: attempts
    /// failing long after an old success must still back off.
    #[test]
    fn failures_after_an_old_success_still_back_off() {
        let mut r = DeviceRecovery::default();
        let mut now = Instant::now();
        lose(&r);
        r.begin(now).expect("due");
        r.succeeded(now);

        now += STABLE * 2;
        lose(&r);
        assert_eq!(r.begin(now), Some(1));
        r.failed();
        now = r.next_attempt().expect("scheduled");
        assert_eq!(r.begin(now), Some(2), "the old success reset the streak again");
    }
}
