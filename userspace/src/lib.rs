//! userspace — runtime for LearnOS microkernel user programs.
//!
//! This crate is the *runtime* that every user-space program links against:
//! it provides the low-level syscall wrappers (ported verbatim from the
//! kernel), a copy of the `IpcMessage` layout, high-level IPC helpers, the
//! `_start` entry trampoline, and the panic handler.
//!
//! Individual programs do NOT live here — they are separate bin targets
//! (see `examples/`) that define their own `extern "C" fn main() -> !`,
//! which this crate's `_start` calls after setting up the user stack.
//!
//! Programs run at ring 3 and use kernel mechanisms via syscalls/IPC. Their
//! ET_EXEC load base is `0x10000000`, separate from the kernel's supervisor
//! identity map; each process receives a private stack near the canonical
//! user-space limit.

#![cfg_attr(not(test), no_std)]
// LESSON (same pattern as the kernel): `cargo test --target <host>` compiles
// this crate hosted, so the `fs`/`block_driver` parsing logic (pure code, no
// syscalls executed) can be unit-tested without QEMU. `_start` and the panic
// handler stay gated on `not(test)` below — the test harness supplies those.

// User-space storage service components:
//   * `block_driver` : legacy VirtIO port-I/O/DMA protocol, behind a narrow TSS grant.
//   * `fs`           : FAT16 parser (BPB parse, root-dir listing, file read).
pub mod block_driver;
pub mod fs;

/// Raw syscall wrappers.
///
/// Convention (matches the kernel exactly):
///   rax = syscall number
///   rdi = arg1  rsi = arg2  rdx = arg3  r10 = arg4  r8 = arg5
///
/// The kernel only implements `syscall0`..`syscall3`; we port exactly those.
pub mod syscall {
    /// Zero-argument syscall: `rax = nr`. Returns the return value in `rax`.
    #[inline(always)]
    pub unsafe fn syscall0(nr: usize) -> isize {
        let ret: isize;
        core::arch::asm!(
            "syscall",
            in("rax") nr,
            lateout("rax") ret,
            lateout("rcx") _, // SYSCALL consumes RCX as the return RIP.
            lateout("r11") _, // SYSCALL consumes R11 as the saved RFLAGS.
            lateout("rdi") _, lateout("rsi") _, lateout("rdx") _,
            lateout("r8") _, lateout("r9") _,
            options(nostack)
        );
        ret
    }

    /// One-argument syscall: `rax = nr`, `rdi = a1`.
    #[inline(always)]
    pub unsafe fn syscall1(nr: usize, a1: usize) -> isize {
        let ret: isize;
        core::arch::asm!(
            "syscall",
            in("rax") nr,
            inlateout("rdi") a1 => _,
            lateout("rax") ret,
            lateout("rcx") _, lateout("r11") _,
            lateout("rsi") _, lateout("rdx") _, lateout("r8") _, lateout("r9") _,
            options(nostack)
        );
        ret
    }

    /// Two-argument syscall: `rax = nr`, `rdi = a1`, `rsi = a2`.
    #[inline(always)]
    pub unsafe fn syscall2(nr: usize, a1: usize, a2: usize) -> isize {
        let ret: isize;
        core::arch::asm!(
            "syscall",
            in("rax") nr,
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            lateout("rax") ret,
            lateout("rcx") _, lateout("r11") _,
            lateout("rdx") _, lateout("r8") _, lateout("r9") _,
            options(nostack)
        );
        ret
    }

    /// Three-argument syscall: `rax = nr`, `rdi = a1`, `rsi = a2`, `rdx = a3`.
    #[inline(always)]
    pub unsafe fn syscall3(nr: usize, a1: usize, a2: usize, a3: usize) -> isize {
        let ret: isize;
        core::arch::asm!(
            "syscall",
            in("rax") nr,
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            inlateout("rdx") a3 => _,
            lateout("rax") ret,
            lateout("rcx") _, lateout("r11") _,
            lateout("r8") _, lateout("r9") _,
            options(nostack)
        );
        ret
    }
}

