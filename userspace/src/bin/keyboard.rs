//! PS/2 keyboard input server (Phase 2 — standalone user-space image).
//!
//! This server owns the PS/2 controller ports (0x60 data, 0x64 status) through
//! a TSS I/O grant installed by the kernel at spawn. The kernel IRQ handler
//! only forwards the IRQ notification — the scancode itself is drained here,
//! from ring 3, when the notification arrives. No kernel code touches port
//! 0x60 (phase A1 purification).
//!
//! Split-phase design: IRQ notifications arrive asynchronously (via IPC type
//! 3), while clients pull decoded keys on demand (type 4 → reply type 5).
//! The ring buffer decouples the interrupt rate from the client rate.

#![no_std]
#![no_main]

use userspace::{get_server_pid, ipc_recv, ipc_send, msg, vga_write, syscall0, syscall_nr, IpcMessage};

/// PS/2 controller ports owned by this server (granted by the kernel).
const PS2_DATA_PORT: u16 = 0x60;
const PS2_STATUS_PORT: u16 = 0x64;
/// Status bit 0 (output-buffer-full): a scancode byte is waiting at 0x60.
const PS2_OBF: u8 = 0x01;

/// Ring buffer size for decoded key events.
const KEY_BUF_CAPACITY: usize = 16;

/// Read one byte from an I/O port (ring 3 — permitted by our TSS grant).
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value, options(nostack, preserves_flags));
    value
}

/// Drain one scancode from the PS/2 controller.
///
/// Waits (bounded) for the output-buffer-full status bit, then reads the data
/// port. Returns `None` if no byte arrives — the IRQ may have been a stray
/// notification, and a server must never spin forever inside its slice.
fn read_scancode() -> Option<u8> {
    unsafe {
        for _ in 0..10_000u32 {
            if inb(PS2_STATUS_PORT) & PS2_OBF != 0 {
                return Some(inb(PS2_DATA_PORT));
            }
            core::hint::spin_loop();
        }
        None
    }
}

/// Handle one drained scancode byte: buffer make-codes, drop the rest.
///
/// The server does NOT echo: the terminal owner (the shell) echoes what it
/// receives. Echoing here as well would print every keystroke twice.
fn handle_scancode(
    scancode: u8,
    key_buf: &mut [u8; KEY_BUF_CAPACITY],
    buf_head: &mut usize,
    buf_tail: &mut usize,
) {
    // Bit 7 = 0 means "make" (key press). Ignore break codes.
    if scancode & 0x80 != 0 {
        return;
    }
    if let Some(ch) = decode_scancode(scancode) {
        // Buffer the decoded key (drop if full).
        let next = (*buf_tail + 1) % KEY_BUF_CAPACITY;
        if next != *buf_head {
            key_buf[*buf_tail] = ch;
            *buf_tail = next;
        }
    }
}

/// Drain only bytes that are already waiting in the controller (no waiting).
///
/// This is cheap enough to run at the top of the server loop: if the ISR
/// notification was dropped because the keyboard queue was full, the scancode
/// is still sitting in the PS/2 buffer and this recovers it.
fn drain_ready(
    key_buf: &mut [u8; KEY_BUF_CAPACITY],
    buf_head: &mut usize,
    buf_tail: &mut usize,
) {
    unsafe {
        for _ in 0..16 {
            if inb(PS2_STATUS_PORT) & PS2_OBF == 0 {
                break;
            }
            handle_scancode(inb(PS2_DATA_PORT), key_buf, buf_head, buf_tail)
        }
    }
}

/// Drain every byte currently waiting in the controller (bounded).
///
/// A boot-time IRQ can fire before this server registers with the ISR, so a
/// stale byte may sit in the 1-byte buffer with no notification ever coming
/// for it: while OBF stays set the controller raises no further IRQs and the
/// keyboard would look dead. Draining at startup (and in bursts per
/// notification) keeps the buffer empty so each new keystroke interrupts.
fn drain_pending(
    key_buf: &mut [u8; KEY_BUF_CAPACITY],
    buf_head: &mut usize,
    buf_tail: &mut usize,
) {
    for _ in 0..16 {
        match read_scancode() {
            Some(sc) => handle_scancode(sc, key_buf, buf_head, buf_tail),
            None => break,
        }
    }
}

