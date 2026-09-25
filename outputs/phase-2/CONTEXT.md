# Phase 2 Context — Runtime loading of servers from embedded ELFs

## Files changed
- `kernel/src/process.rs` — `spawn_from_elf()` now maps the physical VGA text buffer (0xB8000 → 0x600000) into every freshly-spawned process's page table; removed dead `server_entry_point()` + `cstr_eq()`.
- `kernel/src/main.rs` — boot path: dropped per-server `map_phys_page` block for VGA (now centralized in spawn_from_elf); kept `set_vga_server_pid` registration.

## Key decisions
1. **VGA mapping lives in `spawn_from_elf`, not per-server.** Every spawned process gets the device mapping, so all servers share a coherent screen buffer; main.rs only registers the VGA PID for IPC forwarding.
2. **Removed dead inline extern-C registry.** `server_entry_point()` + `cstr_eq()` were never called (only referenced inline server entry points); both deleted — no other references remain.
3. **Inline server modules kept as fallback** per phase spec; they now only produce warnings (unused), not errors (no `deny(warnings)`).

## Exports / symbols
- `process.rs::spawn_from_elf(name, elf_bytes) -> Option<Pid>` — spawns process, loads ELF via `elf::load_elf(cr3)`, maps VGA buffer, sets entry/registers.
- `paging::VGA_BUFFER_VADDR` (0x600000), `paging::VGA_PHYS_ADDR` (0xB8000), `paging::map_phys_page`.

## Configuration / build
- Servers built by `userspace/build.sh` with `-Ttext=0x400000 -no-pie` → ET_EXEC @0x400000.
- Kernel build.rs embeds each server ELF into `kernel/src/server_images.rs` via `include_bytes!`.
- Build: `RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld -C relocation-model=static -C link-arg=-no-pie" cargo +nightly build ... --bin kernel`.

## Verification evidence
- Clean kernel build: Finished, no errors (84 pre-existing warnings).
- `readelf -h` kernel: ELF64 EXEC, entry 0x109900.
- `readelf -l` on each server ELF: ET_EXEC, code LOAD segment @ virtual 0x400000.
- Boot path: all 4 servers (`vga-server`, `keyboard`, `init`, `shell`) load via `spawn_from_elf(name, ...)`; zero inline extern-C refs remain.
