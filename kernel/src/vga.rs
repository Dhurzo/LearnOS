//! VGA Text Mode Display Driver

use core::sync::atomic::{AtomicUsize, Ordering};

// Constants
const VGA_BUFFER: u64 = 0xB8000;
const VGA_COLOR_DEFAULT: u8 = 0x07;
const VGA_COLUMNS: usize = 80;
const VGA_ROWS: usize = 25;
const VGA_CELL_BYTES: usize = 2;
pub const VGA_MAX_CHARS: usize = VGA_COLUMNS * VGA_ROWS;

const SERIAL_PORT: u16 = 0x3F8;

static CURSOR: AtomicUsize = AtomicUsize::new(0);

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

pub fn print_vga(s: &str) {
    for byte in s.bytes() {
        write_byte(byte);
    }
}

fn serial_write(byte: u8) {
    unsafe {
        let port_addr = SERIAL_PORT as *const u8;
        loop {
            let status = core::ptr::read_volatile(port_addr.add(5));
            if status & 0x20 != 0 {
                break;
            }
        }
        core::ptr::write_volatile(port_addr as *mut u8, byte);
    }
}

pub fn serial_print(s: &str) {
    for byte in s.bytes() {
        serial_write(byte);
    }
}
