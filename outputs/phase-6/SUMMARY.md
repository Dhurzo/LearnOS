# Phase 6 Summary — Real block device + filesystem in user space

## TL;DR
Storage stack now runs in user space: kernel virtio-blk (0.95 legacy) driver + `BLOCK_READ`/`BLOCK_WRITE` syscalls (nr 24/25), wrapped by `userspace::block_driver`; FAT16 parser in `userspace::fs` with passing host unit tests; smoke-test server `fs_server`. Kernel builds clean, userspace lib+bin build clean.

## Reviewer Notes
- **QEMU verification NOT performed** — no QEMU / no sudo in this environment (per DECISIONS.md). The HITO-4 gate ("read a block via virtio-blk in QEMU; list/read files from the FAT image") is code-complete but unverified at runtime. Evidence available: clean bare-metal kernel build + userspace build, and host unit tests proving `fs::Fat16` lists a root directory and reads a file from an in-memory FAT16 image (the parser's sector-read path is identical to what virtio_read_block serves).
- **Real bug fixed this session**: test-image builder wrote the data area at the same offset as the root dir (`data_start == rd_sector`), so file bytes clobbered the directory. Fixed `data_start = rd_sector + root_dir_sectors`. Also confirmed `find()` case-insensitive 8.3 matching works (lowercase lookup matches uppercase FAT names).
- **Deliberate design deviation from phase spec**: original MMIO-into-user-space path is impossible for SeaBIOS legacy virtio-blk (BAR0 is an I/O region, not mappable via `map_phys_page`). Fallback to kernel-side raw block transfer + user-space filesystem semantics is the path the phase's own directive sanctions; documented in DECISIONS.md.
- **To close HITO 4**: boot QEMU with a FAT drive (`-drive file=fat.img,...,if=none` + `-device virtio-blk-pci`) and confirm `fs_server` prints root entries + README over serial.
