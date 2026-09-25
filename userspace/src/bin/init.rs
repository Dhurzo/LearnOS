//! Init process (Phase 2 — standalone user-space image).
//!
//! PID 1. In this educational microkernel there is no real init / process
//! reaping; this process simply prints the boot banner and then idles, yielding
//! the CPU on each empty time slice until the system halts.

#![no_std]
#![no_main]

use userspace::{vga_write, syscall0, syscall_nr};

#[no_mangle]
pub extern "C" fn main() -> ! {
    unsafe {
        for b in b"Init\n".iter() {
            let _ = vga_write(*b as usize);
        }
    }
    // Idle forever: yield the CPU on each empty slice instead of spinning.
    loop {
        unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
    }
}
