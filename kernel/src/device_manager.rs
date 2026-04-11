//! Device Manager: Centralizes access and logic for hardware interaction.
//! This component acts as the Hardware Abstraction Layer (HAL) facade,
//! ensuring that subsystems do not directly call low-level I/O routines.

use crate::ipc::{Message, IPC_QUEUE};
use crate::vga;
use core::sync::atomic::{AtomicUsize, Ordering};

/// A central hub for communicating with hardware services through the message queue.
pub struct DeviceManager {
    pub current_cursor: AtomicUsize,
}

impl DeviceManager {
    /// Initializes the manager and sets up initial hardware states (e.g., cursor position).
    pub fn new() -> Self {
        Self {
            current_cursor: AtomicUsize::new(0),
        }
    }

    /// Sends a message to the IPC queue, assuming the kernel core is listening for it.
    fn send_ipc(&self, msg: Message) -> bool {
        unsafe {
            let queue = &mut IPC_QUEUE;
            queue.send(msg)
        }
    }

    /// Initializes VGA by clearing the screen and setting initial cursor position.
    pub fn initialize_vga(&self) {
        let _ = self.send_ipc(Message::VgaClear);
        self.current_cursor.store(0, Ordering::Relaxed);
    }

    /// Writes a visible character by sending an IPC message.
    pub fn write_char(&self, byte: u8) -> bool {
        self.send_ipc(Message::VgaPrint(byte))
    }

    /// Writes an entire string by dispatching character writes sequentially.
    pub fn print_string(&self, s: &str) {
        for byte in s.as_bytes() {
            let _ = self.write_char(*byte);
        }
    }

    /// Moves the cursor back one cell and erases it by sending an IPC command.
    pub fn backspace(&self) -> bool {
        let _ = self.send_ipc(Message::VgaBackspace);
        true
    }
}

// We need a global instance to act as the singleton access point for hardware services in this simulation phase.
pub static DEVICE_MANAGER: DeviceManager = DeviceManager {
    current_cursor: AtomicUsize::new(0),
};

// Helper function to expose manager functionality easily from main loop
pub fn get_device_manager() -> &'static DeviceManager {
    &DEVICE_MANAGER
}
