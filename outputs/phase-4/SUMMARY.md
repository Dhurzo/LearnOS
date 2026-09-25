# Phase 4 Summary — Signals, real blocking & wakeup

## TL;DR
Implemented signal-wait blocking in `sys_wait_signal` (nr 18): process now blocks on state=Blocked with a per-process `deadline`, woken by `send_signal` or the timer. Pure deadline logic extracted to host-tested crate `signal_state/`. Bare-metal build clean; 4 host tests pass.

## What was done
- **Blocking** (`syscall.rs::sys_wait_signal`): replaced single-yield-and-return with real block — records `deadline = current_tick() + timeout`, sets state=Blocked, yields via scheduler. Returns delivered signal value on wakeup; `-1` on timeout/poll (timeout==0 returns immediately).
- **State/flag**: added `Process.deadline: u64` (kernel ticks) + monotonic clock (`KERNEL_TICK` atomic, `current_tick()`/`increment_tick()`). Reused existing `ProcessState::Blocked`; non-zero deadline disambiguates signal-wait from I/O-block.
- **Wakeup by signal** (`signal.rs::send_signal`): sets pending bit and moves `state==Blocked → READY` so scheduler runs it immediately on delivery.
- **Wakeup by timeout** (`process.rs::check_deadlines`): called each `timer_tick`; flips elapsed-waiters back to READY before `schedule_next`.
- **Host tests**: new workspace crate `signal_state/` (core-only, bare-metal-compatible) with 4 unit tests covering deadline math + expiry predicate.

## Verification
- Host: `cargo test -p signal_state --target x86_64-unknown-linux-gnu` → **4 passed**.
- Bare-metal: `cargo +nightly build -Z build-std=core,compiler_builtins --target x86_64-unknown-none --release --bin kernel` → **clean** (no errors; 11.5MB ELF).

## Reviewer Notes
- Completion criteria verified at the logic/compile level: blocking via state=Blocked ✓, timeout/poll returns -1 without busy-spin ✓, send_signal wakes to READY + sets pending ✓, signal defs/handlers unchanged ✓.
- **Not yet runtime-tested**: full end-to-end wake (process blocks → another signals → receives value) requires Phase 5's stack delivery; this phase covers block/wakeup logic only. QEMU smoke is the HITO-3 gate and remains to be run after Phase 5.
- No new warnings introduced beyond pre-existing baseline (~84).
