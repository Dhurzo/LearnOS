# CURRENT_PLAN.md

> Live tracker state. Rewritten by the builder when a HITO is REVIEWED.
> Source of truth for `.phase` states; `plan.md` is a derived view.

## Proyecto
LearnOS — Incomplete-Components Plan

## Estado General
🔶 En planificación (6 fases, 4 HITOs)

## Siguiente ID libre de hito: **HITO 1**

## Mapa HITO ↔ Fases

| HITO | Fases | Gate HITO | Estado |
|------|-------|-----------|--------|
| HITO 1 | phase-1 | `cargo build` userspace sample + `readelf -h` shows ET_EXEC @0x400000 | 🔶 |
| HITO 2 | phase-2, phase-3 | QEMU boot: VGA prints and keyboard input work with servers loaded as ELFs through the ISR | 🔶 |
| HITO 3 | phase-4, phase-5 | Signal tests pass / QEMU smoke: a process blocks on `WAIT_SIGNAL` and wakes correctly | 🔶 |
| HITO 4 | phase-6 | Read a block via virtio-blk in QEMU; list/read files from the FAT/ext2 image | 🔶 |

## Fase → Estado (derived)

> Canonical source: `outputs/phase-X/.phase`. This table must mirror it.

| Fase | Estado |
|------|--------|
| phase-1 | reviewed |
| phase-2 | reviewed |
| phase-3 | reviewed |
| phase-4 | reviewed |
| phase-5 | reviewed |
| phase-6 | requires_fix (driver/filesystem bugs fixed 2026-09-25, QEMU gate still pending) |

## Última verificación por repo

| Fase | Comando | Resultado |
|------|---------|-----------|
| phase-2 | `readelf -l userspace/target/.../out/server_elfs/*.elf` + clean kernel build | ET_EXEC, code LOAD @0x400000 for vga/keyboard/init/shell; kernel ELF64 EXEC builds clean (no errors) |
| phase-3 | `./build.sh` + confirm ISR spawned (`INTERRUPT_SERVER_PID` set on spawn) | INTERRUPT_SERVER_ELF embedded ET_EXEC @0x400000; kernel builds clean; forward_irq routes through ISR when PID≠0 (direct-delivery fallback preserved); routing now active by default. Correction (requires_fix): baseline user caps granted at process creation (`Process::new` → `grant_initial_caps`: VGA_SEND/KEYBOARD_SEND/INTERRUPT_SEND/DRIVER_SEND) so generic IPC works — keyboard self-registration to ISR (dst PID2→KEYBOARD_SEND) and ISR→keyboard hop (dst PID3→DRIVER_SEND) now pass `check_ipc_send`; build clean, no new warnings |
| phase-4 | `cargo test -p signal_state --target x86_64-unknown-linux-gnu` + bare-metal kernel build | Signal-wait blocking implemented: `sys_wait_signal` (nr 18) blocks on state=Blocked with per-process `deadline` (kernel ticks), returns signal value on delivery / `-1` on timeout/poll; `send_signal` wakes BLOCKED→READY+pending; timer `check_deadlines()` wakes elapsed waiters. Pure deadline logic isolated in new host-tested crate `signal_state/` (core-only, bare-metal-compatible). Host: 4 passed. Bare-metal: clean build, no errors. Full QEMU wake deferred to Phase 5 (stack delivery) |
| phase-6 | userspace lib+bin + bare-metal kernel build; host unit tests (`--target x86_64-unknown-linux-gnu`) | Storage in user space: kernel virtio-blk 0.95 legacy driver (`block_dev.rs`, `init()` tries virtio→RAM-disk fallback) + `BLOCK_READ`(24)/`BLOCK_WRITE`(25) syscalls → `userspace::block_driver` wrapper; FAT16 parser `userspace::fs` (`Fat16::new`, `list_root`, `find` case-insensitive, `read_file` follows FAT chain); smoke server `fs_server` prints root dir + README via vga_write→serial. Host: 2 passed (parses BPB+reads file; rejects non-FAT). Bare-metal: clean build. Deviation from spec documented in DECISIONS.md: SeaBIOS legacy virtio-blk BAR0 is I/O-space, not mappable by `map_phys_page`, so kernel does raw block transfer + user-space FS semantics (phase-sanctioned fallback). QEMU runtime verification blocked (no QEMU/sudo) — HITO-4 gate code-complete but unverified. **Bugfix 2026-09-25 (code, QEMU gate still pending):** PCI config now `out dx,eax` with spec bit layout; vendor+device halves compared; legacy register map corrected (QUEUE_PFN/SEL/NOTIFY/STATUS + config @0x14, capacity u64); queue rebuilt as 2 contiguous pages with bounce buffer + status byte + idx-change completion wait; `read_bytes` underflow guard; `sys_block_write` clamped + read-modify-write; full-range user validation; FAT `root_dir_sector`/entry offsets fixed + `read_file` clamped + honest fixtures. Host now: kernel 7 passed, userspace 5 passed, signal_state 4 passed; `./build.sh` release PVH clean |