/// Syscall numbers.
///
/// Mirrors `kernel/src/syscall.rs::syscall_nr`. Only the numbers whose
/// wrappers we actually use are listed here; keep these in sync with the
/// kernel — a mismatch silently calls the wrong handler.
pub mod syscall_nr {
    pub const EXIT: usize = 0;
    pub const WRITE: usize = 1;
    pub const READ: usize = 2;
    pub const IPC_SEND: usize = 3;
    pub const IPC_RECV: usize = 4;
    pub const BRK: usize = 5;
    pub const GETPID: usize = 6;
    pub const VGA_WRITE: usize = 7;
    pub const VGA_CLEAR: usize = 8;
    pub const SCHEDULE: usize = 9;
    // GET_SERVER_PID (10): return the PID of a well-known server process.
    //   1 = VGA display server, 2 = Keyboard input server.
    // Exposed so user-space servers can discover each other's PIDs without
    // hardcoding them — mirrors kernel/src/syscall.rs::sys_get_server_pid.
    pub const GET_SERVER_PID: usize = 10;
    // GRANT_CAP (19): delegate a held capability bit to another process.
    pub const GRANT_CAP: usize = 19;
    // KILL (20): send a signal to another process.
    pub const KILL: usize = 20;
    // EXEC (21): replace this process's image with the ELF at a user buffer.
    pub const EXEC: usize = 21;
}

/// Permission denied: the caller lacks the capability for the requested
/// endpoint or operation (mirrors the kernel's EACCES).
pub const EACCES: isize = -2;

/// Well-known signal numbers (mirrors `kernel/src/signal.rs::sig_num`).
pub mod sig {
    pub const SIGTERM: usize = 5;
    pub const SIGSTOP: usize = 9;
    pub const SIGCONT: usize = 10;
}

// Re-export the raw syscall wrappers at the crate root for ergonomic use by
// server binaries (e.g. `use userspace::{syscall0, syscall_nr};`).
pub use syscall::{syscall0, syscall1, syscall2};

/// Tiny file-service protocol between the shell and the storage server.
///
/// The storage server owns the FAT parser and the block driver; the shell
/// asks it for directory listings and file chunks over IPC. Fixed 64-byte
/// messages, little-endian integers, 8.3 upper-case names.
pub mod fs_proto {
    /// Shell → storage: list the root directory.
    pub const LIST: u16 = 0x40;
    /// Storage → shell: one directory entry.
    /// `data[0..11]` = raw 8.3 name, `data[11..15]` = size u32 LE,
    /// `data[15..17]` = first cluster u16 LE, `data[17]` = 1 if dir.
    pub const ENTRY: u16 = 0x41;
    /// Storage → shell: end of listing.
    pub const END: u16 = 0x42;
    /// Shell → storage: read file bytes.
    /// `data[0..11]` = raw 8.3 name, `data[12..16]` = offset u32 LE,
    /// `data[16..20]` = length u32 LE (clamped to DATA_MAX).
    pub const READ: u16 = 0x43;
    /// Storage → shell: file bytes.
    /// `data[0..4]` = byte count u32 LE (READ_ERROR on failure),
    /// `data[4..60]` = up to DATA_MAX bytes.
    pub const DATA: u16 = 0x44;
    /// `DATA` count value meaning "no such file / too big".
    pub const READ_ERROR: u32 = 0xFFFF_FFFF;
    /// Largest payload one `DATA` message can carry.
    pub const DATA_MAX: usize = 56;
    /// Largest file the storage server will stage for a `READ` (16 KiB).
    pub const MAX_STAGED: usize = 16 * 1024;
}

/// Shell command-line parsing and number conversion (pure, no syscalls).
///
/// Lives in the runtime so it is unit-testable on the host; the shell binary
/// itself is `no_std` without a test harness.
pub mod cmd {
    /// Maximum arguments per line (including the command).
    pub const MAX_ARGS: usize = 8;
    /// Maximum bytes per argument.
    pub const MAX_ARG_LEN: usize = 32;

