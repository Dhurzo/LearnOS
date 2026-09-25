//! Interactive shell (Phase 2 — standalone user-space image).
//!
//! Reads keystrokes from the keyboard server via IPC, echoes them to the VGA
//! server, and processes line input. This is a minimal echo command interpreter:
//! each completed line is echoed back prefixed with "Echo:".

#![no_std]
#![no_main]

use userspace::{get_server_pid, ipc_recv, ipc_send, vga_write, syscall0, syscall_nr, IpcMessage};

const LINE_BUF_SIZE: usize = 128;

fn print_char(ch: u8) {
    unsafe { let _ = vga_write(ch as usize); }
}

fn print_str(s: &str) {
    for &b in s.as_bytes() {
        print_char(b);
    }
}

/// Ask the keyboard server for the next buffered key.
///
/// Protocol:
///   1. Send MSG_KEY_REQUEST (type 4) to the keyboard server.
///   2. Receive MSG_KEY_EVENT (type 5) reply.
///   3. If `reply.data[1] == 1`, `reply.data[0]` is the ASCII character.
///
/// Returns `None` if no key is available or the request failed.
fn get_key(kbd_pid: usize) -> Option<u8> {
    // Send a key request to the keyboard server.
    let req = IpcMessage::new(0, 4, [0u8; 60]); // MSG_KEY_REQUEST
    if unsafe { ipc_send(kbd_pid, &req as *const IpcMessage as usize) } < 0 {
        return None;
    }
    // Receive the reply.
    let mut reply: IpcMessage = unsafe { core::mem::zeroed() };
    let ret = unsafe { ipc_recv(&mut reply as *mut IpcMessage as usize) };
    if ret > 0 && reply.msg_type == 5 && reply.data[1] == 1 {
        Some(reply.data[0])
    } else {
        None
    }
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    // Discover the keyboard server PID via GET_SERVER_PID (server 2).
    let kbd_pid = unsafe { get_server_pid(2) };
    if kbd_pid <= 0 {
        // Keyboard server not available — halt.
        loop {}
    }
    let kbd_pid = kbd_pid as usize;

    // Line buffer for the current input line.
    let mut line_buf: [u8; LINE_BUF_SIZE] = [0; LINE_BUF_SIZE];
    let mut line_len: usize = 0;

    // Display initial prompt.
    print_str("Shell> ");

    loop {
        if let Some(ch) = get_key(kbd_pid) {
            match ch {
                b'\n' | b'\r' => {
                    // Enter: echo newline and process the command.
                    print_char(b'\n');
                    // For now, just echo the input back.
                    if line_len > 0 {
                        print_str("Echo: ");
                        for i in 0..line_len {
                            print_char(line_buf[i]);
                        }
                        print_char(b'\n');
                    }
                    line_len = 0;
                    print_str("Shell> ");
                }
                0x08 => {
                    // Backspace: remove last character if any.
                    if line_len > 0 {
                        line_len -= 1;
                        print_char(0x08); // VGA cursor back
                    }
                }
                _ => {
                    // Regular character: append to line buffer and echo.
                    if line_len < LINE_BUF_SIZE - 1 {
                        line_buf[line_len] = ch;
                        line_len += 1;
                    }
                    print_char(ch);
                }
            }
        } else {
            // No key available — yield CPU cooperatively.
            unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
        }
    }
}
