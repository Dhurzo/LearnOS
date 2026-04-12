//! PS/2 Keyboard Driver (Polling Mode)
//!
//! This module implements a minimal driver for the legacy PS/2 keyboard controller.
//! It operates in **polling mode**, meaning it actively checks the hardware for input
//! rather than relying on interrupts.
//!
//! ## Hardware Interface
//!
//! The driver communicates with the i8042 PS/2 controller via x86 port-mapped I/O:
//! - **Status Register (Read)**: Port `0x64`
//! - **Data Register (Read/Write)**: Port `0x60`
//!
//! ## Microkernel Context
//!
//! In a monolithic kernel, this code would run in the kernel address space.
//! In this microkernel implementation, this module acts as the **Keyboard Service**.
//! The `poll_keyboard()` function is called by the kernel dispatcher to retrieve
//! input events which are then converted into `KeyEvent` messages for the system.
//!
//! ## Design Decisions
//!
//! 1.  **Polling vs Interrupts**:
//!     Using interrupts requires setting up the Programmable Interrupt Controller (PIC)
//!     and an Interrupt Descriptor Table (IDT). For this minimal kernel, polling keeps
//!     the boot process simple and deterministic.
//!
//! 2.  **Scan Code Set 1**:
//!     We assume the standard PS/2 scan code set 1 (used by most modern emulators like QEMU).
//!     Only "make" codes (key press) are handled. "Break" codes (key release, indicated by
//!     bit 7 in the scancode) are ignored.
//!
//! 3.  **US Layout Subset**:
//!     Only a subset of US keys are mapped to ASCII. Features like Shift, CapsLock, and
//!     AltGr are not implemented yet.
//!
//! ## Safety
//!
//! This module contains unsafe code:
//! - `inb`: Inline assembly for reading from I/O ports.
//! All public interfaces (`poll_keyboard`, `read_key_blocking`) are safe, encapsulating this unsafety.

use core::arch::asm;
use core::hint::spin_loop;

/// I/O port for the PS/2 controller data register.
const KBD_DATA_PORT: u16 = 0x60;

/// I/O port for the PS/2 controller status register.
const KBD_STATUS_PORT: u16 = 0x64;

/// Bit 0 of the Status Register: Output Buffer Full.
/// If set, the controller has data ready for the CPU to read.
const STATUS_OUTPUT_FULL: u8 = 1 << 0;

/// High-level key events returned by the driver.
///
/// These events are processed by the kernel dispatcher and potentially
/// converted into IPC messages.
#[derive(Clone, Copy)]
pub enum KeyEvent {
    /// A printable character byte.
    Char(u8),
    /// The Enter (Return) key.
    Enter,
    /// The Backspace (Delete) key.
    Backspace,
}

/// Polls the keyboard for input without blocking.
///
/// This function checks the controller status register. If data is available,
/// it reads and decodes the scan code. If no data is available, it returns immediately.
///
/// # Returns
///
/// - `Some(KeyEvent)` if a key was pressed and decoded.
/// - `None` if no data was available or the event was ignored (e.g., key release).
pub(crate) fn poll_keyboard() -> Option<KeyEvent> {
    // 1. Check if the output buffer contains data
    let status = inb(KBD_STATUS_PORT);
    if status & STATUS_OUTPUT_FULL != 0 {
        // 2. Read the scan code from the data port
        let scancode = inb(KBD_DATA_PORT);

        // 3. Ignore "break" codes (key release)
        // In scan code set 1, the high bit (0x80) is set for key release.
        if scancode & 0x80 != 0 {
            return None;
        }

        // 4. Decode the make code into a KeyEvent
        return decode_scancode(scancode);
    }

    // No data available
    None
}

/// Blocking read of a single key press.
///
/// This function waits in a busy loop until a supported key is pressed.
/// It is retained for backwards compatibility and simple debugging scenarios.
///
/// # Note
///
/// This function should not be used in the main microkernel loop as it blocks
/// the dispatcher. Use `poll_keyboard()` instead.
#[allow(dead_code)]
pub(crate) fn read_key_blocking() -> KeyEvent {
    loop {
        // Attempt to get a key non-blocking
        if let Some(event) = poll_keyboard() {
            return event;
        }

        // If no key was pressed, give the CPU a hint to wait (saves power/storms)
        spin_loop();
    }
}

/// Internal: Reads a scan code in a blocking manner.
///
/// Waits until the controller output buffer is full, then reads the data.
#[allow(dead_code)]
fn read_scancode_blocking() -> u8 {
    loop {
        let status = inb(KBD_STATUS_PORT);
        if status & STATUS_OUTPUT_FULL != 0 {
            return inb(KBD_DATA_PORT);
        }
        spin_loop();
    }
}

/// Decodes a PS/2 scan code (Set 1) into a high-level `KeyEvent`.
///
/// # Arguments
///
/// * `scancode` - The raw 8-bit scan code received from the keyboard controller.
///
/// # Returns
///
/// `Some(KeyEvent)` if the scancode corresponds to a supported key, or `None` otherwise.
fn decode_scancode(scancode: u8) -> Option<KeyEvent> {
    // Mapping table for US QWERTY layout (subset)
    let event = match scancode {
        // Numbers
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

        // Punctuation (Top row)
        0x0C => KeyEvent::Char(b'-'),
        0x0D => KeyEvent::Char(b'='),

        // Row 1 (QWERTY...)
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

        // Enter
        0x1C => KeyEvent::Enter,

        // Row 2 (ASDF...)
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

        // Backslash
        0x2B => KeyEvent::Char(b'\\'),

        // Row 3 (ZXCV...)
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

        // Space
        0x39 => KeyEvent::Char(b' '),

        // Backspace
        0x0E => KeyEvent::Backspace,

        // Unknown/Unsupported
        _ => return None,
    };

    Some(event)
}

/// Reads a byte from the specified x86 I/O port.
///
/// This is a low-level wrapper around the `IN` instruction.
///
/// # Arguments
///
/// * `port` - The 16-bit I/O port address to read from.
///
/// # Returns
///
/// The 8-bit value read from the port.
///
/// # Safety
///
/// This function executes privileged I/O instructions. It should only be called
/// from contexts where it is safe to do so (like kernel code).
fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!(
            "in al, dx",   // Input: AL = IN(DX)
            in("dx") port, // Port number in DX
            out("al") value, // Result in AL
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}
