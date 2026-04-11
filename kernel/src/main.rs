//! Microkernel Core: PVH + IPC + Services
//!
//! This crate implements the core of a microkernel. It does NOT contain
//! hardware drivers directly. Instead, it manages a message queue (IPC)
//! and dispatches messages to appropriate services (simulated here as modules).
//! Drivers (VGA, Keyboard) are now decoupled and communicate via messages.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

mod ipc;
mod keyboard;
mod vga;
mod device_manager;

core::arch::global_asm!(include_str!("boot.S"));

use crate::ipc::{Message, IPC_QUEUE};
use crate::device_manager::get_device_manager;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    const MAX_INPUT: usize = 128;
    let mut input = [0u8; MAX_INPUT];
    let mut len = 0usize;

    // --- Microkernel Initialization Phase ---
    let device_manager = get_device_manager();
    
    // 1. Initialize VGA Service (send clear command)
    device_manager.initialize_vga();

    // 2. Print boot messages
    device_manager.print_string("Microkernel booting...\n");
    device_manager.print_string("Services: VGA [OK], Keyboard [OK]\n");
    device_manager.print_string("Type text and press Enter\n");
    device_manager.print_string("> ");

    // --- Main Loop: Message Dispatcher ---
    // In this phase, we simulate the kernel loop acting as a message dispatcher.
    // It polls the keyboard for input and dispatches it, while also checking the IPC queue.
    loop {
        // 1. Poll for input from Keyboard Service (Driver)
        if let Some(event) = keyboard::poll_keyboard() {
            match event {
                keyboard::KeyEvent::Char(byte) => {
                    if len < input.len() {
                        input[len] = byte;
                        len += 1;
                        // Send to VGA Service via IPC
                        device_manager.write_char(byte);
                    }
                }
                keyboard::KeyEvent::Backspace => {
                    if len > 0 {
                        len -= 1;
                        // Send to VGA Service via IPC
                        device_manager.backspace();
                    }
                }
                keyboard::KeyEvent::Enter => {
                    // Process line: Echo back
                    let line = &input[..len];
                    
                    // Send newline
                    unsafe {
                        let queue = &mut *(&mut IPC_QUEUE as *mut crate::ipc::MessageQueue);
                        let _ = queue.send(Message::VgaPrint(b'\n'));
                    }
                    vga::print_vga("Echo: ");
                    
                    // Echo each char (send to service)
                    for &byte in line {
                        device_manager.write_char(byte);
                    }
                    
                    // Send new prompt
                    unsafe {
                        let queue = &mut *(&mut IPC_QUEUE as *mut crate::ipc::MessageQueue);
                        let _ = queue.send(Message::VgaPrint(b'\n'));
                        let _ = queue.send(Message::VgaPrint(b'>'));
                        let _ = queue.send(Message::VgaPrint(b' '));
                    }
                    
                    len = 0;
                }
            }
        }

        // 2. Process messages from IPC Queue (e.g., from other services)
        unsafe {
            let queue = &mut *(&mut IPC_QUEUE as *mut crate::ipc::MessageQueue);
            while let Some(msg) = queue.receive() {
                match msg {
                    Message::VgaPrint(byte) => vga::write_byte(byte),
                    Message::VgaClear => vga::clear_screen(),
                    Message::VgaBackspace => vga::backspace(),
                    Message::VgaNewline => vga::write_byte(b'\n'),
                    Message::KeyboardEvent(e) => {
                        // Handle keyboard events from other sources if any
                        match e {
                            ipc::KeyEvent::Char(byte) => vga::write_byte(byte),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}