    /// Split `line` on ASCII whitespace into `argv`/`lens`.
    ///
    /// Returns the argument count (0 for a blank line). Arguments longer
    /// than `MAX_ARG_LEN` are truncated; extras beyond `MAX_ARGS` are
    /// dropped. Never fails — callers dispatch on `argv[0]` when `argc > 0`.
    pub fn parse_line(line: &[u8], argv: &mut [[u8; MAX_ARG_LEN]; MAX_ARGS], lens: &mut [usize; MAX_ARGS]) -> usize {
        let mut argc = 0;
        let mut i = 0;
        while i < line.len() && argc < MAX_ARGS {
            while i < line.len() && (line[i] == b' ' || line[i] == b'\t' || line[i] == b'\n' || line[i] == b'\r') {
                i += 1;
            }
            if i >= line.len() {
                break;
            }
            let mut len = 0;
            while i < line.len() && line[i] != b' ' && line[i] != b'\t' && line[i] != b'\n' && line[i] != b'\r' {
                if len < MAX_ARG_LEN {
                    argv[argc][len] = line[i];
                    len += 1;
                }
                i += 1;
            }
            lens[argc] = len;
            argc += 1;
        }
        argc
    }

    /// Parse an ASCII decimal number. Returns `None` on empty input,
    /// non-digit bytes, or overflow past `u32`.
    pub fn atoi(s: &[u8]) -> Option<u32> {
        if s.is_empty() {
            return None;
        }
        let mut v: u32 = 0;
        for &b in s.iter() {
            if b < b'0' || b > b'9' {
                return None;
            }
            v = v.checked_mul(10)?.checked_add((b - b'0') as u32)?;
        }
        Some(v)
    }

    /// Convert a command-line file name to raw 8.3 bytes (upper-cased,
    /// space-padded). `"runme"` → `RUNME   ` + `   `; `"a.elf"` → `A` + `ELF`.
    pub fn to_83(name: &[u8]) -> [u8; 11] {
        let mut out = [b' '; 11];
        let mut dot = None;
        for (i, &b) in name.iter().enumerate() {
            if b == b'.' {
                dot = Some(i);
                break;
            }
        }
        let (base, ext) = match dot {
            Some(d) => (&name[..d], &name[d + 1..]),
            None => (name, &[][..]),
        };
        for (i, &b) in base.iter().take(8).enumerate() {
            out[i] = ascii_up(b);
        }
        for (i, &b) in ext.iter().take(3).enumerate() {
            out[8 + i] = ascii_up(b);
        }
        out
    }

    fn ascii_up(b: u8) -> u8 {
        if b >= b'a' && b <= b'z' {
            b - 32
        } else {
            b
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn parse(s: &[u8]) -> ([[u8; MAX_ARG_LEN]; MAX_ARGS], [usize; MAX_ARGS], usize) {
            let mut argv = [[0u8; MAX_ARG_LEN]; MAX_ARGS];
            let mut lens = [0usize; MAX_ARGS];
            let argc = parse_line(s, &mut argv, &mut lens);
            (argv, lens, argc)
        }

        fn arg<'a>(argv: &'a [[u8; MAX_ARG_LEN]; MAX_ARGS], lens: &[usize], i: usize) -> &'a [u8] {
            &argv[i][..lens[i]]
        }

        #[test]
        fn splits_words_and_skips_blanks() {
            let (argv, lens, argc) = parse(b"  cat   README  \n");
            assert_eq!(argc, 2);
            assert_eq!(arg(&argv, &lens, 0), b"cat");
            assert_eq!(arg(&argv, &lens, 1), b"README");
            let (_, _, argc) = parse(b"   \n");
            assert_eq!(argc, 0);
        }

        #[test]
        fn truncates_long_args_and_caps_argc() {
            let (argv, lens, argc) = parse(b"echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
            assert_eq!(argc, 2);
            assert_eq!(lens[1], MAX_ARG_LEN);
            let mut many = [b'a'; 64];
            for i in (0..64).step_by(2) {
                many[i] = b' ';
            }
            let (_, _, argc2) = parse(&many);
            assert!(argc2 <= MAX_ARGS);
        }

        #[test]
        fn atoi_accepts_decimals_rejects_junk() {
            assert_eq!(atoi(b"0"), Some(0));
            assert_eq!(atoi(b"42"), Some(42));
            assert_eq!(atoi(b"007"), Some(7));
            assert_eq!(atoi(b""), None);
            assert_eq!(atoi(b"4x"), None);
            assert_eq!(atoi(b"-1"), None);
            assert_eq!(atoi(b"99999999999"), None);
        }

        #[test]
        fn to_83_pads_and_upcases() {
            assert_eq!(to_83(b"runme"), *b"RUNME      ");
            assert_eq!(to_83(b"a.elf"), *b"A       ELF");
            assert_eq!(to_83(b"README"), *b"README     ");
        }
    }
}

