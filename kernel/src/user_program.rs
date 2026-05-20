//! User programs
//!
//! These are the first user-space processes in the microkernel. They run at
//! ring 3 (unprivileged) and communicate via IPC through the kernel.
//!
//! In a monolithic kernel, hardware access (display, keyboard, storage) is
//! handled inside the kernel. In our microkernel, each device has its own
//! user-space server process. If the VGA server crashes, the kernel and
//! other processes keep running — that's the key reliability advantage.
//!
//! Currently these servers are compiled into the kernel binary (linked as
//! extern "C" functions). A mature system would load them separately as
//! ELF executables from disk.

pub mod vga_server {
    //! User-space VGA display driver.
    //!
    //! In a microkernel, even the display driver can run in user space.
    //! The VGA server has the VGA text buffer (0xB8000) mapped into its
    //! address space via map_phys_page(). It can write characters directly
    //! to video memory — no syscall overhead per character.
    //!
    //! This is the key insight about user-space drivers: you map device
    //! memory into the driver process, and the driver accesses hardware
    //! at the speed of a memory store. Only the initial mapping setup
    //! requires a privileged operation (the kernel's map_phys_page).

    use crate::paging::VGA_BUFFER_VADDR;
    use crate::process::IpcMessage;
    use crate::syscall::{syscall0, syscall2, syscall_nr};

    // Standard VGA text mode: 80 columns × 25 rows
    const VGA_COLS: usize = 80;
    const VGA_ROWS: usize = 25;

    /// User-space VGA display server main loop.
    ///
    /// Architecture:
    ///   1. The kernel maps 0xB8000 → VGA_BUFFER_VADDR (0x600000) with
    ///      USER_ACCESS | CACHE_DISABLE flags before starting us.
    ///   2. We loop on sys_ipc_recv, waiting for other processes to send
    ///      us messages asking us to print characters.
    ///   3. For each character, we write directly to the mapped VGA buffer.
    ///      Since the mapping is user-accessible, this works from ring 3.
    ///
    /// Why CACHE_DISABLE? The VGA buffer is device memory, not RAM.
    /// CPU caches would delay writes from appearing on screen, and stale
    /// cached reads could show wrong data. The PCD (Page Cache Disable)
    /// bit in the PTE tells the CPU to bypass caches for this page.
    ///
    /// Why SCHEDULE on empty IPC?
    ///   Instead of busy-waiting (spinning the CPU doing nothing), we
    ///   cooperatively yield to the next process. The timer interrupt
    ///   will bring us back when our next time slice starts.
    #[no_mangle]
    pub extern "C" fn vga_main() -> ! {
        // Cursor position in the 80×25 VGA text grid
        let mut cursor: usize = 0;
        // Pointer to the VGA buffer virtual address (mapped by kernel)
        let vga = VGA_BUFFER_VADDR as *mut u8;

        loop {
            // Wait for IPC — this is an event-driven server.
            // If no message is available, we yield the CPU (SCHEDULE)
            // instead of busy-waiting. The timer will bring us back.
            let mut msg: IpcMessage = unsafe { core::mem::zeroed() };
            let ret = unsafe {
                syscall2(
                    syscall_nr::IPC_RECV,
                    &mut msg as *mut IpcMessage as usize,
                    0,
                )
            };
            if ret <= 0 {
                // No message — yield CPU cooperatively instead of busy-waiting
                unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
                continue;
            }

            // Dispatch by message type
            match msg.msg_type {
                // ── MSG_VGA_PRINT (type 1): Write a single character ──
                // data[0] = the ASCII byte to display
                1 => {
                    let byte = msg.data[0];
                    match byte {
                        // Newline: advance to the next row
                        b'\n' => {
                            let row = cursor / VGA_COLS;
                            if row < VGA_ROWS - 1 {
                                cursor = (row + 1) * VGA_COLS;
                            } else {
                                cursor = 0; // Scroll off top
                            }
                        }
                        // Carriage return: back to start of current row
                        b'\r' => {
                            let row = cursor / VGA_COLS;
                            cursor = row * VGA_COLS;
                        }
                        // Regular character: write to VGA buffer
                        _ => {
                            if cursor < VGA_COLS * VGA_ROWS {
                                // VGA text mode: each cell is 2 bytes
                                // [byte 0]:  ASCII character code
                                // [byte 1]:  Attribute (foreground|background colour)
                                // 0x07 = light grey on black
                                let offset = cursor * 2;
                                unsafe {
                                    core::ptr::write_volatile(vga.add(offset), byte);
                                    core::ptr::write_volatile(vga.add(offset + 1), 0x07);
                                }
                                cursor += 1;
                            }
                        }
                    }
                }
                // ── MSG_VGA_CLEAR (type 2): Clear the entire screen ──
                2 => {
                    // Fill every cell with a space and default attribute
                    for i in (0..VGA_COLS * VGA_ROWS * 2).step_by(2) {
                        unsafe {
                            core::ptr::write_volatile(vga.add(i), b' ');
                            core::ptr::write_volatile(vga.add(i + 1), 0x07);
                        }
                    }
                    cursor = 0;
                }
                _ => {}
            }
        }
    }
}

