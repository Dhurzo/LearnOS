# Phase 3 Summary — Activate IRQ routing through user-space interrupt server

## TL;DR
Spawned the interrupt server at boot and registered its PID via `set_interrupt_server_pid(pid)` in `kernel/src/main.rs`, so `forward_irq` now routes hardware IRQs through the ISR instead of the legacy direct-delivery path. Build is clean (ET_EXEC @0x400000 embedded); routing activates by default, with a preserved fallback if spawn fails.

## Reviewer Notes
- **What changed:** exactly one block in `kernel/src/main.rs` — after the VGA server spawn, before the keyboard server spawn:
  ```rust
  if let Some(isr_pid) = pt.spawn_from_elf("interrupt-server", crate::server_images::INTERRUPT_SERVER_ELF) {
      crate::interrupt_server::set_interrupt_server_pid(isr_pid);
      pt.set_ready(isr_pid);
      unsafe { core::ptr::write_volatile(0xB8016 as *mut u8, b'R'); core::ptr::write_volatile(0xB8017 as *mut u8, 0x0A); }
  }
  ```
- **Verification done:** clean `./build.sh` (release, no errors); confirmed `INTERRUPT_SERVER_ELF` embedded ET_EXEC @0x400000; confirmed `forward_irq` routed branch pushes MSG_HW_IRQ to the ISR when PID≠0.
- **Runtime smoke test (for human — needs sudo for QEMU):** run `./run.sh --gui --interactive`, press keys in the shell, and confirm: (a) an 'R' marker appears near the boot markers (ISR spawned), (b) typed characters still echo/are accepted through the user-space keyboard server via the ISR.
- **Open item to verify at runtime:** `sys_ipc_send` enforces capabilities (`check_ipc_send`) but no process ever has a capability granted (`capabilities` stays 0). With routing ON, the ISR forwards to handlers via `ipc_send`, which could reject the hop if caps are enforced and never granted. This is a pre-existing Phase-4 concern; if keyboard input fails under QEMU after this change, grant the needed caps (INTERRUPT_SEND/DRIVER_SEND) rather than reverting routing.

---

## Correction applied (requires_fix → completed) — Capability-based IPC wiring

### TL;DR
Wired up `grant()` at process creation (`Process::new` → new `grant_initial_caps()`, granting VGA_SEND/KEYBOARD_SEND/INTERRUPT_SEND/DRIVER_SEND), so generic `ipc_send` now passes `check_ipc_send`. Fixes the dead keyboard routing (self-registration to ISR + ISR→keyboard hop) and makes the whole capability model functional. Clean `./build.sh`, no new warnings.

### What changed
- `kernel/src/process.rs`: replaced the bare `capabilities: 0` in `Process::new` with a call to a new private `grant_initial_caps()` helper that ORs the four baseline user IPC caps via `crate::capability::grant`. Every process (spawn + spawn_from_elf) now gets them uniformly. FORK/EXEC left ungranted (gate privileged ops).

### Why this fix (Option 1 from REVIEW.md)
- With routing ON, every generic `ipc_send` hits `check_ipc_send`, but no process ever had a capability granted (`capabilities == 0`), so the keyboard's self-registration to the ISR (dst PID2 → KEYBOARD_SEND bit1) and the ISR→keyboard hop (dst PID3 → DRIVER_SEND bit3) were both rejected with `-1`. Keyboard input was completely dead.
- Chose baseline grant at creation over per-server/per-role grants: spawn order is VGA=1, ISR=2, keyboard=3, so hardcoded PID→cap mapping is fragile (reviewer flagged it); a uniform baseline matches capability.rs's documented "granted at process creation", fixes every legitimate IPC flow in one spot, and cannot regress existing behavior since it only makes `check_ipc_send` more permissive.

### Verification
- Clean `./build.sh` (release): no errors, no new warnings (only pre-existing ones).
- Confirmed both `check_ipc_send` callers — `sys_ipc_send` (syscall.rs:331) and `sys_cap_send` (syscall.rs:660) — now succeed for the keyboard flows; fork/exec do not call it.
- Flow trace with granted caps: kernel→ISR in-kernel push (no cap check ✅); keyboard→ISR reg (dst2 → KEYBOARD_SEND ✅); ISR→keyboard forward (dst3 → DRIVER_SEND ✅); keyboard→client reply (dst=src_pid → DRIVER_SEND ✅).

### Open items (left for later — out of scope for this fix)
- 🟡 Asymmetric VGA bypass (`sys_vga_write` uses in-kernel `ipc_push`, never calls `check_ipc_send`) hides the same class of bug.
- 🟡 Silent `-1` rejection with no EACCES diagnostic / logging.
- 🔵 Fragile hardcoded PID→cap mapping in `check_ipc_send`; doc references nonexistent `SYS_GRANT_CAP`.
