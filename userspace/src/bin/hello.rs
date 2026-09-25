//! Example user program: prints a greeting and exits.
//!
//! This is a standalone bin target. It defines only `main()`; the runtime's
//! `_start` (in the linked `userspace` crate) sets up the stack and calls it.
//! The panic handler also lives in the runtime, so we must NOT define one here
//! (a duplicate would fail to link).

#![no_std]
#![no_main]

/// Program entry point, called by the runtime's `_start`.
#[no_mangle]
pub extern "C" fn main() -> ! {
    unsafe {
        let msg = b"Hello, LearnOS user space!\n";
        // fd 1 = stdout. sys_write returns bytes written (or negative on error).
        let _ = userspace::sys_write(1, msg.as_ptr() as usize, msg.len());
    }
    // Exit halts the system in this educational microkernel.
    unsafe {
        userspace::sys_exit(0);
    }
}
