//! Pure, platform-independent signal-wait state-machine logic for LearnOS.
//!
//! These functions contain NO bare-metal dependencies (no globals, no I/O, no
//! atomics) so they can be unit-tested on the host with `cargo test`. The kernel
//! (`process.rs`, `syscall.rs`) calls into this crate for its deadline math and
//! wakeup predicate, keeping the platform-dependent code in one place.
//!
//! `#![no_std]` is applied unless we are building for the host test harness, so
//! the same crate compiles both inside the no_std kernel binary and under
//! `cargo test`.
#![cfg_attr(not(test), no_std)]

/// Compute the absolute deadline (in kernel ticks) for a signal wait whose
/// timeout is expressed in ticks.
///
/// A `timeout` of 0 means "poll" — callers should return immediately without
/// blocking; this still returns `now`, which is safe because the caller checks
/// `timeout == 0` before deciding to block.
///
/// LESSON (overflow): `now + timeout` on u64 wraps in release and PANICS in
/// debug. A huge user-supplied timeout near `u64::MAX` would crash the kernel.
/// `saturating_add` clamps at `u64::MAX` instead: the deadline simply becomes
/// "as late as representable", which `deadline_expired` still handles fine.
#[allow(clippy::arithmetic_implicit_inequalities)]
pub fn compute_deadline(now: u64, timeout: u64) -> u64 {
    now.saturating_add(timeout)
}

/// Return true if a signal waiter's deadline has elapsed at the current tick.
///
/// A `deadline` of 0 means "no deadline" (wait indefinitely until a signal
/// arrives); such a waiter is never woken by the timer on its own.
#[allow(clippy::arithmetic_implicit_inequalities)]
pub fn deadline_expired(deadline: u64, now: u64) -> bool {
    deadline != 0 && deadline <= now
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_deadline_adds_timeout_to_now() {
        assert_eq!(compute_deadline(100, 50), 150);
        assert_eq!(compute_deadline(0, 0), 0);
        // Overflow saturates at u64::MAX instead of wrapping/panicking: a
        // huge user timeout near the end of the tick counter just means
        // "as late as representable".
        assert_eq!(compute_deadline(u64::MAX, 1), u64::MAX);
        assert_eq!(compute_deadline(u64::MAX - 5, 10), u64::MAX);
    }

    #[test]
    fn deadline_zero_never_expires() {
        // A waiter with no deadline (deadline == 0) must never be woken by the
        // timer on its own — it only wakes when a signal arrives.
        assert!(!deadline_expired(0, 0));
        assert!(!deadline_expired(0, u64::MAX));
    }

    #[test]
    fn deadline_expires_at_or_after_target() {
        assert!(!deadline_expired(150, 149));
        assert!(deadline_expired(150, 150)); // exactly at deadline -> wake
        assert!(deadline_expired(150, 200)); // past deadline -> wake
    }

    #[test]
    fn deadline_not_before_target() {
        assert!(!deadline_expired(150, 100));
    }
}
