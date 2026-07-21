//! Interrupt Server Module
//!
//! Phase 1 of the microkernel plan: moves IRQ-to-server bridging out of the
//! kernel into a user-space interrupt server.  The interrupt server receives
//! raw IRQ messages from the kernel and forwards them to registered handler
//! processes (drivers) via IPC.
//!
//! This module provides the kernel-side infrastructure:
//!   - A handler table: (irq → [handler_pid; 4])
//!   - Registration / deregistration syscall helpers
//!   - IRQ forwarding from kernel handlers to the interrupt server
//!
//! The actual interrupt-server process lives in user space
//! (interrupt_server/src/main.rs) and loops on IPC_RECV.

use crate::process::{IpcMessage, PROCESS_TABLE};

/// Number of hardware IRQs managed by this table (IRQ 0–3: Timer, Keyboard, RTC, Serial).
const BASE_IRQ_COUNT: usize = 4;

/// Maximum number of handler processes that can be registered per IRQ.
const HANDLERS_PER_IRQ: usize = 4;

/// Per-IRQ handler PID table — for each IRQ (indexed 0–3), holds up to `HANDLERS_PER_IRQ` PIDs.
/// A zero entry means the slot is free.
static mut IRQ_HANDLERS: [[u16; HANDLERS_PER_IRQ]; BASE_IRQ_COUNT] =
    [[0; HANDLERS_PER_IRQ]; BASE_IRQ_COUNT];

/// PID of the user-space interrupt server process (0 means not yet registered).
static mut INTERRUPT_SERVER_PID: u16 = 0;

// =============================================================================
// Initialisation
// =============================================================================

/// Set the PID of the interrupt server.
pub fn set_interrupt_server_pid(pid: u16) {
    unsafe { INTERRUPT_SERVER_PID = pid };
}

/// Get the PID of the interrupt server.
pub fn get_interrupt_server_pid() -> u16 {
    unsafe { INTERRUPT_SERVER_PID }
}

// =============================================================================
// Registration helpers (called from syscall handlers)
// =============================================================================

/// Register a process as a handler for a given IRQ.
///
/// Returns `true` on success, `false` if the IRQ is unknown or all slots full.
pub fn register_handler(irq: u8, pid: u16) -> bool {
    if (irq as usize) >= BASE_IRQ_COUNT || pid == 0 {
        return false;
    }
    unsafe {
        let slots = &mut IRQ_HANDLERS[irq as usize];
        for slot in slots.iter_mut() {
            if *slot == 0 {
                *slot = pid;
                return true;
            }
        }
    }
    false // All slots full
}

/// Unregister a handler for a given IRQ.
pub fn unregister_handler(irq: u8, pid: u16) -> bool {
    if (irq as usize) >= BASE_IRQ_COUNT {
        return false;
    }
    unsafe {
        let slots = &mut IRQ_HANDLERS[irq as usize];
        for slot in slots.iter_mut() {
            if *slot == pid {
                *slot = 0;
                return true;
            }
        }
    }
    false
}

/// Check whether any handler is registered for a given IRQ.
pub fn has_handler(irq: u8) -> bool {
    if (irq as usize) >= BASE_IRQ_COUNT {
        return false;
    }
    unsafe {
        IRQ_HANDLERS[irq as usize].iter().any(|&pid| pid != 0)
    }
}

/// Get all registered handler PIDs for an IRQ.
pub fn get_handlers(irq: u8) -> &'static [u16] {
    if (irq as usize) >= BASE_IRQ_COUNT {
        return &[];
    }
    unsafe { &IRQ_HANDLERS[irq as usize] }
}

// =============================================================================
// IRQ forwarding (called from kernel IRQ handlers)
// =============================================================================

/// Forward a hardware IRQ to the interrupt server (or directly to handlers).
///
/// If an interrupt server is registered, the message goes there first.
/// Otherwise, it is delivered directly to all registered handlers for `irq`.
///
/// `irq`   — hardware IRQ number.
/// `data`  — up to 60 bytes of IRQ-specific payload (e.g. scancode).
pub fn forward_irq(irq: u8, data: &[u8; 60]) {
    let isr_pid = unsafe { INTERRUPT_SERVER_PID };

    if isr_pid != 0 {
        // Route through the interrupt server
        let msg = IpcMessage::new(
            0,                      // src_pid = kernel
            4,                      // msg_type = MSG_HW_IRQ
            {
                let mut d = [0u8; 60];
                d[0] = irq;
                d[1..].copy_from_slice(data);
                d
            },
        );
        unsafe {
            let pt = &raw mut PROCESS_TABLE;
            if let Some(p) = (*pt).get_mut(isr_pid) {
                let _ = p.ipc_push(msg);
            }
        }
    } else {
        // No interrupt server — deliver directly (backward-compatible path)
        // Use the correct message type per IRQ:
        //   IRQ 1 (keyboard) → msg_type 3 (MSG_KEY_SCANCODE)
        //   Other IRQs       → msg_type 5 (MSG_IRQ_DATA)
        let msg_type = if irq == 1 { 3u16 } else { 5u16 };
        for &pid in get_handlers(irq) {
            if pid == 0 {
                continue;
            }
            let msg = IpcMessage::new(0, msg_type, *data);
            unsafe {
                let pt = &raw mut PROCESS_TABLE;
                if let Some(p) = (*pt).get_mut(pid) {
                    let _ = p.ipc_push(msg);
                }
            }
        }
    }
}

/// Translate a PC scancode (0–57) to an ASCII character.
///
/// Covers the primary QWERTY row of the keyboard. Scancodes outside this
/// range fall back to `'?'`. This is intentionally minimal — only used by
/// the interrupt-server loop for quick echo; full decoding should happen in
/// the user-space keyboard server.
fn translate_scancode(scancode: u8) -> u8 {
    // QWERTY row 1
    const SC2ASCII: &[u8] = b"1234567890-=\x08\tqwertyuiop[]\nasdfghjkl;'`\\zxcvbnm,./ ";
    if scancode < SC2ASCII.len() as u8 {
        SC2ASCII[scancode as usize]
    } else {
        b'?'
    }
}
