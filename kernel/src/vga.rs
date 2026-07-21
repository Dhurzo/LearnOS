//! VGA Text Mode Display Driver (Phase 0 — Basic Boot)
//!
//! Writes directly to the VGA text mode framebuffer at physical address
//! `0xB8000` (identity-mapped in the kernel's page tables). Supports
//! 80×25 colour character output using a simple cursor-based writer.
//!
//! The driver is used by:
//!   - the kernel for debug/panic messages during early boot
//!   - user-space processes via the VGA server IPC endpoint (PID 1)

use core::sync::atomic::{AtomicUsize, Ordering};

/// Kernel debug print macro — writes to the VGA text mode buffer.
///
/// Re-exports `core::fmt::Write` and formats arguments through the `VgaWriter`.
#[macro_export]
macro_rules! kprintln {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!(crate::vga::VgaWriter, $($arg)*);
    }};
}

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

/// Physical address of the VGA text mode framebuffer (80 columns × 25 rows).
const VGA_BUFFER: u64 = 0xB8000;

/// Default foreground/background colour byte (light grey on black).
const VGA_COLOR_DEFAULT: u8 = 0x07;

/// Number of character columns in text mode.
const VGA_COLUMNS: usize = 80;

/// Number of character rows in text mode.
const VGA_ROWS: usize = 25;

/// Bytes per cell (character + attribute).
const VGA_CELL_BYTES: usize = 2;

/// Total number of characters the screen can hold at once.
pub const VGA_MAX_CHARS: usize = VGA_COLUMNS * VGA_ROWS;

/// Standard COM1 serial port I/O base address.
const SERIAL_PORT: u16 = 0x3F8;

/// Current cursor position (character index into the framebuffer).
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Clear the entire VGA text screen to blank spaces and reset the cursor to
/// the top-left corner (index 0).
pub fn clear_screen() {
    let vga = VGA_BUFFER as *mut u8;
    for i in (0..VGA_MAX_CHARS * VGA_CELL_BYTES).step_by(VGA_CELL_BYTES) {
        unsafe {
            core::ptr::write_volatile(vga.add(i), b' ');
            core::ptr::write_volatile(vga.add(i + 1), VGA_COLOR_DEFAULT);
        }
    }
    CURSOR.store(0, Ordering::Release);
}

/// Write a single character byte to the VGA framebuffer.
///
/// Handles newline (`\n`), carriage return (`\r`) by advancing or wrapping the
/// cursor; all other bytes are written at the current cursor position with
/// the default colour attribute. Writes are silently dropped if past the end
/// of the 80×25 screen.
pub fn write_byte(byte: u8) {
    match byte {
        b'\n' => {
            let pos = CURSOR.load(Ordering::Acquire);
            let row = pos / VGA_COLUMNS;
            if row < VGA_ROWS - 1 {
                CURSOR.store((row + 1) * VGA_COLUMNS, Ordering::Release);
            } else {
                CURSOR.store(0, Ordering::Release);
            }
        }
        b'\r' => {
            let pos = CURSOR.load(Ordering::Acquire);
            let row = pos / VGA_COLUMNS;
            CURSOR.store(row * VGA_COLUMNS, Ordering::Release);
        }
        _ => {
            let pos = CURSOR.fetch_add(1, Ordering::Acquire);
            if pos < VGA_MAX_CHARS {
                let vga = VGA_BUFFER as *mut u8;
                let offset = pos * VGA_CELL_BYTES;
                unsafe {
                    core::ptr::write_volatile(vga.add(offset), byte);
                    core::ptr::write_volatile(vga.add(offset + 1), VGA_COLOR_DEFAULT);
                }
            }
        }
    }
}

/// Print a UTF-8 string to the VGA framebuffer, one byte at a time.
pub fn print_vga(s: &str) {
    for byte in s.bytes() {
        write_byte(byte);
    }
}

/// Write a single byte to the COM1 serial port via its I/O ports.
///
/// Busy-waits on the Line Status Register (port offset 5) for bit 5
/// (`Transmitter Holding Register Empty`) before writing through data port
/// (offset 0). Used only during early boot when no real serial driver exists.
fn serial_write(byte: u8) {
    unsafe {
        // Wait for transmitter holding register to be empty (bit 5)
        loop {
            let status: u8;
            core::arch::asm!("in al, dx", out("al") status, in("dx") SERIAL_PORT + 5, options(nostack));
            if status & 0x20 != 0 {
                break;
            }
        }
        // Write byte to data port
        core::arch::asm!("out dx, al", in("dx") SERIAL_PORT, in("al") byte, options(nostack));
    }
}

/// Print a UTF-8 string to the serial port via `serial_write`.
fn serial_print(s: &str) {
    for byte in s.bytes() {
        serial_write(byte);
    }
}

fn hex_nibble(v: u8) -> u8 {
    if v < 10 { b'0' + v } else { b'A' + v - 10 }
}

fn write_hex_u64(val: u64) {
    for i in (0..16).rev() {
        serial_write(hex_nibble(((val >> (i * 4)) & 0xF) as u8));
    }
}

/// Print a labelled hex value to the serial port, useful for early-boot debug.
pub fn serial_print_hex(label: &str, val: u64) {
    for &b in label.as_bytes() {
        serial_write(b);
    }
    serial_write(b':');
    serial_write(b' ');
    write_hex_u64(val);
    serial_write(b'\n');
}
