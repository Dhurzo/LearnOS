# Phase 6 Context — Real block device + filesystem in user space

## Files (Phase 6)
- `kernel/src/block_dev.rs` — virtio-blk 0.95 legacy driver (PCI enum, descriptor ring, raw 512-byte transfer) + RAM disk fallback; `init()` tries virtio then falls back to RAM disk.
- `kernel/src/syscall.rs` — `BLOCK_READ` (nr 24) / `BLOCK_WRITE` (nr 25) handlers dispatch to `block_dev::read_bytes`/`write_block`.
- `userspace/src/block_driver.rs` — thin syscall wrapper: `read_sector`/`write_sector`/`read_bytes`/`write_bytes`/`read_file` over BLOCK_READ/BLOCK_WRITE.
- `userspace/src/fs.rs` — FAT16 parser: `Fat16::new` (BPB), `list_root`, `find` (case-insensitive 8.3 match), `read_file` (FAT chain follow); host unit tests pass.
- `userspace/src/bin/fs_server.rs` — smoke-test server: reads sector 0, parses BPB, lists root dir, reads README; prints via `vga_write`→serial.

## Decisions
- Fallback to kernel-side virtio-blk + BLOCK syscalls instead of MMIO into user space (SeaBIOS legacy 0.95 = BAR0 I/O-space, cannot be mapped by `map_phys_page`). Documented in DECISIONS.md.
- FAT16 chosen over ext2 (simpler minimal implementation).
- `block_driver` + `fs` implemented as internal modules of the `userspace` crate (not separate top-level crates) to satisfy build.sh's outside-tree build constraint.

## Exports
- `block_driver`: `read_sector`, `write_sector`, `read_bytes`, `write_bytes`, `read_file`, `SECTOR_SIZE=512`.
- `fs::Fat16`: `new(&bpb) -> Option<Fat16>`, `list_root(read, &mut [Option<DirEntry>;N]) -> usize`, `find(read, name) -> Option<DirEntry>`, `read_file(read, entry, out) -> usize`.
- `fs::DirEntry`: `is_valid()`, `display_name()`, `raw_name`, `is_dir`, `size`, `first_cluster`.

## Configuration
- Build userspace from outside repo tree via `build.sh` with command-line `--config` (Phase 1 decision).
- Kernel bare-metal: `cargo +nightly build -Z build-std=core,compiler_builtins --target x86_64-unknown-none`.
