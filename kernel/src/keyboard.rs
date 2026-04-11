//! Minimal PS/2 keyboard input (polling mode).
//!
//! This module reads scan code set 1 from the legacy i8042 controller using
//! I/O ports:
//! - `0x64`: status register
//! - `0x60`: data register
//!
//! It is intentionally small and supports the keys needed for basic line input.
//!
//! ## Why polling?
//!
//! In a full kernel, keyboard input is usually interrupt-driven (IRQ1 + IDT/PIC).
//! This project uses polling to keep early bring-up simple:
//! - no interrupt descriptor table yet,
//! - no PIC remapping/unmasking logic,
//! - deterministic control flow during initial hardware learning.
//!
//! ## Scan code model
//!
//! This implementation assumes PS/2 scan code set 1 as provided by QEMU's
//! legacy-compatible keyboard path. Only *make* codes (key press) are handled.
//! *Break* codes (key release) are identified by bit 7 set and ignored.
//!
//! ## Safety boundaries
//!
//! Unsafety is fully contained in `inb` where inline assembly performs
//! privileged port I/O. All callers operate through safe Rust APIs.

use core::arch::asm;
use core::hint::spin_loop;

const KBD_DATA_PORT: u16 = 0x60;
const KBD_STATUS_PORT: u16 = 0x64;
/// Status register bit 0: output buffer full (controller has data for CPU).
const STATUS_OUTPUT_FULL: u8 = 1 << 0;

/// Decoded keyboard events used by the kernel line editor.
pub(crate) enum KeyEvent {
    /// Printable ASCII byte.
    Char(u8),
    /// Enter/Return key.
    Enter,
    /// Backspace key.
    Backspace,
}

/// Non-blocking poll for key events. Returns None if no key is available.
pub(crate) fn poll_keyboard() -> Option<KeyEvent> {
    // Check if data is available
    let status = inb(KBD_STATUS_PORT);
    if status & STATUS_OUTPUT_FULL != 0 {
        let scancode = inb(KBD_DATA_PORT);
        // Ignore release events (bit 7 set)
        if scancode & 0x80 != 0 {
            return None;
        }
        return decode_scancode(scancode);
    }
    None
}

/// Block until a supported key press is available.
///
/// Release scan codes are ignored.
/// Unsupported keys are skipped.
///
/// # Behavior
///
/// 1. Wait for controller output buffer to become ready.
/// 2. Read one scan code from data port `0x60`.
/// 3. Ignore release events (`scancode & 0x80 != 0`).
/// 4. Decode supported make codes into a [`KeyEvent`].
/// 5. Repeat until successful decode.
pub(crate) fn read_key_blocking() -> KeyEvent {
    loop {
        let scancode = read_scancode_blocking();

        if scancode & 0x80 != 0 {
            continue;
        }

        if let Some(event) = decode_scancode(scancode) {
            return event;
        }
    }
}

fn read_scancode_blocking() -> u8 {
    // Busy-wait polling loop. `spin_loop` gives the CPU a hint that we are
    // intentionally waiting on hardware state and can reduce contention.
    loop {
        let status = inb(KBD_STATUS_PORT);
        if status & STATUS_OUTPUT_FULL != 0 {
            return inb(KBD_DATA_PORT);
        }
        spin_loop();
    }
}

fn decode_scancode(scancode: u8) -> Option<KeyEvent> {
    // Minimal scan code set 1 table (US layout subset).
    // Shift/CapsLock/AltGr modifiers are intentionally not handled yet.
    let event = match scancode {
        0x02 => KeyEvent::Char(b'1'),
        0x03 => KeyEvent::Char(b'2'),
        0x04 => KeyEvent::Char(b'3'),
        0x05 => KeyEvent::Char(b'4'),
        0x06 => KeyEvent::Char(b'5'),
        0x07 => KeyEvent::Char(b'6'),
        0x08 => KeyEvent::Char(b'7'),
        0x09 => KeyEvent::Char(b'8'),
        0x0A => KeyEvent::Char(b'9'),
        0x0B => KeyEvent::Char(b'0'),
        0x0C => KeyEvent::Char(b'-'),
        0x0D => KeyEvent::Char(b'='),
        0x10 => KeyEvent::Char(b'q'),
        0x11 => KeyEvent::Char(b'w'),
        0x12 => KeyEvent::Char(b'e'),
        0x13 => KeyEvent::Char(b'r'),
        0x14 => KeyEvent::Char(b't'),
        0x15 => KeyEvent::Char(b'y'),
        0x16 => KeyEvent::Char(b'u'),
        0x17 => KeyEvent::Char(b'i'),
        0x18 => KeyEvent::Char(b'o'),
        0x19 => KeyEvent::Char(b'p'),
        0x1A => KeyEvent::Char(b'['),
        0x1B => KeyEvent::Char(b']'),
        0x1C => KeyEvent::Enter,
        0x1E => KeyEvent::Char(b'a'),
        0x1F => KeyEvent::Char(b's'),
        0x20 => KeyEvent::Char(b'd'),
        0x21 => KeyEvent::Char(b'f'),
        0x22 => KeyEvent::Char(b'g'),
        0x23 => KeyEvent::Char(b'h'),
        0x24 => KeyEvent::Char(b'j'),
        0x25 => KeyEvent::Char(b'k'),
        0x26 => KeyEvent::Char(b'l'),
        0x27 => KeyEvent::Char(b';'),
        0x28 => KeyEvent::Char(b'\''),
        0x29 => KeyEvent::Char(b'`'),
        0x2B => KeyEvent::Char(b'\\'),
        0x2C => KeyEvent::Char(b'z'),
        0x2D => KeyEvent::Char(b'x'),
        0x2E => KeyEvent::Char(b'c'),
        0x2F => KeyEvent::Char(b'v'),
        0x30 => KeyEvent::Char(b'b'),
        0x31 => KeyEvent::Char(b'n'),
        0x32 => KeyEvent::Char(b'm'),
        0x33 => KeyEvent::Char(b','),
        0x34 => KeyEvent::Char(b'.'),
        0x35 => KeyEvent::Char(b'/'),
        0x39 => KeyEvent::Char(b' '),
        0x0E => KeyEvent::Backspace,
        _ => return None,
    };

    Some(event)
}

fn inb(port: u16) -> u8 {
    // x86 port-mapped I/O read.
    // Instruction semantics: AL <- IN(DX)
    // - DX selects the I/O port.
    // - AL receives the byte read from hardware.
    //
    // `options` rationale:
    // - `nomem`: no direct memory accesses in this asm block.
    // - `nostack`: does not modify the stack pointer.
    // - `preserves_flags`: EFLAGS/RFLAGS are preserved.
    let value: u8;
    unsafe {
        asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}
