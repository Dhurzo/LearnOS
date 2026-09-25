# Review — Phase 6: Real block device + filesystem in user space

**Date:** 2026-09-25
**Files reviewed:** 9
**Result:** 🔴 REQUIRES CORRECTIONS

Files: `kernel/src/block_dev.rs`, `kernel/src/syscall.rs` (BLOCK handlers nr 24/25), `userspace/src/block_driver.rs`, `userspace/src/fs.rs`, `userspace/src/bin/fs_server.rs`, `userspace/src/lib.rs`, `kernel/src/main.rs` (boot wiring), `kernel/build.rs`, `DECISIONS.md` + `outputs/phase-6/SUMMARY.md` / `CONTEXT.md`.

## Plan Compliance

| Output | Status | Observation |
|--------|--------|-------------|
| virtio-blk user-space driver crate (MMIO-driven block I/O) | ⚠️ Partial / deviation | Delivered as `userspace::block_driver` thin wrapper over `BLOCK_READ`/`BLOCK_WRITE` + in-kernel virtio 0.95 driver. Fallback path is sanctioned by phase directive ("if SeaBIOS only offers a kernel-consumable device…") and documented in `DECISIONS.md` [Phase 6] — acceptable as design. But kernel driver itself cannot work (see Critical PCI/virtio bugs). |
| FAT16/ext2 parser crate exposing open/read/readdir-style calls | ⚠️ Partial | `userspace::fs::Fat16::{new,list_root,find,read_file}` exists with host tests (verified `cargo test --lib → 2 passed`). But BPB geometry + dir-entry offsets are incompatible with real `mkfs.fat` images — only self-consistent with its own test builder (see Critical). |
| Kernel boot wiring that maps device MMIO into user space | ⚠️ Deviation documented | No `map_phys_page` MMIO mapping — justified (BAR0 is I/O-space, not mappable). Replacement wiring `block_dev::init()` is called in `main.rs:307` and `BLOCK_READ`/`BLOCK_WRITE` registered in `syscall.rs:184-185`. `fs_server.elf` is embedded via `kernel/build.rs:20` but never spawned at boot (see Important). |
| QEMU-observable verification (serial output of read block + listed/read file) | ❌ Missing | Explicitly not performed: `SUMMARY.md → Reviewer Notes`: "QEMU verification NOT performed — no QEMU / no sudo". HITO-4 gate requires QEMU read + FAT list/read. Code-complete only. |

HITO: Part-Of HITO 4 | Closes-HITO: yes | Gate: "read a block via virtio-blk + list/read files from the FAT image in QEMU" — **not evidenced, blocks HITO close per Evidence Gate rule.**

Evidence Gate: `SUMMARY.md → TL;DR` contains no `command → literal output` pair (only adjectives "builds clean", "passing"). Verified myself where cheap: `cargo test --lib --target x86_64-unknown-linux-gnu` in `userspace/` → `2 passed` (passes); full `cargo test` without `--lib` fails with `E0152 duplicate panic_impl` in all bins (bins lack `cfg(test)` gating); bare-metal `cargo +nightly build -Z build-std … --bin kernel` → clean (81 warnings, 0 errors). QEMU gate unverifiable here.

## Issues Found

### 🔴 Critical (block next phase / HITO close)

