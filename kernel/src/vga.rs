//! VGA text-mode writer utilities.
//!
//! This module is now designed to be a *Service* that listens for IPC messages
//! and performs the actual low-level I/O writes when commanded by the Kernel Core.
//! It no longer assumes direct calls from `main`.

use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

/// VGA text buffer base address.
const VGA_BUFFER: *mut u8 = 0xb8000 as *mut u8;

/// VGA color attribute: white foreground on black background.
const VGA_COLOR_WHITE: u8 = 0x0f;

/// VGA text-mode screen dimensions.
const VGA_COLUMNS: usize = 80;
const VGA_ROWS: usize = 25;
const VGA_CELL_BYTES: usize = 2;
pub const VGA_MAX_CHARS: usize = VGA_COLUMNS * VGA_ROWS;

/// Current cursor position in character cells.
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Clear the full screen and move the cursor to the top-left corner.
pub(crate) fn clear_screen() {
    for i in 0..VGA_MAX_CHARS {
        let offset = i * VGA_CELL_BYTES;
        unsafe {
            ptr::write_volatile(VGA_BUFFER.add(offset), b' ');
            ptr::write_volatile(VGA_BUFFER.add(offset + 1), VGA_COLOR_WHITE);
        }
    }
    CURSOR.store(0, Ordering::Relaxed);
}

/// Write a single byte at the current cursor position.
pub(crate) fn write_byte(byte: u8) {
    match byte {
        b'\n' => newline(),
        _ => write_visible_byte(byte),
    }
}

/// Move cursor one position left and clear that character.
pub(crate) fn backspace() {
    let pos = CURSOR.load(Ordering::Relaxed);
    if pos == 0 {
        return;
    }

    let new_pos = pos - 1;
    CURSOR.store(new_pos, Ordering::Relaxed);
    let offset = new_pos * VGA_CELL_BYTES;

    unsafe {
        ptr::write_volatile(VGA_BUFFER.add(offset), b' ');
        ptr::write_volatile(VGA_BUFFER.add(offset + 1), VGA_COLOR_WHITE);
    }
}

/// Print text to VGA text buffer.
pub(crate) fn print_vga(s: &str) {
    for &byte in s.as_bytes() {
        write_byte(byte);
    }
}

fn write_visible_byte(byte: u8) {
    let pos = CURSOR.load(Ordering::Relaxed);
    if pos >= VGA_MAX_CHARS {
        return;
    }

    let offset = pos * VGA_CELL_BYTES;
    unsafe {
        ptr::write_volatile(VGA_BUFFER.add(offset), byte);
        ptr::write_volatile(VGA_BUFFER.add(offset + 1), VGA_COLOR_WHITE);
    }

    CURSOR.store(pos + 1, Ordering::Relaxed);
}

fn newline() {
    let pos = CURSOR.load(Ordering::Relaxed);
    let row = pos / VGA_COLUMNS;
    let next_row = row + 1;

    if next_row >= VGA_ROWS {
        CURSOR.store(VGA_MAX_CHARS, Ordering::Relaxed);
        return;
    }

    CURSOR.store(next_row * VGA_COLUMNS, Ordering::Relaxed);
}
