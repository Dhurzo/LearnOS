//! VGA Text Mode Driver (Service)
//!
//! This module implements the **VGA Display Service** for the microkernel.
//! In a microkernel architecture, this service would typically run in user space
//! or as a highly privileged kernel module, handling all graphical output requests
//! sent via IPC.
//!
//! ## Hardware Model
//!
//! The driver targets the standard VGA text mode (mode 3), which provides:
//! - **Resolution**: 80 columns x 25 rows
//! - **Buffer**: Memory-mapped I/O at physical address `0xB8000`
//! - **Encoding**: 1 byte per character + 1 byte for color attribute per cell.
//!
//! ## Memory Layout
//!
//! The buffer is a linear array of "cells". Each cell is 2 bytes:
//! - `offset + 0`: ASCII character code
//! - `offset + 1`: Color attribute (Background | Foreground)
//!
//! ## Service Interface
//!
//! This module provides the following public functions, typically called by the
//! kernel dispatcher in response to IPC messages:
//!
//! - `clear_screen()`: Fills the screen with spaces.
//! - `write_byte(byte)`: Writes a single character (handles `\n`).
//! - `backspace()`: Moves cursor back and clears the cell.
//! - `print_vga(str)`: Writes a string.
//!
//! ## Safety
//!
//! This driver accesses hardware via volatile memory writes. The use of
//! `core::ptr::write_volatile` ensures the compiler does not optimize away
//! or reorder these operations.
//!
//! ## Cursor Management
//!
//! The driver maintains a software cursor (`CURSOR`). It does not program
//! the hardware CRT controller (port 0x3D4/0x3D5) for cursor position.
//! This simplifies the implementation but limits functionality (e.g., no
//! scroll handling when the screen is full).

use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

/// The base physical address of the VGA text buffer.
const VGA_BUFFER: *mut u8 = 0xb8000 as *mut u8;

/// Default color attribute: Light grey text on black background.
const VGA_COLOR_DEFAULT: u8 = 0x07;

/// Bright white text on black background (used for clear screen).
const VGA_COLOR_WHITE: u8 = 0x0f;

/// Number of text columns.
const VGA_COLUMNS: usize = 80;

/// Number of text rows.
const VGA_ROWS: usize = 25;

/// Size of one text cell (character + attribute).
const VGA_CELL_BYTES: usize = 2;

/// Total number of character cells on screen.
pub const VGA_MAX_CHARS: usize = VGA_COLUMNS * VGA_ROWS;

/// Global software cursor position.
///
/// Tracks the linear index (0 to MAX_CHARS) of the next character to be written.
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Clears the entire screen.
///
/// Writes a space character to every cell and resets the cursor to the top-left (0).
///
/// # Implementation
///
/// Uses `write_volatile` to ensure the writes are not optimized away.
/// This is critical for memory-mapped I/O.
pub(crate) fn clear_screen() {
    for i in 0..VGA_MAX_CHARS {
        let offset = i * VGA_CELL_BYTES;
        unsafe {
            // Write character
            ptr::write_volatile(VGA_BUFFER.add(offset), b' ');
            // Write attribute (White on Black)
            ptr::write_volatile(VGA_BUFFER.add(offset + 1), VGA_COLOR_WHITE);
        }
    }
    CURSOR.store(0, Ordering::Relaxed);
}

/// Writes a single byte to the screen at the current cursor position.
///
/// Handles special control characters:
/// - `\n` (newline): Advances cursor to the next line.
///
/// # Arguments
///
/// * `byte`: The ASCII byte to write.
///
/// # Note
///
/// Does not handle tab (`\t`) or carriage return (`\r`) explicitly;
/// they are treated as regular characters or ignored.
pub(crate) fn write_byte(byte: u8) {
    match byte {
        b'\n' => newline(),
        _ => write_visible_byte(byte),
    }
}

/// Moves the cursor one position to the left and clears that cell.
///
/// If the cursor is already at position 0, this is a no-op.
pub(crate) fn backspace() {
    let pos = CURSOR.load(Ordering::Relaxed);
    if pos == 0 {
        return;
    }

    let new_pos = pos - 1;
    CURSOR.store(new_pos, Ordering::Relaxed);

    // Calculate offset and clear the cell
    let offset = new_pos * VGA_CELL_BYTES;
    unsafe {
        ptr::write_volatile(VGA_BUFFER.add(offset), b' ');
        ptr::write_volatile(VGA_BUFFER.add(offset + 1), VGA_COLOR_DEFAULT);
    }
}

/// Writes a string to the screen.
///
/// Iterates through the string bytes and calls `write_byte` for each.
/// This is a convenience function for the service handler.
pub(crate) fn print_vga(s: &str) {
    for &byte in s.as_bytes() {
        write_byte(byte);
    }
}

/// Internal: Writes a visible (non-newline) character byte to the screen.
///
/// Checks bounds before writing. If the cursor is at the end of the screen,
/// the write is ignored (or could be extended to implement scrolling).
fn write_visible_byte(byte: u8) {
    let pos = CURSOR.load(Ordering::Relaxed);
    if pos >= VGA_MAX_CHARS {
        return;
    }

    let offset = pos * VGA_CELL_BYTES;

    unsafe {
        // Write character
        ptr::write_volatile(VGA_BUFFER.add(offset), byte);
        // Write color attribute
        ptr::write_volatile(VGA_BUFFER.add(offset + 1), VGA_COLOR_DEFAULT);
    }

    // Advance cursor
    CURSOR.store(pos + 1, Ordering::Relaxed);
}

/// Internal: Handles the newline character.
///
/// Moves the cursor to the first column of the next row.
/// If the cursor is on the last row, it clamps to the end of the screen
/// (simple "no-scroll" behavior).
fn newline() {
    let pos = CURSOR.load(Ordering::Relaxed);
    let current_row = pos / VGA_COLUMNS;
    let next_row = current_row + 1;

    if next_row >= VGA_ROWS {
        // Screen full - clamp to end
        CURSOR.store(VGA_MAX_CHARS, Ordering::Relaxed);
        return;
    }

    // Move to start of next row
    CURSOR.store(next_row * VGA_COLUMNS, Ordering::Relaxed);
}