- **`kernel/src/block_dev.rs:95-111` — PCI config write only low byte + wrong address layout, enumeration can never work.** `out dx, al` with `addr as u8` writes only bits 7:0 of the 32-bit PCI address; enable bit (31) is lost. Must be `out dx, eax` with full `u32`. Bit shifts are also wrong vs PCI spec: `bus<<24` (should `<<16`), `slot<<19` (should `<<11`), `reg<<16` (should `<<8` + function bits, masked `&0xFC`). As written every `pci_config_read_reg` reads garbage.
- **`kernel/src/block_dev.rs:309` — vendor/device compare inverted, virtio never matched.** `vid_dev as u16` is the *vendor* ID (0x1AF4 for QEMU), compared against `VIRTIO_BLK_DEVICE_ID 0x1001` (device ID). Should test `(vid_dev>>16) as u16 == 0x1001` (and vendor `==0x1AF4`). Combined with above, `init_virtio_blk` always returns `false` → always RAM-disk fallback, HITO-4 gate unreachable.
- **`kernel/src/block_dev.rs:205-207` — descriptor-ring offsets outside allocated page (wild DMA write).** `RING_SCRATCH=4096` is the page *size*, reused as `desc_off`. `desc_off=4096`, `avail_off=4096+24`, `used_off=…` all lie beyond the single `alloc_frame()` page (bytes 0..4095). Every `copy_nonoverlapping` at `ring_phys+desc_off` corrupts adjacent physical memory (page tables/frames). Must be small in-page offsets (e.g. req 0, descs 64, avail/used after).
- **`kernel/src/block_dev.rs:223` — DMA address is a virtual stack address, not physical.** `data.as_ptr() as u64` for a `&[u8;512]` (typically a kernel stack buffer in `read_bytes:418`) is used as device-visible physical address. Kernel is only identity-mapped for low 4 MB (`paging.rs:684-690`); stack/frame addresses beyond that are not `phys==virt`. Needs bounce buffer inside the known-physical `RING_SCRATCH` page + copy.
- **`kernel/src/block_dev.rs:271-276` — completion poll always succeeds instantly (masks all failures).** Scratch page is zeroed at init; `UsedEntry{id:0}` already equals `0` before `QUEUE_NOTIFY`. First poll iteration returns `true` without device action, so `read_block` returns stale/zero data as "success". Must init used `idx` to non-zero / poll `idx` increment + status byte.
- **`kernel/src/block_dev.rs:226-248` — only 2 descriptors, missing status byte (spec violation).** virtio-blk requires header → data → status (1-byte `VIRTIO_BLK_S_OK`). Code chains only header+data, never checks status. Device may fail silently or write status out-of-bounds.
- **`userspace/src/fs.rs:85-87` — root/data geometry off by one root-dir region, incompatible with real FAT16.** `root_dir_sector = reserved + num_fats*fat_size + root_dir_sectors` should be `reserved + num_fats*fat_size`; `data_start = root + root_dir_sectors`. Current code adds `root_dir_sectors` twice, so `list_root`/`cluster_to_sector` read wrong sectors on any `mkfs.fat` image. Test builder `fs.rs:327-328` uses the *same* wrong formula, so `parses_bpb_and_reads_file` passes by shared bug — false confidence.
- **`userspace/src/fs.rs:172` — `first_cluster` bytes swapped with file-size LSB.** `get(0x1C) | get(0x1A)<<8` mixes size and cluster. Spec: cluster low = `0x1A|0x1B<<8`. Test builder `fs.rs:358` writes cluster at `0x1C` (size field) to match the bug. Real images return wrong cluster (e.g. size 18 + cluster 2 → 0x0212=530).
- **`userspace/src/fs.rs:173-176` — `size` read from `0x1E-0x21` instead of `0x1C-0x1F`, bleeds into next entry.** Test builder `fs.rs:359` writes size at `0x1E` to match. On real images size is garbage (includes next entry's name bytes at 0x20-0x21).
- **`userspace/src/fs.rs:246-257` — `read_file` ignores `out.len()`, panics/corrupts when file > buffer.** `copy = min(SECTOR_SIZE, remaining)` where `remaining = size-written`, then `out[written..written+copy]` without `min(out.len()-written)`. `fs_server.rs:104-106` passes `[0u8;256]`; any README >256 B panics the server (userspace panic = `loop{}`, hang). Must clamp to `out.len()`.
- **`kernel/src/syscall.rs:680-690` — `sys_block_write` destroys rest of sector + ignores intra-block offset.** Builds zeroed `buf`, copies to `buf[..chunk]` (always at block start) instead of `buf[(offset+written)%512 ..]`, without first `read_block`. Any partial/unaligned write zeroes neighbouring bytes at wrong position. Userspace `block_driver.rs:72-76` does correct read-modify-write; kernel must do the same. Data-corruption.
- **`kernel/src/syscall.rs:681` — `sys_block_write` loops over `count` but buffer holds `max_count`, OOB panic when `count>4096`.** `max_count=count.min(4096)`, copy fills only `max_count`, but `while written<count` indexes `kernel_buf[written..]` beyond 4096 → kernel panic (`loop{}`, DoS). Loop over `max_count`.
- **`kernel/src/block_dev.rs:407-412` — `read_bytes` underflows when `offset >= total`, panics.** `end=(offset+len).min(total); actual=end-offset` wraps/panics if `offset>=total`. Must `if offset>=total return 0`.

### 🟡 Important (should fix, does not block alone but blocks HITO in aggregate)

- **`kernel/src/block_dev.rs:129-137` — legacy register map implausible + collision.** `QUEUE_NOTIFY 0x10` == `CONFIG_SECTORS 0x10`; `QUEUE_SELECT 0x20 / QUEUE_NUM 0x30 / DESC_LO 0x50` do not match virtio-pci legacy BAR0 layout (legacy uses QueueAddress PFN at 0x08, Select/Size/Notify at small offsets). Verify against spec; capacity read `in eax,dx` at `base+0x10` collides with notify path.
- **`kernel/src/block_dev.rs:321` — BAR0 masking keeps I/O flag bits.** `(bar0 & 0xFFFF) as u16` includes bits 1:0 (`01` = I/O). Base must be `bar0 & 0xFFFFFFFC`. Current `base+off` ports off by 1-3, touches wrong hardware.
- **`kernel/src/block_dev.rs:263` — `QUEUE_NUM=1` invalid + reprogrammed per I/O.** Queues must be ≥16 / device-reported max, programmed once at init, not every transfer.
- **`kernel/src/block_dev.rs:306` — PCI scan `0..8` misses slots 8..31.** Use `0..32`.
- **`kernel/src/block_dev.rs:328-333` — capacity as single `u32` at wrong offset.** Legacy capacity is 64-bit total sectors; code reads one `u32`. Truncates >2 TB (minor) but offset suspect — confirm vs spec.
- **`kernel/src/block_dev.rs:343` — `phys as *mut u8` assumes identity map of any frame.** `alloc_frame` can return frames >4 MB outside `PD[0..1]` identity window. Writing via phys-as-virt may fault/corrupt. Use mapped accessor or low-memory guarantee.
- **`userspace/src/fs.rs:79` — missing `fat_size==0` / `bps!=512` rejection.** FAT32 images (fat16 field 0) misparsed as FAT16; `bps!=512` breaks fixed `[0u8;512]` + `read_sector` assumption. Reject `fat_size==0`, require `bytes_per_sector==512` (or `SECTOR_SIZE`).
- **`userspace/src/fs.rs:190-208` — LFN (attr `0x0F`) + volume-label (`0x08`) entries listed as files.** Only `is_valid()` checked. Skip `attrs & 0x0F == 0x0F` and optionally `0x08`. Doc line 189 claims labels "reported" — intentional but then `display_name` misleads; document or filter.
- **`userspace/src/fs.rs:246-264` — only one sector per cluster; `spc>1` images read corrupt.** `cluster_to_sector` gives first sector, code reads single `read(sector)` then next cluster. Must loop `for s in 0..sectors_per_cluster`. Test uses `spc=1`, hides bug. Real `mkfs.fat` often `spc=4+`.
- **`userspace/src/fs.rs:242` — loop bound `size/512+2` unbounded on corrupt huge `size`.** Misparsed size (see Critical) → millions of I/O iterations, hang. Cap by device clusters / sane max.
- **`userspace/src/fs.rs:33-40` — `display_name` drops extension + `from_utf8_unchecked` on untrusted disk.** `"HELLO.TXT"` → `"HELLO"` (stops at first space, never inserts `.`). Unchecked on arbitrary disk bytes = UB on non-UTF8. Use checked/`from_utf8` + format `BASE[.EXT]` trimming padding.
- **`userspace/src/fs.rs:211-227` — `find` root-only, fragile for `"README.TXT"`.** `to_83("README")` = `"README    "` won't match on-disk `"README  TXT"`. `fs_server` looks for `"README"`; smoke test fails unless image has extensionless name. Document or try both / list-then-match.
- **`userspace/src/fs.rs:374-383` test helper underflows when `sector*512 > img.len()`.** `img.len()-start` panics instead of returning 0. Guard `if start>=img.len() return 0`. Also tests never use a real `mkfs.fat` image — add golden image + cross-check with spec offsets.
- **`userspace/src/block_driver.rs:23,33` — `sector*SECTOR_SIZE` unchecked overflow.** Huge `sector` wraps offset to small value, reads wrong block. Use `checked_mul` / return false on overflow. Same for `offset+total` in `read_bytes`.
- **`userspace/src/bin/fs_server.rs:30` — `print_num(0)` emits NUL (`vga_write(0)`), not `'0'`.** Should be `b'0'`. All zero sizes/clusters print invisible byte.
- **`kernel/src/main.rs:359-429` — `fs_server` embedded but never spawned.** `build.rs:20` includes `"fs_server"`, but boot spawns only vga/isr/keyboard/init/shell. No observable FAT output at boot without manual spawn — HITO verification not wired.
- **Security: `BLOCK_READ`/`BLOCK_WRITE` have no capability/ownership check.** Any userspace process can read/write entire disk (including other files, FAT, boot sector). At minimum gate `BLOCK_WRITE` behind a capability (cf. `check_ipc_send` pattern) or document as accepted educational risk.
- **`userspace/src/lib.rs:panic` + bins: full `cargo test` fails (`E0152 duplicate panic_impl`).** `#[panic_handler] cfg(not(test))` on lib is correct, but each `bin` is `#![no_std](#![no_main])` with its own panic path? Bins inherit lib panic under host test → collision. Gate tests with `cargo test --lib` only and document; or `cfg_attr` bins.
- **Evidence: `SUMMARY.md TL;DR` lacks `command → literal output`.** Required for `Closes-HITO:yes`. Must include e.g. `cargo test --lib --target … → 2 passed`, `cargo +nightly build … → Finished …`, plus future `QEMU … → root entries + README bytes`.

### 🔵 Minor (improvement suggestions)

- Duplication: `block_dev.rs:46-48` RAM strings duplicated verbatim in `filesystem.rs:56-62`. Single source or comment link.
- Dead code: `inw` (`block_dev.rs:282`) never used; `QUEUE_USED_IDX`, `AvailEntry.offset` semantics unclear vs spec; `outq` name writes `u32` (rename `outl`).
- Perf: `list_root` does 1-2 syscalls per entry (512 entries → ~512-1024 reads). Batch whole root-dir sectors once.
- Truncation: `entries=[None;64]` silently drops roots >64; log count vs total.
- `to_83` multi-dot (`a.b.c`) and trailing-dot edge; `normalize_name` `0x05→0xE5` Kanji edge — document non-goals.
- Coupling: `block_dev` calls `vga::serial_print` directly; consider caller-provided logger for testability.
- `sys_block_read` returns `0` on EOF/error indistinguishably; consider `-1` on error vs `0` on EOF, mirror `read` convention.
- `is_valid_user_ptr` range-only, no `buf+count` wrap check — add `checked_add`.

## Summary

- 🔴 Critical: 12
- 🟡 Important: 17
- 🔵 Minor: 8
- ✅ Plan compliance: ~50% (structures exist, fallback documented, but driver/FAT incompatible with real hardware/images + QEMU gate unrun)
- 📊 Overall quality: 4/10 (good modular split + injectable-`read` testability + honest Reviewer Notes; undone by PCI/virtio + FAT-offset + write-path correctness gaps)

## Final Verdict

🔴 **REQUIRES CORRECTIONS** — Do not close HITO 4. Missing QEMU evidence alone blocks close (`Closes-HITO:yes` + unverified gate → must be red per Evidence Gate rule). Independently, PCI enumeration, virtio ring/DMA/completion, FAT geometry + dir-entry offsets, `read_file` buffer clamp, and `sys_block_write` read-modify-write/offset/count bugs must be fixed and re-tested (host unit tests with a real `mkfs.fat` golden image + `cargo test --lib → 2+ passed` literal, bare-metal build literal, then QEMU boot with `-drive … -device virtio-blk-pci` showing sector 0 + root list + file bytes over serial) before next phase.