/// Size of a single IPC message in bytes (fixed-size), matching the kernel.
pub const IPC_MSG_SIZE: usize = 64;

/// Initial IPC message carrying storage-device resources from the kernel.
/// data[0..2] is the legacy BAR0 I/O-port base, data[2..10] is the physical
/// address of the driver's two-page DMA queue.
pub const MSG_STORAGE_CONFIG: u16 = 0x30;

/// A single IPC message exchanged between processes via kernel-mediated IPC.
///
/// This is an exact copy of `kernel/src/process.rs::IpcMessage`. The layout
/// MUST match byte-for-byte, because the kernel copies this struct across the
/// sender/receiver address spaces. Any divergence would corrupt IPC payloads.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IpcMessage {
    /// PID of the sending process (filled in by the kernel).
    pub src_pid: u16,
    /// Message type (interpreted by the receiver).
    pub msg_type: u16,
    /// Payload bytes (60 bytes — total struct is 64).
    pub data: [u8; IPC_MSG_SIZE - 4],
}

/// Compile-time guarantee that `IpcMessage` is exactly 64 bytes.
///
/// If the kernel ever changes the layout, this assertion fails to compile in
/// every program that links this runtime — catching drift before it corrupts
/// a message at runtime.
const _IPC_MSG_SIZE_CHECK: [(); 64] = [(); core::mem::size_of::<IpcMessage>()];

impl IpcMessage {
    /// Create a new IPC message with the given source PID and type.
    pub const fn new(src_pid: u16, msg_type: u16, data: [u8; IPC_MSG_SIZE - 4]) -> Self {
        Self { src_pid, msg_type, data }
    }
}

/// Write `count` bytes from `buf` to file descriptor `fd`.
///
/// Mirrors the kernel's high-level `sys_write(fd, buf, count)`: only stdout
/// (1) and stderr (2) are supported. Returns the number of bytes written on
/// success, or a negative value on failure.
#[inline]
pub unsafe fn sys_write(fd: usize, buf: usize, count: usize) -> isize {
    syscall::syscall3(syscall_nr::WRITE, fd, buf, count)
}

/// Send an IPC message to `dst_pid`.
///
/// Kernel signature: `IPC_SEND(dst_pid, msg_ptr)` → `syscall2`. The kernel
/// copies the `IpcMessage` pointed at by `msg_ptr` into its own address space.
#[inline]
pub unsafe fn ipc_send(dst_pid: usize, msg_ptr: usize) -> isize {
    syscall::syscall2(syscall_nr::IPC_SEND, dst_pid, msg_ptr)
}

/// Receive an IPC message into the buffer at `buf_ptr`.
///
/// Kernel signature: `IPC_RECV(buf_ptr)` → `syscall1`. The kernel writes a
/// message (up to 64 bytes) into the caller's buffer and returns its length.
#[inline]
pub unsafe fn ipc_recv(buf_ptr: usize) -> isize {
    syscall::syscall1(syscall_nr::IPC_RECV, buf_ptr)
}

/// Terminate the calling process and halt the system.
///
/// Mirrors the kernel's `sys_exit`, which simply enters an infinite loop (no
/// user-level cleanup in this educational microkernel).
#[inline]
pub unsafe fn sys_exit(_code: usize) -> ! {
    let _ = syscall::syscall0(syscall_nr::EXIT);
    loop {}
}

/// Write a single character to the VGA text-mode display.
///
/// Mirrors the kernel's `sys_vga_write`: the kernel forwards this via IPC to
/// the user-space VGA server (PID 1), which renders it into its own mapped
/// framebuffer at 0x20000000. Returns bytes written on success, or a negative
/// value on failure.
#[inline]
pub unsafe fn vga_write(ch: usize) -> isize {
    syscall::syscall1(syscall_nr::VGA_WRITE, ch)
}

