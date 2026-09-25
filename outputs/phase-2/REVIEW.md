# Review — Phase 2: Runtime loading of servers from embedded ELFs

**Date:** 2026-09-25
**Files reviewed:** 6 (`kernel/build.rs`, `kernel/src/server_images.rs`, `kernel/src/process.rs` spawn_from_elf, `kernel/src/main.rs` boot path, `kernel/src/elf.rs::load_elf`, `kernel/src/paging.rs::map_phys_page/create_address_space`) + the 4 produced server ELFs
**Result:** ⚠️ APPROVED WITH OBSERVATIONS

## Plan Compliance

| Expected output | Status | Observation |
|-----------------|--------|-------------|
| Standalone server crates → ELF-64 ET_EXEC @0x400000 | ✅ | All 4 (`vga/keyboard/init/shell`) verified `readelf -h` = ET_EXEC, ELF64, LE, x86-64; code LOAD @ vaddr 0x400000 |
| Embedded ELF byte table in kernel build | ✅ | `build.rs` compiles each server via `userspace/build.sh`, copies to `$OUT_DIR/server_elfs/<name>.elf`, generates `include_bytes!` constants → `server_images::<NAME>_ELF` |
| `process.rs`: new `spawn_from_elf()` helper | ✅ | Implemented (line 472): spawns, `elf::load_elf(cr3)`, maps VGA buffer, sets entry/registers — mirrors `load_elf_into_process` |
| `main.rs`: boot path loads from embedded ELFs via loader | ✅ | All 4 servers spawned by name via `spawn_from_elf(name, ...)`; old inline extern-C entry points removed |

## Static Verification (run this session)

| Check | Command / method | Result |
|-------|------------------|--------|
| Kernel compiles cleanly | `touch kernel/src/main.rs kernel/build.rs && cargo +nightly build --bin kernel` | ✅ exit 0, only pre-existing warnings (no errors) |
| Servers are ET_EXEC @0x400000 | `readelf -h` / `readelf -l` on each `$OUT_DIR/server_elfs/*.elf` | ✅ all 4: Tipo EXEC, LOAD R-E segment @ virtual 0x400000 |
| No dangling refs to deleted dead code | `grep -rn server_entry_point\|cstr_eq --include=*.rs .` | ✅ none (exit 1) |
| VGA buffer lands in each page table | traced `spawn_from_elf` → `map_phys_page(p.cr3, 0x600000, 0xB8000, …)`; lazy alloc at every level | ✅ maps into every spawned process's cr3 |
| ABI consistency (kernel ↔ userspace) | compared `IpcMessage` + `IPC_MSG_SIZE` both sides | ✅ identical, compile-time `[();64]` size check on both |

## Issues Found

### 🔴 Critical (block next phase)
- None.

### 🟡 Important (should fix, does not block)
- **`outputs/phase-2/SUMMARY.md` / Completion Criteria — executed QEMU boot missing.** The criteria require "the kernel builds **and boots in QEMU**", but only the static build was verified this session (`cargo build` → exit 0). No `readelf -l` on the *kernel* image boot, no live run. This is an intermediate phase (`Closes-HITO: no`), so per tracker rules it caps at 🟡. The deferral is documented in Reviewer Notes (no-sudo constraint) and HITO-2's gate remains 🔶 (not green). **Must run a QEMU smoke boot before closing HITO-2 / advancing** — the phase must not be treated as fully complete until live boot evidence exists.

### 🔵 Minor (improvement suggestions)
- **Dead-code duplication (documented intentional deviation).** `kernel/src/user_program.rs` still contains inline `vga_server`/`keyboard_server`/`init`/`shell` modules that duplicate the standalone `userspace/src/bin/*` implementations now actually loaded at runtime. Per DECISIONS.md + Reviewer Notes this is an intentional "fallback per phase spec" and only yields warnings (no `deny(warnings)`). Not a bug — flagged as observation. Worth cleanup in a later phase once the fallback path is truly dead.
- **Pre-existing unused variables.** `elf.rs` reports unused `mem_size` / `page_base`; these belong to the 84 pre-existing warnings, not introduced by this phase. No action required.

## Summary

- 🔴 Critical: 0
- 🟡 Important: 1 (deferred QEMU boot evidence — documented, intermediate phase)
- 🔵 Minor: 2 (intentional dead-code duplication; pre-existing unused vars)
- ✅ Plan compliance: 100%
- 📊 Overall quality: 8/10 — clean build, correct ELF loading, centralized device mapping, consistent ABI. Slight deduction for the still-unexecuted boot path and retained duplicate inline modules (both documented).

## Final Verdict

⚠️ **APPROVED WITH OBSERVATIONS** — Phase-2 deliverables are correct and statically verified (clean build + all 4 servers ET_EXEC @0x400000, no dangling refs, VGA mapping lands in every page table, ABI consistent). The one important observation is that the live QEMU boot remains unverified this session; it must be run before HITO-2's gate is marked green. No corrections block continuation to Phase 3.
