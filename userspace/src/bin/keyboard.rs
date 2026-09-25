//! PS/2 keyboard input server (Phase 2 — standalone user-space image).
//!
//! This server receives raw scancodes from the kernel's IRQ handler, decodes
//! them into ASCII characters, buffers them in a ring buffer, and serves them to
//! client processes on request.
//!
//! Split-phase design: the kernel IRQ handler pushes scancodes asynchronously
//! (via IPC type 3), while clients pull decoded keys on demand (type 4 → reply
//! type 5). The ring buffer decouples the interrupt rate from the client rate,
//! keeping the kernel minimal — all decode/buffer logic lives in user space.

#![no_std]
#![no_main]

use userspace::{get_server_pid, ipc_recv, ipc_send, vga_write, syscall0, syscall_nr, IpcMessage};

/// Ring buffer size for decoded key events.
const KEY_BUF_CAPACITY: usize = 16;

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
    if isr_pid > 0 {
        let my_pid = unsafe { syscall0(syscall_nr::GETPID) } as u16;
        let mut data = [0u8; 60];
        data[0] = 1; // IRQ number (keyboard line = 1)
        data[1] = my_pid as u8; // our PID, little-endian
        data[2] = (my_pid >> 8) as u8;
        let reg = IpcMessage::new(0, 6, data); // MSG_IRQ_REGISTER
        unsafe { let _ = ipc_send(isr_pid as usize, &reg as *const IpcMessage as usize); }
    }

    loop {
        // Wait for the next IPC message.
        let mut msg: IpcMessage = unsafe { core::mem::zeroed() };
        let ret = unsafe { ipc_recv(&mut msg as *mut IpcMessage as usize) };
        if ret <= 0 {
            // No message — yield CPU cooperatively instead of busy-waiting.
            unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
            continue;
        }

        match msg.msg_type {
            // ── MSG_KEY_SCANCODE (type 3): raw scancode from IRQ handler ──
            // data[0] = the raw PS/2 scancode byte.
            3 => {
                let scancode = msg.data[0];
                // Bit 7 = 0 means "make" (key press). Ignore break codes.
                if scancode & 0x80 == 0 {
                    if let Some(ch) = decode_scancode(scancode) {
                        // Buffer the decoded key (drop if full).
                        let next = (buf_tail + 1) % KEY_BUF_CAPACITY;
                        if next != buf_head {
                            key_buf[buf_tail] = ch;
                            buf_tail = next;
                        }
                        // Echo the key to VGA so the user can see what they typed.
                        // The kernel forwards this via IPC to the VGA server.
                        unsafe { let _ = vga_write(ch as usize); }
                    }
                }
            }
            // ── MSG_KEY_REQUEST (type 4): a client wants a key ──
            // Reply with type 5 (MSG_KEY_EVENT) addressed to the sender.
            4 => {
                if buf_head != buf_tail {
                    // Key available: pop from ring buffer.
                    let ch = key_buf[buf_head];
                    buf_head = (buf_head + 1) % KEY_BUF_CAPACITY;
                    // Reply with key data (data[0] = char, data[1] = 1 = available).
                    let reply = IpcMessage::new(0, 5, {
                        let mut d = [0u8; 60];
                        d[0] = ch;
                        d[1] = 1;
                        d
                    });
                    unsafe { let _ = ipc_send(msg.src_pid as usize, &reply as *const IpcMessage as usize); }
                } else {
                    // No key available: send empty reply (data[1] = 0 = none).
                    let reply = IpcMessage::new(0, 5, {
                        let mut d = [0u8; 60];
                        d[0] = 0;
                        d[1] = 0;
                        d
                    });
                    unsafe { let _ = ipc_send(msg.src_pid as usize, &reply as *const IpcMessage as usize); }
                }
            }
            _ => {}
        }
    }
}
