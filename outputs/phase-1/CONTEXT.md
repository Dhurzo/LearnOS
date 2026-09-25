# Phase 1 — Context

## Files
- `userspace/Cargo.toml` — standalone no_std crate manifest (bin target `hello`).
- `userspace/src/lib.rs` — syscall wrappers (`syscall0..3`), `IpcMessage`, `ipc_send`/`ipc_recv`, `_start` trampoline, panic handler.
- `userspace/src/bin/hello.rs` — example: `sys_write(1,"Hello\n")` then `sys_exit(0)`.
- `userspace/build.sh` — reproducible ET_EXEC build (runs from scratch dir + `--config`).
- `userspace/ABI.md` — user-space ABI contract (load base, entry, registers, IpcMessage).

## Decisions
- Load base `0x400000` for ET_EXEC; avoids kernel (`0x100000`) and user stack (`0xFFF000`).
- Build from outside the repo tree via command-line `--config`; file `.cargo/config` rustflags don't load under `--manifest-path` (verified) and root config leaks the kernel linker script.
- Link flags `-Ttext=0x400000 -no-pie -e _start` (self-contained; no external linker script needed).

## Exports
- `userspace::syscall::{syscall0..3}` — raw wrappers, register convention rax/nr, rdi/rsi/rdx/r10/r8.
- `userspace::sys_write/sys_exit/ipc_send/ipc_recv` — high-level helpers.
- `userspace::IpcMessage { src_pid:u16, msg_type:u16, data:[u8;60] }` (64 bytes, matches kernel).
- `userspace::syscall_nr::*` — syscall numbers (mirror kernel).

## Configuration
- Toolchain pinned nightly via root `rust-toolchain.toml`.
- Root Cargo.toml excludes `userspace` from the workspace.
- Build: `./userspace/build.sh [bin] [release]`; output at `target/x86_64-unknown-none/{debug|release}/deps/<bin>-<hash>`.
