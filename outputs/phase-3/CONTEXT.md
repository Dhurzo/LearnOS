# Context — Phase 3 (Activate IRQ routing through user-space interrupt server)

## Files changed / created
- `kernel/src/main.rs` — added ISR spawn + `set_interrupt_server_pid(pid)` at boot (the only new code).
- `kernel/src/process.rs` — **Phase 3 correction:** replaced bare `capabilities: 0` in `Process::new` with a call to new private `grant_initial_caps()` (grants VGA_SEND/KEYBOARD_SEND/INTERRUPT_SEND/DRIVER_SEND via `crate::capability::grant`). Baseline user caps granted at creation so generic `ipc_send` passes `check_ipc_send`. FORK/EXEC intentionally left ungranted.
- `DECISIONS.md` — appended Phase 3 decision entry + Phase 3 correction (Option 1: wire up grant()).
- `CURRENT_PLAN.md` — added phase-3 row to `## Última verificación por repo` (updated with capability-fix verification).

## Files already present (no change needed)
- `interrupt_server/src/main.rs` — dispatcher: handles MSG_IRQ_REGISTER (6), HW_IRQ (4), forwards to handlers via ipc_send.
- `kernel/src/interrupt_server.rs` — `forward_irq` routed branch (isr_pid != 0 → push MSG_HW_IRQ) + direct-delivery fallback; `set_interrupt_server_pid`.
- `userspace/src/bin/keyboard.rs` — self-registers with ISR via MSG_IRQ_REGISTER using get_server_pid(3).
- `kernel/build.rs` — builds interrupt_server.elf and embeds it as INTERRUPT_SERVER_ELF.

## Decisions
- Activate routing by default (spawn ISR + set_interrupt_server_pid) per the decision point / HITO-2 gate.
- Guard with `if let Some(isr_pid)` so a failed spawn leaves PID==0 → keeps direct-delivery fallback.
- Spawn ISR before keyboard so it is up when the keyboard self-registers; use set_ready (round-robin services it).

## Exports / capabilities
- `INTERRUPT_SERVER_ELF` embedded ET_EXEC @ 0x400000 (verified via readelf: code LOAD R E at VMA 0x400000).
- Kernel routes IRQs through ISR only when INTERRUPT_SERVER_PID != 0.

## Configuration / build
- `./build.sh` builds kernel + all userspace servers + ISR; clean release build, no errors (only pre-existing warnings).
- Run in QEMU: `./run.sh --gui --interactive`.