/// Clear the screen via the VGA display server (VGA_CLEAR, nr 8).
#[inline]
pub unsafe fn vga_clear() -> isize {
    syscall::syscall0(syscall_nr::VGA_CLEAR)
}

/// Return the PID of a well-known server process (GET_SERVER_PID, nr 10).
///
///   `server` = 1 → VGA display server, 2 → Keyboard input server.
/// Returns -1 if the lookup fails. Lets servers route IPC by name without
/// hardcoding PIDs.
#[inline]
pub unsafe fn get_server_pid(server: usize) -> isize {
    syscall::syscall1(syscall_nr::GET_SERVER_PID, server)
}

/// Delegate a held capability bit to another process (GRANT_CAP, nr 19).
///
/// Returns 0 on success, -1 on malformed arguments, EACCES (-2) when the
/// caller does not hold the bit itself.
#[inline]
pub unsafe fn grant_cap(dst_pid: usize, cap: usize) -> isize {
    syscall::syscall2(syscall_nr::GRANT_CAP, dst_pid, cap)
}

/// Send a signal to another process (KILL, nr 20).
///
/// Returns 0 on success, -1 on unknown PID or out-of-range signal.
#[inline]
pub unsafe fn kill(pid: usize, sig: usize) -> isize {
    syscall::syscall2(syscall_nr::KILL, pid, sig)
}

/// Replace this process's image with the ELF found at `buf` (`len` bytes).
///
/// On success this never returns: execution restarts at the new entry point
/// with a fresh stack. Returns a negative value on failure (bad image, bad
/// range) and the current image keeps running.
#[inline]
pub unsafe fn exec(buf: usize, len: usize) -> isize {
    syscall::syscall2(syscall_nr::EXEC, buf, len)
}

/// Adjust the process break (heap end) to `addr` (BRK, nr 5).
///
/// `addr == 0` queries the current break without changing it. Returns the
/// (possibly new) break address, or -1 on failure.
#[inline]
pub unsafe fn sys_brk(addr: usize) -> isize {
    syscall::syscall1(syscall_nr::BRK, addr)
}

/// Minimal bump allocator on top of `BRK` (no `free`).
///
/// One 8-aligned bump pointer per process; the kernel maps fresh zeroed
/// pages as the break advances. Enough for a shell that needs a
/// file-sized buffer once per command — a real allocator can replace
/// this module without touching callers.
pub mod heap {
    static mut HEAP_PTR: usize = 0;

    /// Reserve `size` bytes and return their address, or 0 on failure.
    pub unsafe fn alloc(size: usize) -> usize {
        if size == 0 {
            return 0;
        }
        let size = (size + 7) & !7;
        if HEAP_PTR == 0 {
            let cur = super::sys_brk(0);
            if cur <= 0 {
                return 0;
            }
            HEAP_PTR = cur as usize;
        }
        let base = HEAP_PTR;
        let new_end = match base.checked_add(size) {
            Some(v) => v,
            None => return 0,
        };
        let got = super::sys_brk(new_end);
        if got < 0 || (got as usize) < new_end {
            return 0;
        }
        HEAP_PTR = new_end;
        base
    }
}

// The entry point must be assembly, not an ordinary Rust function: Rust inline
// asm is required to restore RSP before returning to compiler-generated code.
// This trampoline starts with the ELF loader's minimal stack, installs this
// process's private high-canonical user stack, and only then calls its `main`.
// Keeping it outside a Rust prologue also makes the SysV call alignment explicit.
#[cfg(not(test))]
core::arch::global_asm!(
    ".globl _start",
    ".type _start, @function",
    "_start:",
    "movabs rsp, {stack_top}",
    "call main",
    "ud2", // Every program entry is declared -> !; returning is a bug.
    stack_top = const 0x0000_7FFF_FFFF_F000u64,
);

/// Panic handler for user-space programs.
///
/// Mirrors the kernel's panic handler: on any panic we disable further work
/// and halt by spinning. A real kernel would report the panic to the caller
/// process, but this educational microkernel halts instead.
///
/// Excluded from host test builds (`cfg(test)`), where std provides its own
/// panic handler; otherwise the two `panic_impl` lang items collide.
#[panic_handler]
#[cfg(not(test))]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
