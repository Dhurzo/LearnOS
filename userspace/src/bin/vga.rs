//! VGA text-mode display server (Phase 2 — standalone user-space image).
//!
//! In a microkernel, even the display driver runs in user space. The kernel
//! maps the physical VGA text buffer (0xB8000) into this process's page tables
//! at virtual address `VGA_BUFFER_VADDR` (0x20000000) before starting us — see
//! `kernel/src/main.rs` (map_phys_page on the vga server's cr3). We then write
//! characters directly to video memory with no per-character syscall overhead.
//!
//! This is the key user-space-driver insight: map device memory into the driver
//! process and access hardware at RAM speed. Only the initial mapping requires a
//! privileged kernel operation (map_phys_page), done once at spawn time.

#![no_std]
#![no_main]

use userspace::{ipc_recv, msg, syscall0, syscall_nr, IpcMessage};

/// Mirror output to COM1 serial so user-space rendering is observable from the
/// host (`-serial mon:stdio`). The kernel initialized COM1 at boot and granted
/// this process the serial port range via its TSS I/O bitmap — the driver owns
/// the actual port I/O.
fn serial_putc(b: u8) {
    unsafe {
        let port: u16 = 0x3F8;
        loop {
            let status: u8;
            core::arch::asm!(
                "in al, dx",
                in("dx") port + 5,
                out("al") status,
                options(nostack, preserves_flags),
            );
            if status & 0x20 != 0 {
                break;
            }
        }
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") b,
            options(nostack, preserves_flags),
        );
    }
}

/// Virtual address of the VGA text buffer (mapped by the loader for this process).
const VGA_BUFFER_VADDR: usize = 0x20000000;
/// Standard VGA text-mode grid.
const VGA_COLS: usize = 80;
const VGA_ROWS: usize = 25;
/// Default cell attribute: light grey on black.
const ATTR_DEFAULT: u8 = 0x07;

#[no_mangle]
pub extern "C" fn main() -> ! {
    // Cursor position in the 80×25 VGA text grid.
    let mut cursor: usize = 0;
    // Pointer to the mapped VGA buffer (set up by the kernel loader).
    let vga = VGA_BUFFER_VADDR as *mut u8;

    loop {
        // Event-driven server: wait for IPC, then yield on empty queue.
        let mut event: IpcMessage = unsafe { core::mem::zeroed() };
        let ret = unsafe { ipc_recv(&mut event as *mut IpcMessage as usize) };
        if ret <= 0 {
            // No message available — yield the CPU cooperatively instead of
            // busy-waiting. The timer interrupt brings us back next slice.
            unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
            continue;
        }

        match event.msg_type {
            // ── MSG_VGA_CHAR: write a single character ──
            // data[0] = the ASCII byte to display.
            msg::VGA_CHAR => {
                let byte = event.data[0];
                serial_putc(byte);
                match byte {
                    b'\n' => {
                        let row = cursor / VGA_COLS;
                        if row < VGA_ROWS - 1 {
                            cursor = (row + 1) * VGA_COLS;
                        } else {
                            cursor = 0; // scroll off the top
                        }
                    }
                    b'\r' => {
                        let row = cursor / VGA_COLS;
                        cursor = row * VGA_COLS;
                    }
                    0x08 => {
                        // Backspace: erase the previous cell and step back.
                        if cursor > 0 {
                            cursor -= 1;
                            let offset = cursor * 2;
                            unsafe {
                                core::ptr::write_volatile(vga.add(offset), b' ');
                                core::ptr::write_volatile(vga.add(offset + 1), ATTR_DEFAULT);
                            }
                        }
                    }
                    _ => {
                        if cursor < VGA_COLS * VGA_ROWS {
                            let offset = cursor * 2;
                            unsafe {
                                core::ptr::write_volatile(vga.add(offset), byte);
                                core::ptr::write_volatile(vga.add(offset + 1), ATTR_DEFAULT);
                            }
                            cursor += 1;
                        }
                    }
                }
            }
            // ── MSG_VGA_CLEAR: clear the entire screen ──
            msg::VGA_CLEAR => {
                let mut i = 0;
                while i < VGA_COLS * VGA_ROWS * 2 {
                    unsafe {
                        core::ptr::write_volatile(vga.add(i), b' ');
                        core::ptr::write_volatile(vga.add(i + 1), ATTR_DEFAULT);
                    }
                    i += 2;
                }
                cursor = 0;
            }
            _ => {}
        }
    }
}