pub mod keyboard_server {
    //! User-space PS/2 keyboard driver.
    //!
    //! This server receives raw scancodes from the kernel's IRQ handler,
    //! decodes them into ASCII characters, stores them in a ring buffer,
    //! and serves them to client processes on request.
    //!
    //! The key design decision: the kernel IRQ handler reads the scancode
    //! from port 0x60 (must be fast — PS/2 only buffers 1 byte) and
    //! immediately forwards it to us via IPC. The decode and buffering
    //! happens in user space, keeping the kernel minimal.

    use crate::process::IpcMessage;
    use crate::syscall::{syscall0, syscall1, syscall2, syscall_nr};

    // Ring buffer size for decoded key events
    const KEY_BUF_CAPACITY: usize = 16;

    /// Decode a PS/2 scancode (set 1, make code only) into an ASCII character.
    ///
    /// PS/2 scancodes come in two flavours:
    ///   - Make code (bit 7 = 0): key was pressed
    ///   - Break code (bit 7 = 1): key was released
    ///
    /// We only decode make codes. Break codes are ignored (no repeat
    /// handling yet). The scancode-to-character mapping here is for a
    /// US QWERTY layout without shift modifiers.
    ///
    /// Returns None for scancodes we don't recognise (modifier keys,
    /// function keys, etc.).
    fn decode_scancode(scancode: u8) -> Option<u8> {
        // Row-by-row mapping of PS/2 set 1 make codes → ASCII.
        // These are the scan codes for the top row (number keys),
        // QWERTY row, ASDF row, and ZXCV row, plus a few specials.
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

    /// Keyboard server main loop.
    ///
    /// Protocol:
    ///   - Type 3 (MSG_KEY_SCANCODE): Incoming from kernel IRQ handler.
    ///     Contains raw scancode. We decode and buffer it.
    ///   - Type 4 (MSG_KEY_REQUEST): A client (e.g. shell) wants a key.
    ///     We reply with type 5 (MSG_KEY_EVENT) containing the key data.
    ///
    /// This is a split-phase design: the IRQ handler pushes scancodes
    /// asynchronously, and clients pull decoded keys on demand. The
    /// ring buffer decouples the interrupt rate from the client rate.
    #[no_mangle]
    pub extern "C" fn keyboard_main() -> ! {
        // Ring buffer for decoded key events
        // head = where to read next, tail = where to write next
        // Empty: head == tail   Full: (tail + 1) % cap == head
        let mut key_buf: [u8; KEY_BUF_CAPACITY] = [0; KEY_BUF_CAPACITY];
        let mut buf_head: usize = 0;
        let mut buf_tail: usize = 0;

        loop {
            // Wait for the next IPC message
            let mut msg: IpcMessage = unsafe { core::mem::zeroed() };
            let ret = unsafe {
                syscall2(
                    syscall_nr::IPC_RECV,
                    &mut msg as *mut IpcMessage as usize,
                    0,
                )
            };
            if ret <= 0 {
                // No message — yield CPU cooperatively instead of busy-waiting
                unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
                continue;
            }

            match msg.msg_type {
                // ── MSG_KEY_SCANCODE (type 3): Raw scancode from IRQ handler ──
                // data[0] = the raw PS/2 scancode byte
                3 => {
                    let scancode = msg.data[0];
                    // Bit 7 = 0 means "make" (key press). We ignore break codes.
                    if scancode & 0x80 == 0 {
                        // Try to decode the scancode to an ASCII character
                        if let Some(ch) = decode_scancode(scancode) {
                            // Buffer the decoded key (drop if full)
                            let next = (buf_tail + 1) % KEY_BUF_CAPACITY;
                            if next != buf_head {
                                key_buf[buf_tail] = ch;
                                buf_tail = next;
                            }
                            // Echo the key to VGA so the user can see what they typed
                            // This syscall goes through the kernel and gets forwarded
                            // via IPC to the VGA server.
                            unsafe {
                                let _ = syscall1(syscall_nr::VGA_WRITE, ch as usize);
                            }
                        }
                    }
                }
                // ── MSG_KEY_REQUEST (type 4): Client requests a key ──
                // We reply with MSG_KEY_EVENT (type 5) addressed to the sender.
                4 => {
                    if buf_head != buf_tail {
                        // Key available: pop from ring buffer
                        let ch = key_buf[buf_head];
                        buf_head = (buf_head + 1) % KEY_BUF_CAPACITY;
                        // Reply with key data (data[0] = char, data[1] = 1 = available)
                        let reply = IpcMessage::new(0, 5, {
                            let mut d = [0u8; 60];
                            d[0] = ch;
                            d[1] = 1;
                            d
                        });
                        unsafe {
                            let _ = syscall2(
                                syscall_nr::IPC_SEND,
                                msg.src_pid as usize,
                                &reply as *const IpcMessage as usize,
                            );
                        }
                    } else {
                        // No key available: send empty reply (data[1] = 0 = none)
                        let reply = IpcMessage::new(0, 5, {
                            let mut d = [0u8; 60];
                            d[0] = 0;
                            d[1] = 0;
                            d
                        });
                        unsafe {
                            let _ = syscall2(
                                syscall_nr::IPC_SEND,
                                msg.src_pid as usize,
                                &reply as *const IpcMessage as usize,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

pub mod init {
    use crate::syscall::{syscall1, syscall_nr};

    #[no_mangle]
    pub extern "C" fn init_main() {
        unsafe {
            let _ = syscall1(syscall_nr::VGA_WRITE, b'I' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'n' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'i' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b't' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'\n' as usize);
        }
        loop {}
    }
}

pub mod shell {
    //! Interactive shell that reads keystrokes from the keyboard server
    //! via IPC, echoes them to the VGA server, and processes line input.

    use crate::process::IpcMessage;
    use crate::syscall::{syscall0, syscall1, syscall2, syscall_nr};

    const LINE_BUF_SIZE: usize = 128;

    fn print_char(ch: u8) {
        unsafe { let _ = syscall1(syscall_nr::VGA_WRITE, ch as usize); }
    }

    fn print_str(s: &str) {
        for &b in s.as_bytes() {
            print_char(b);
        }
    }

    /// Ask the keyboard server for the next buffered key.
    ///
    /// Protocol:
    ///   1. Send MSG_KEY_REQUEST (type 4) to keyboard server (PID via GET_SERVER_PID)
    ///   2. Receive MSG_KEY_EVENT (type 5) reply
    ///   3. If `reply.data[1] == 1`, `reply.data[0]` is the ASCII character
    ///
    /// Returns `None` if no key is available or the request failed.
    fn get_key(kbd_pid: u16) -> Option<u8> {
        // Send a key request to the keyboard server
        let req = IpcMessage::new(0, 4, [0u8; 60]); // MSG_KEY_REQUEST
        let ret = unsafe {
            syscall2(
                syscall_nr::IPC_SEND,
                kbd_pid as usize,
                &req as *const IpcMessage as usize,
            )
        };
        if ret < 0 {
            return None;
        }
        // Receive the reply
        let mut reply: IpcMessage = unsafe { core::mem::zeroed() };
        let ret = unsafe {
            syscall2(
                syscall_nr::IPC_RECV,
                &mut reply as *mut IpcMessage as usize,
                0,
            )
        };
        if ret > 0 && reply.msg_type == 5 && reply.data[1] == 1 {
            Some(reply.data[0])
        } else {
            None
        }
    }

    #[no_mangle]
    pub extern "C" fn shell_main() -> ! {
        // Discover the keyboard server PID via syscall
        let kbd_pid = unsafe { syscall1(syscall_nr::GET_SERVER_PID, 2) };
        if kbd_pid <= 0 {
            // Keyboard server not available — halt
            loop {}
        }
        let kbd_pid = kbd_pid as u16;

        // Line buffer for the current input line
        let mut line_buf: [u8; LINE_BUF_SIZE] = [0; LINE_BUF_SIZE];
        let mut line_len: usize = 0;

        // Display initial prompt
        print_str("Shell> ");

        loop {
            // Try to get a key from the keyboard server
            if let Some(ch) = get_key(kbd_pid) {
                match ch {
                    b'\n' | b'\r' => {
                        // Enter: echo newline and process the command
                        print_char(b'\n');
                        // For now, just echo the input back
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
                        // Backspace: remove last character if any
                        if line_len > 0 {
                            line_len -= 1;
                            print_char(0x08); // VGA cursor back
                        }
                    }
                    _ => {
                        // Regular character: append to line buffer and echo
                        if line_len < LINE_BUF_SIZE - 1 {
                            line_buf[line_len] = ch;
                            line_len += 1;
                        }
                        print_char(ch);
                    }
                }
            } else {
                // No key available — yield CPU cooperatively
                unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
            }
        }
    }
}
