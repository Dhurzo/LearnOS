//! Device Manager: Hardware Abstraction Layer (HAL) Facade
//!
//! This module provides a centralized interface for interacting with hardware devices.
//! In a microkernel architecture, drivers (services) should not be called directly by
//! the kernel core. Instead, the kernel dispatches requests through an abstraction layer.
//!
//! ## Role in Microkernel
//!
//! The `DeviceManager` acts as the primary facade for the kernel's initialization and
//! boot process. It translates high-level operations (like "print a string") into
//! Inter-Process Communication (IPC) messages that the corresponding service (VGA) handles.
//!
//! This decouples the kernel logic from specific hardware implementations. For example,
//! the kernel doesn't need to know the memory address of the VGA buffer; it simply sends
//! a `VgaPrint` message.
//!
//! ## Services Managed
//!
//! - **VGA/Display**: Handles text output to the screen.
//! - **Keyboard**: (Managed via direct polling in this version, but could be abstracted similarly).
//!
//! ## Implementation Details
//!
//! It uses a global static instance (`DEVICE_MANAGER`) to provide singleton access
//! throughout the kernel lifetime. It wraps the global `IPC_QUEUE` to send messages
//! to services.
//!
//! # Example
//!
//! ```rust
//! let dm = get_device_manager();
//! dm.print_string("Hello Microkernel!\n");
//! ```
//!
//! The above code does not write to video memory directly. Instead, it enqueues a
//! `Message::VgaPrint('H')` for the VGA Service to process.

use crate::ipc::{Message, IPC_QUEUE};
use core::sync::atomic::{AtomicUsize, Ordering};

/// The Device Manager coordinates hardware access.
///
/// It maintains the state of the system (like cursor position) and
/// provides a safe API for the kernel to request hardware operations.
pub struct DeviceManager {
    /// Tracks the current software cursor position.
    /// This is a simplification; in a real system, the VGA service would own this state.
    pub current_cursor: AtomicUsize,
}

impl DeviceManager {
    /// Constructs a new DeviceManager.
    ///
    /// Initializes the cursor to position 0.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            current_cursor: AtomicUsize::new(0),
        }
    }

    /// Sends a message to the system IPC queue.
    ///
    /// This is the core mechanism by which the DeviceManager communicates with services.
    fn send_ipc(&self, msg: Message) -> bool {
        unsafe {
            let queue = &mut *(&raw mut IPC_QUEUE as *mut crate::ipc::MessageQueue);
            queue.send(msg)
        }
    }

    /// Initializes the Display Service (VGA).
    ///
    /// Sends a clear command to the VGA service and resets the local cursor state.
    pub fn initialize_vga(&self) {
        let _ = self.send_ipc(Message::VgaClear);
        self.current_cursor.store(0, Ordering::Relaxed);
    }

    /// Writes a single character to the display.
    ///
    /// Converts the byte to a `VgaPrint` message and sends it to the queue.
    pub fn write_char(&self, byte: u8) -> bool {
        self.send_ipc(Message::VgaPrint(byte))
    }

    /// Writes a string of text to the display.
    ///
    /// Iterates over the string bytes and sends them individually to the service.
    /// Note: This is inefficient for high performance but clear for demonstration.
    pub fn print_string(&self, s: &str) {
        // Also print to serial for debugging
        crate::vga::serial_print(s);

        for byte in s.as_bytes() {
            let _ = self.write_char(*byte);
        }
    }

    /// Simulates a backspace operation.
    ///
    /// Sends a backspace command to the VGA service.
    pub fn backspace(&self) -> bool {
        let _ = self.send_ipc(Message::VgaBackspace);
        true
    }
}

/// Global singleton instance of the DeviceManager.
///
/// This is used by the kernel core to access hardware without knowing
/// specific driver details.
pub static DEVICE_MANAGER: DeviceManager = DeviceManager {
    current_cursor: AtomicUsize::new(0),
};

/// Returns a reference to the global DeviceManager.
pub fn get_device_manager() -> &'static DeviceManager {
    &DEVICE_MANAGER
}