/// Decode a PS/2 scancode (set 1, make code only) into an ASCII character.
///
/// PS/2 scancodes come in two flavours:
///   - Make code (bit 7 = 0): key was pressed
///   - Break code (bit 7 = 1): key was released
///
/// We only decode make codes; break codes are ignored (no repeat handling yet).
/// The scancode-to-character mapping is for a US QWERTY layout without shift.
/// Returns None for scancodes we don't recognise (modifiers, function keys…).
fn decode_scancode(scancode: u8) -> Option<u8> {
    // Row-by-row mapping of PS/2 set 1 make codes → ASCII.
    let ch = match scancode {
        // Number row: `1234567890-=`
        0x02..=0x0D => b"1234567890-="[(scancode - 0x02) as usize],
        // QWERTY row: `qwertyuiop[]`
        0x10..=0x1B => b"qwertyuiop[]"[(scancode - 0x10) as usize],
        // Enter key
        0x1C => return Some(b'\n'),
        // ASDF row: `asdfghjkl;'`
        0x1E..=0x29 => b"asdfghjkl;'"[(scancode - 0x1E) as usize],
        // Backslash
        0x2B => b'\\',
        // ZXCV row: `zxcvbnm,./`
        0x2C..=0x35 => b"zxcvbnm,./"[(scancode - 0x2C) as usize],
        // Space bar
        0x39 => b' ',
        // Backspace
        0x0E => return Some(b'\x08'),
        _ => return None,
    };
    Some(ch)
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    // Ring buffer for decoded key events.
    // head = where to read next, tail = where to write next.
    // Empty: head == tail   Full: (tail + 1) % cap == head
    let mut key_buf: [u8; KEY_BUF_CAPACITY] = [0; KEY_BUF_CAPACITY];
    let mut buf_head: usize = 0;
    let mut buf_tail: usize = 0;

    // Self-register with the interrupt server (ISR). The kernel only enables
    // user-space IRQ routing by setting INTERRUPT_SERVER_PID != 0; when it is 0
    // the kernel delivers scancodes to us directly and there is nothing to
    // register with, so we skip silently. When routing is on, tell the ISR that
    // this process owns IRQ 1 (the keyboard) by sending MSG_IRQ_REGISTER.
    let isr_pid = unsafe { get_server_pid(3) };
    // Boot diagnostic (also proves this server runs): report the ISR pid.
    // A missing line here means the keyboard server never started.
    unsafe {
        let _ = vga_write(b'K' as usize);
        let _ = vga_write(b'D' as usize);
        let _ = vga_write(b' ' as usize);
        if isr_pid > 0 {
            let _ = vga_write((b'0' + (isr_pid as u8).min(9)) as usize);
        } else {
            let _ = vga_write(b'-' as usize);
        }
        let _ = vga_write(b'\n' as usize);
    }
    if isr_pid > 0 {
        let my_pid = unsafe { syscall0(syscall_nr::GETPID) } as u16;
        let mut data = [0u8; 60];
        data[0] = 1; // IRQ number (keyboard line = 1)
        data[1] = my_pid as u8; // our PID, little-endian
        data[2] = (my_pid >> 8) as u8;
        let reg = IpcMessage::new(0, msg::IRQ_REGISTER, data);
        unsafe { let _ = ipc_send(isr_pid as usize, &reg as *const IpcMessage as usize); }
    }
    // Drain any stale boot-time byte (see `drain_pending`) so the first real
    // keystroke raises a fresh IRQ.
    drain_pending(&mut key_buf, &mut buf_head, &mut buf_tail);

    loop {
        // Recover any scancode that is already waiting, even if the ISR
        // notification was dropped because this server's IPC queue was full.
        drain_ready(&mut key_buf, &mut buf_head, &mut buf_tail);

        // Wait for the next IPC message.
        let mut event: IpcMessage = unsafe { core::mem::zeroed() };
        let ret = unsafe { ipc_recv(&mut event as *mut IpcMessage as usize) };
        if ret <= 0 {
            // No message — yield CPU cooperatively instead of busy-waiting.
            unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
            continue;
        }

        match event.msg_type {
            // ── MSG_KEY_SCANCODE: keyboard IRQ notification ──
            // The payload carries no scancode: the byte is still sitting in
            // the controller. Drain port 0x60 ourselves (ring 3, granted),
            // in a burst so a fast typist never overruns the 1-byte buffer.
            msg::KEY_SCANCODE => {
                drain_pending(&mut key_buf, &mut buf_head, &mut buf_tail);
            }
            // ── MSG_KEY_REQUEST: a client wants a key ──
            // Reply with MSG_KEY_EVENT addressed to the sender.
            msg::KEY_REQUEST => {
                if buf_head != buf_tail {
                    // Key available: pop from ring buffer.
                    let ch = key_buf[buf_head];
                    buf_head = (buf_head + 1) % KEY_BUF_CAPACITY;
                    // Reply with key data (data[0] = char, data[1] = 1 = available).
                    let reply = IpcMessage::new(0, msg::KEY_EVENT, {
                        let mut d = [0u8; 60];
                        d[0] = ch;
                        d[1] = 1;
                        d
                    });
                    unsafe { let _ = ipc_send(event.src_pid as usize, &reply as *const IpcMessage as usize); }
                } else {
                    // No key available: send empty reply (data[1] = 0 = none).
                    let reply = IpcMessage::new(0, msg::KEY_EVENT, {
                        let mut d = [0u8; 60];
                        d[0] = 0;
                        d[1] = 0;
                        d
                    });
                    unsafe { let _ = ipc_send(event.src_pid as usize, &reply as *const IpcMessage as usize); }
                }
            }
            _ => {}
        }
    }
}
