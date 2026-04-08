//! VGA text-mode writer utilities.
//!
//! This module centralizes low-level screen output for the early kernel boot phase.
//! It writes bytes directly into the memory-mapped VGA text buffer.
//!
//! ## Hardware model
//!
//! VGA text mode exposes an 80x25 character grid backed by memory at `0xB8000`.
//! Each cell occupies two bytes:
//! - byte 0: character code
//! - byte 1: color attribute
//!
//! This module keeps a linear software cursor and renders sequentially.
//! It does not program the hardware CRT cursor register yet.
//!
//! ## Design constraints
//!
//! - `no_std`: no heap, no formatting machinery
//! - early boot safety: minimal API, predictable behavior
//! - single writer assumption: suitable for this single-core educational kernel

use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

/// VGA text buffer base address.
///
/// Physical memory address: `0xB8000`.
/// In text mode, each screen cell uses two bytes:
/// - Byte 0: ASCII character
/// - Byte 1: color attribute
const VGA_BUFFER: *mut u8 = 0xb8000 as *mut u8;

/// VGA color attribute: white foreground on black background.
const VGA_COLOR_WHITE: u8 = 0x0f;

/// VGA text-mode screen dimensions.
const VGA_COLUMNS: usize = 80;
const VGA_ROWS: usize = 25;
const VGA_CELL_BYTES: usize = 2;
const VGA_MAX_CHARS: usize = VGA_COLUMNS * VGA_ROWS;

/// Current cursor position in character cells.
///
/// This is a linear index in `[0, VGA_MAX_CHARS)`.
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Clear the full screen and move the cursor to the top-left corner.
///
/// The routine writes space characters with the default color to all cells.
/// It uses volatile writes because VGA text memory is memory-mapped I/O.
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
///
/// Supports `\n` (newline) and printable bytes.
///
/// # Notes
///
/// - This function does not interpret UTF-8; callers should pass displayable bytes.
/// - Non-printable control bytes (except `\n`) are written as-is.
pub(crate) fn write_byte(byte: u8) {
    match byte {
        b'\n' => newline(),
        _ => write_visible_byte(byte),
    }
}

/// Move cursor one position left and clear that character.
///
/// If the cursor is already at the first cell, this function is a no-op.
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
///
/// Writes bytes from `s` starting at the current cursor.
/// Bytes beyond visible screen capacity are ignored.
///
/// # Notes
/// - Uses volatile writes because VGA memory is memory-mapped I/O.
/// - UTF-8 multi-byte sequences are written byte-by-byte and may not render as expected.
/// - This routine intentionally stays minimal for early-boot use.
pub(crate) fn print_vga(s: &str) {
    for &byte in s.as_bytes() {
        write_byte(byte);
    }
}

fn write_visible_byte(byte: u8) {
    // Read/modify/write cursor state using relaxed ordering because this kernel
    // currently runs single-core without concurrent VGA writers.
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
    // Move cursor to the first column of the next row.
    // If we are already on the last row, clamp to end-of-screen and drop future
    // characters until higher-level logic clears or scrolls the screen.
    let pos = CURSOR.load(Ordering::Relaxed);
    let row = pos / VGA_COLUMNS;
    let next_row = row + 1;

    if next_row >= VGA_ROWS {
        CURSOR.store(VGA_MAX_CHARS, Ordering::Relaxed);
        return;
    }

    CURSOR.store(next_row * VGA_COLUMNS, Ordering::Relaxed);
}
