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
    pub const SCHEDULE: usize = 9;
    // GET_SERVER_PID (10): return the PID of a well-known server process.
    //   1 = VGA display server, 2 = Keyboard input server.
    // Exposed so user-space servers can discover each other's PIDs without
    // hardcoding them — mirrors kernel/src/syscall.rs::sys_get_server_pid.
    pub const GET_SERVER_PID: usize = 10;
}

// Re-export the raw syscall wrappers at the crate root for ergonomic use by
// server binaries (e.g. `use userspace::{syscall0, syscall_nr};`).
pub use syscall::{syscall0, syscall1, syscall2};

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

/// Return the PID of a well-known server process (GET_SERVER_PID, nr 10).
///
///   `server` = 1 → VGA display server, 2 → Keyboard input server.
/// Returns -1 if the lookup fails. Lets servers route IPC by name without
/// hardcoding PIDs.
#[inline]
pub unsafe fn get_server_pid(server: usize) -> isize {
    syscall::syscall1(syscall_nr::GET_SERVER_PID, server)
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
