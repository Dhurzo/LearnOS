# Userspace ABI

Contract between a standalone `userspace` ELF and the LearnOS microkernel.
Later phases must stay consistent with this file — it is the single source of
truth for how user programs are loaded, entered, and talk to the kernel.

## Image format

| Property        | Value                                   |
|-----------------|-----------------------------------------|
| ELF class       | ELF-64, little-endian                   |
| `e_type`        | `ET_EXEC` (2) — **not** PIE/`DYN`        |
| Machine         | x86-64                                  |
| Load base (VMA) | `0x400000`                              |
| Entry symbol    | `_start`                                |
| Linking         | static; no dynamic linker, no PT_INTERP |

The kernel's `load_elf` (`kernel/src/elf.rs`) validates and maps exactly this
shape. The load base was chosen to avoid the kernel (physical `0x100000`) and
the user stack top (`USER_STACK_VADDR = 0xFFF000`). Confirm it is free against
`kernel/src/paging.rs` before changing it.

## Entry trampoline

The loader jumps to `_start` with user page tables active but a minimal stack.
The runtime's `_start` (in `src/lib.rs`) sets the user stack top and calls the
program's `main`:

```rust
#[no_mangle]
pub extern "C" fn _start() -> ! {
    core::arch::asm!("mov rsp, 0xFFF000", options(nostack)); // USER_STACK_VADDR
    main(); // never returns
}
```

Each program is a separate bin target defining its own:

```rust
#[no_mangle] pub extern "C" fn main() -> ! { … }
```

## Syscall convention (x86-64 `syscall` instruction)

| Register | Role                              |
|----------|-----------------------------------|
| `rax`    | syscall number                    |
| `rdi`    | arg1                              |
| `rsi`    | arg2                              |
| `rdx`    | arg3                              |
| `r10`    | arg4                              |
| `r8`     | arg5                              |

This matches the kernel's SYSCALL entry path (`kernel/src/syscall.rs`, lines 32,
104, 793) and `boot.S`. The runtime wraps this in `syscall0..syscall3`; only
those four are implemented. Return value comes back in `rax` as an `isize`.

## Syscall numbers (`src/lib.rs::syscall_nr`)

| Name        | nr | Wrapper            |
|-------------|----|--------------------|
| `EXIT`      | 0  | `sys_exit`         |
| `WRITE`     | 1  | `sys_write(fd, buf, count)` |
| `READ`      | 2  | —                  |
| `IPC_SEND`  | 3  | `ipc_send(dst_pid, msg_ptr)` |
| `IPC_RECV`  | 4  | `ipc_recv(buf_ptr)` |
| `BRK`       | 5  | —                  |
| `GETPID`    | 6  | —                  |
| `VGA_WRITE` | 7  | —                  |
| `SCHEDULE`  | 9  | —                  |

Keep these in sync with the kernel; a mismatch silently calls the wrong handler.

## IPC message layout (`src/lib.rs::IpcMessage`)

Exact byte-for-byte copy of `kernel/src/process.rs::IpcMessage`. The kernel
copies this struct across sender/receiver address spaces, so any divergence
corrupts payloads. A compile-time assertion in `lib.rs` fails the build if it
ever drifts from 64 bytes.

```rust
#[repr(C)]
pub struct IpcMessage {
    pub src_pid: u16,          // filled in by the kernel
    pub msg_type: u16,         // interpreted by the receiver
    pub data: [u8; 60],        // payload; total struct = 64 bytes
}
```

High-level helpers: `ipc_send(dst_pid, msg_ptr)` → `IPC_SEND` (`syscall2`);
`ipc_recv(buf_ptr)` → `IPC_RECV` (`syscall1`).

## Build producing ET_EXEC @ 0x400000

Run `./build.sh [bin] [release]`. It invokes cargo from a scratch directory so
the repo-root `.cargo/config.toml` (which injects the kernel linker script)
cannot leak in, and passes the required link args via `--config`:

```
-Ttext=0x400000  -no-pie  -e _start
```

Verify:

```bash
readelf -h target/x86_64-unknown-none/debug/deps/<bin>-<hash>
# → Type: EXEC, Entry point: 0x4000a0 (inside the 0x400000 load region)
```
