# Review — Phase 4: Signals, real blocking & wakeup

**Date:** 2026-09-25
**Files reviewed:** 5 (`signal_state/src/lib.rs`, `signal_state/Cargo.toml`, `kernel/src/syscall.rs` `sys_wait_signal`, `kernel/src/signal.rs` `send_signal`, `kernel/src/process.rs` clock/deadline/`check_deadlines`)
**Result:** ✅ APPROVED

## Plan Compliance

| Expected output | Status | Observation |
|-----------------|--------|-------------|
| `syscall.rs`: corrected `sys_wait_signal` (real blocking) + wakeup path | ✅ Compliant | Rewritten: inline returns pending sig; else records deadline, sets `state=Blocked`, yields via `schedule_next()`. Poll (`timeout==0`) returns `-1` immediately. |
| `process.rs` / scheduler: blocked-for-signal state + timer-based wakeup | ✅ Compliant | Reused `ProcessState::Blocked` + new `Process.deadline: u64`; `check_deadlines()` called from `timer_tick()` before `schedule_next()`. |
| New/updated host tests for wait/send cycle | ✅ Compliant | New workspace crate `signal_state/` with 4 unit tests. **Verified:** `cargo test -p signal_state --target x86_64-unknown-linux-gnu` → 4 passed. |
| Bare-metal kernel build compiles cleanly | ✅ Compliant | **Verified:** `cargo +nightly build ... --bin kernel` → clean (no errors; 84 pre-existing warnings, none from phase-4 code). |

## Issues Found

### 🔴 Critical (block next phase)
- None.

### 🟡 Important (should fix, does not block)
- None.

### 🔵 Minor (improvement suggestions)
- **`signal_state/src/lib.rs:21`** — `compute_deadline` uses a raw `now + timeout`. On u64 overflow the deadline wraps to a tiny value and would immediately expire. Theoretical only (~584 years at 100 Hz), but `now.saturating_add(timeout)` would be strictly safer with no downside.
- **`kernel/src/syscall.rs:734`** — After waking from `schedule_next()`, `sys_wait_signal` unconditionally returns `-1`; the delivered signal value is not returned here (delivery happens later in `deliver_pending()` before `iretq`). This matches the documented coarse-grained / timer-driven design and is explicitly deferred to Phase 5 (`DECISIONS.md` → [Phase 4]); noted so it isn't mistaken for a regression.
- **Race condition (latent, out of this phase's runtime scope)** — `signal_pending` and `deadline` are plain non-atomic `u64`s accessed from both syscall context (`sys_wait_signal`, `deliver_pending`) and timer-interrupt context (`check_deadlines`). Correctness therefore relies entirely on the kernel's interrupt-disabling discipline around `PROCESS_TABLE`. Single-core educational kernel, and nothing calls `send_signal` yet (runtime dead code until Phase 5), so no cross-context RMW race is currently exercised. Worth making explicit with an atomic/spinlock or a documented "IRQs disabled" invariant in Phase 5.

## Summary
- 🔴 Critical: 0
- 🟡 Important: 0
- 🔵 Minor: 3 (all observations; none block)
- ✅ Plan compliance: 100%
- 📊 Overall quality: 9/10 — clean, well-documented, minimal diff. Pure deadline logic correctly isolated in a host-testable crate; existing signal definitions/handlers and `deliver_pending` left untouched per constraints.

## Final Verdict

✅ **APPROVED** — Phase 4 meets all expected outputs and completion criteria at the logic/compile level:
- `sys_wait_signal` now truly blocks (`state=Blocked` + recorded deadline, yields via scheduler) instead of yield-and-return `-1`.
- `send_signal` sets the pending bit and wakes a blocked target `BLOCKED → READY`; timer `check_deadlines()` (each `timer_tick`) wakes elapsed waiters.
- Timeout/poll returns `-1` without busy-spinning; signal definitions/handler table unchanged.
- Host tests pass (4/4) and bare-metal build is clean — both verified empirically this session.

Full end-to-end wake (block → signal → receive value) remains a Phase 5 concern (stack delivery / page translation); QEMU smoke for HITO-3 stays with the human after Phase 5, as documented in `SUMMARY.md` / `CONTEXT.md`.
