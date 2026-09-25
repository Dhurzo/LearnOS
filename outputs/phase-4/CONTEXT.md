# Context — Phase 4: Signals, real blocking & wakeup

**State:** in_progress → completed (implementation + host tests). HITO-3 intermediate phase (`Closes-HITO: no`). Full end-to-end wake deferred to Phase 5 (stack delivery).

## Files changed / created
- `signal_state/` — NEW workspace crate; pure, core-only signal-wait deadline logic + unit tests. Host-testable AND bare-metal-compatible.
- `signal_state/Cargo.toml` — package decl, no deps.
- `signal_state/src/lib.rs` — `compute_deadline(now, timeout) -> u64`, `deadline_expired(deadline, now) -> bool`; 4 host unit tests.
- `Cargo.toml` — added `signal_state` to workspace members.
- `kernel/Cargo.toml` — added `[dependencies] signal_state = { path = "../signal_state" }`.
- `kernel/src/process.rs` — new `deadline: u64` field on `Process`; `KERNEL_TICK` atomic clock + `current_tick()`/`increment_tick()`; `deadline_expired(p, now)` wrapper (delegates math to `signal_state`) and `check_deadlines()` (timer wakeup across table).
- `kernel/src/syscall.rs` — `sys_wait_signal` (nr 18) rewritten: real block via state=Blocked + recorded deadline, returns signal value on delivery / `-1` on timeout/poll.
- `kernel/src/signal.rs` — `send_signal`: sets pending bit + wakes `state==Blocked → READY` (existing wake path; no new variant).

## Key decisions
- Reused `ProcessState::Blocked` + per-process `deadline: u64` instead of a new state variant (keeps signal/state definitions unchanged; non-zero deadline disambiguates signal-wait from I/O-block).
- Pure deadline math isolated in `signal_state/` crate so it is host-tested without dragging in the no_std kernel bin's generated `server_images` / paging / tss / vga modules.
- Wakeup is coarse-grained & timer-driven: kernel returns from wait via `sysretq`; `check_deadlines()` (called each `timer_tick`) flips elapsed-waiters back to READY so round-robin runs them; `send_signal` wakes immediately on delivery.

## Exports / API
- `signal_state::compute_deadline(now: u64, timeout: u64) -> u64` — absolute deadline in kernel ticks.
- `signal_state::deadline_expired(deadline: u64, now: u64) -> bool` — true iff `deadline != 0 && deadline <= now`.
- `process::current_tick()`, `process::increment_tick()` — monotonic kernel clock (PIT-driven).
- `process::check_deadlines()` — wake all elapsed signal-waiters (called from `timer_tick`).

## Configuration / build
- Host tests: `cargo test -p signal_state --target x86_64-unknown-linux-gnu` (overrides bare-metal default target set in `.cargo/config.toml`).
- Bare-metal: `RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld -C relocation-model=static -C link-arg=-no-pie" cargo +nightly build -Z build-std=core,compiler_builtins --target x86_64-unknown-none --release --bin kernel` → clean.

## Verification
- `cargo test -p signal_state --target x86_64-unknown-linux-gnu` → 4 passed.
- Bare-metal kernel build → no errors (11.5MB ELF produced).
- Runtime/QEMU smoke: NOT yet run — full wake requires Phase 5 stack delivery; block/wakeup logic verified by compile + unit tests only.
