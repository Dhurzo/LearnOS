//! VGA text-mode writer utilities.
//!
//! This module centralizes low-level screen output for the early kernel boot phase.
//! It writes bytes directly into the memory-mapped VGA text buffer.

use core::ptr;

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

/// Print text to VGA text buffer.
///
/// Writes bytes from `s` into the first cells of the VGA text buffer.
/// Bytes after the visible screen capacity are ignored.
///
/// # Notes
/// - Uses volatile writes because VGA memory is memory-mapped I/O.
/// - UTF-8 multi-byte sequences are written byte-by-byte and may not render as expected.
/// - This routine intentionally stays minimal for early-boot use.
pub(crate) fn print_vga(s: &str) {
    for (i, &byte) in s.as_bytes().iter().take(VGA_MAX_CHARS).enumerate() {
        let offset = i * VGA_CELL_BYTES;

        unsafe {
            ptr::write_volatile(VGA_BUFFER.add(offset), byte);
            ptr::write_volatile(VGA_BUFFER.add(offset + 1), VGA_COLOR_WHITE);
        }
    }
}
