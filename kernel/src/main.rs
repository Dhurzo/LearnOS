//! Microkernel Core: PVH + IPC + Services
//!
//! This crate implements the core of a microkernel architecture for the LearnOS project.
//! Unlike a monolithic kernel where drivers and system services run in kernel space,
//! this implementation decouples hardware drivers into "services" that communicate
//! via Inter-Process Communication (IPC).
//!
//! ## Architectural Overview
//!
//! The microkernel is designed around a central **Message Dispatcher** loop.
//! Its responsibilities are:
//! 1.  **Initialization**: Set up the hardware abstraction layer (DeviceManager).
//! 2.  **Scheduling**: Poll hardware for events (e.g., keyboard input) and dispatch them.
//! 3.  **IPC Routing**: Process messages in the global IPC queue, forwarding them to the appropriate service handler.
//! 4.  **System Call Interface**: (Future) Handle requests from user-space processes.
//!
//! ## Components
//!
//! - **ipc**: Defines the message passing primitives (`MessageQueue`) and message types.
//! - **device_manager**: Acts as the Hardware Abstraction Layer (HAL), sending commands to services.
//! - **vga**: The VGA Service. It listens for drawing commands and performs memory-mapped I/O.
//! - **keyboard**: The Keyboard Service. It provides raw input events to be consumed by the dispatcher.
//!
//! ## Safety Model
//!
//! This code runs in a `no_std` environment without a runtime.
//! - **No Heap**: All memory is static or stack-allocated.
//! - **No Locks**: Concurrency is managed via atomic operations in the IPC queue.
//! - **Unsafe Boundaries**: Hardware access (port I/O, volatile memory) is contained in dedicated modules.
//!
//! ## Boot Flow
//!
//! 1.  **Bootloader (`boot.S`)**: Initializes 64-bit mode and page tables, jumps to `_start`.
//! 2.  **`_start` (here)**: Initializes the `DeviceManager` and prints boot messages.
//! 3.  **Main Loop**: Enters the infinite dispatch loop.
//!
//! # Examples
//!
//! To visualize the flow of a key press:
//! `Keyboard (HW)` -> `poll_keyboard()` -> `KeyEvent` -> `Dispatcher` -> `IPC_QUEUE` -> `VGA Service` -> `Video Memory`

#![no_std] // Don't use the standard library (no heap, no stdio)
#![no_main] // Custom entry point (_start) instead of main

use core::panic::PanicInfo;

mod device_manager;
mod ipc;
mod keyboard;
mod vga;

// Include the boot assembly which handles CPU mode switching before Rust runs.
core::arch::global_asm!(include_str!("boot.S"));

// Import core IPC components
use crate::device_manager::get_device_manager;
use crate::ipc::{Message, IPC_QUEUE};

/// The kernel entry point, called by the bootloader (boot.S).
///
/// This function sets up the microkernel environment and transitions to the
/// main dispatch loop.
///
/// # Safety
///
/// This is the first Rust function executed. It assumes the CPU is in 64-bit
/// long mode with paging enabled. It never returns.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Input buffer for the line editor.
    const MAX_INPUT: usize = 128;
    let mut input = [0u8; MAX_INPUT];
    let mut len = 0usize;

    // --- Microkernel Initialization Phase ---
    //
    // In a microkernel, we don't mount drivers directly. We initialize a
    // "Device Manager" which acts as the interface to the system.
    let device_manager = get_device_manager();

    // 1. Initialize the VGA Service (send a clear command via IPC/HAL)
    device_manager.initialize_vga();

    // 2. Print boot messages to verify the service is working
    device_manager.print_string("Microkernel booting...\n");
    device_manager.print_string("Services: VGA [OK], Keyboard [OK]\n");
    device_manager.print_string("Type text and press Enter\n");
    device_manager.print_string("> ");

    // --- Main Loop: Message Dispatcher ---
    //
    // This loop simulates the role of the kernel core. It:
    // 1. Polls for hardware events (Keyboard)
    // 2. Processes IPC messages from other services (if any)
    //
    // In a fully fledged microkernel, this would also handle context switching
    // between user processes.
    loop {
        // 1. Poll for input from Keyboard Service (Driver)
        // We use non-blocking polling here. If a key is pressed, we process it.
        if let Some(event) = keyboard::poll_keyboard() {
            match event {
                keyboard::KeyEvent::Char(byte) => {
                    // Buffer the character locally
                    if len < input.len() {
                        input[len] = byte;
                        len += 1;
                        // Send to VGA Service via IPC (DeviceManager facade)
                        device_manager.write_char(byte);
                    }
                }
                keyboard::KeyEvent::Backspace => {
                    if len > 0 {
                        len -= 1;
                        // Send backspace command to VGA Service
                        device_manager.backspace();
                    }
                }
                keyboard::KeyEvent::Enter => {
                    // Process line completion (Echo logic)
                    let line = &input[..len];

                    // Send newline character via IPC
                    // Note: We use unsafe here to alias the static mutable IPC_QUEUE.
                    // In a real kernel, this would be abstracted into a system call or safe wrapper.
                    unsafe {
                        let queue = &mut *(&raw mut IPC_QUEUE as *mut crate::ipc::MessageQueue);
                        let _ = queue.send(Message::VgaPrint(b'\n'));
                    }
                    vga::print_vga("Echo: ");

                    // Echo each character back to the user
                    for &byte in line {
                        device_manager.write_char(byte);
                    }

                    // Send new prompt
                    unsafe {
                        let queue = &mut *(&raw mut IPC_QUEUE as *mut crate::ipc::MessageQueue);
                        let _ = queue.send(Message::VgaPrint(b'\n'));
                        let _ = queue.send(Message::VgaPrint(b'>'));
                        let _ = queue.send(Message::VgaPrint(b' '));
                    }

                    len = 0;
                }
            }
        }

        // 2. Process messages from IPC Queue
        // This handles messages sent by other services or background tasks.
        unsafe {
            let queue = &mut *(&raw mut IPC_QUEUE as *mut crate::ipc::MessageQueue);
            while let Some(msg) = queue.receive() {
                match msg {
                    // Route messages to the VGA Service handler
                    Message::VgaPrint(byte) => vga::write_byte(byte),
                    Message::VgaClear => vga::clear_screen(),
                    Message::VgaBackspace => vga::backspace(),
                    Message::VgaNewline => vga::write_byte(b'\n'),
                    // Handle external keyboard events (e.g., from a virtual terminal)
                    Message::KeyboardEvent(e) => match e {
                        ipc::KeyEvent::Char(byte) => vga::write_byte(byte),
                        _ => {}
                    },
                }
            }
        }
    }
}

/// Panic handler for the kernel.
///
/// In a microkernel, panics are critical failures. In a more advanced
/// implementation, we would send a message to a "Debug Service" running
/// in user space to log the crash.
///
/// # Safety
///
/// This function is unsafe because it halts the CPU and never returns.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // Halting the CPU in a tight loop.
    loop {}
}
