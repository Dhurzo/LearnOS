# Phase 1 — Summary

## TL;DR
Userspace crate now builds as a non-PIE `ET_EXEC` at load base `0x400000`. Root cause of the earlier PIE failure: file `.cargo/config` rustflags don't load under `--manifest-path`, so `-no-pie` never reached `rust-lld`; fixed by building from a scratch dir with command-line `--config`.

Gate evidence (`./userspace/build.sh hello`):
```
$ readelf -h target/x86_64-unknown-none/debug/deps/hello-d1429c9be38eb299
Type: EXEC (Fichero ejecutable)   Entry point: 0x4000a0   (static, no PT_INTERP)
```

## Reviewer Notes
- All completion criteria met: ET_EXEC @ 0x400000; syscall register convention verified against `kernel/src/syscall.rs`/`boot.S`; hello builds and inspected.
- `_start` trampoline confirmed via objdump: `push rax` → `mov rsp,0xFFF000` → calls `main`.
- No MILESTONES.md update (HITO closer — written only after REVIEWED). Evidence left above for the closer to copy verbatim.
