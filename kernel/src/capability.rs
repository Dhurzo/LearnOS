//! Capability-Based IPC (Phase 4 — System Hardening)
//!
//! Capabilities are unforgeable tokens that grant a process the right to
//! send messages to a specific IPC endpoint.  Without a valid capability,
//! a process cannot communicate with the target.
//!
//! Each process holds a bitmap of capabilities it owns (stored in its PCB).
//! The kernel grants capabilities at process creation or via explicit
//! `SYS_GRANT_CAP` from a privileged process.

/// Maximum number of distinct capability types.
pub const MAX_CAPABILITIES: usize = 64;

/// Well-known capability indices.
pub mod cap_id {
    /// Send to VGA server
    pub const VGA_SEND: usize = 0;
    /// Send to keyboard server
    pub const KEYBOARD_SEND: usize = 1;
    /// Send to interrupt server
    pub const INTERRUPT_SEND: usize = 2;
    /// Send to any driver
    pub const DRIVER_SEND: usize = 3;
    /// Create new processes (fork)
    pub const FORK: usize = 4;
    /// Load executables (exec)
    pub const EXEC: usize = 5;
    /// Register IRQ handlers
    pub const IRQ_REGISTER: usize = 6;
}

/// Grant a capability to a process.
///
/// # Safety
///
/// The caller must be privileged (kernel or a process with GRANT capability).
pub unsafe fn grant(proc_caps: &mut u64, cap: usize) -> bool {
    if cap >= MAX_CAPABILITIES {
        return false;
    }
    *proc_caps |= 1u64 << cap;
    true
}

/// Revoke a capability from a process.
pub unsafe fn revoke(proc_caps: &mut u64, cap: usize) -> bool {
    if cap >= MAX_CAPABILITIES {
        return false;
    }
    *proc_caps &= !(1u64 << cap);
    true
}

/// Check whether a process holds a given capability.
pub fn has(proc_caps: u64, cap: usize) -> bool {
    if cap >= MAX_CAPABILITIES {
        return false;
    }
    proc_caps & (1u64 << cap) != 0
}

/// Verify that a process can send an IPC message to the given endpoint.
///
/// This is called by `sys_ipc_send` (Phase 4 hardening).  If the sender
/// does not hold a capability for the target endpoint type, the message
/// is rejected.
pub fn check_ipc_send(sender_caps: u64, dst_pid: u16) -> bool {
    // PID 0 is kernel — always allowed
    if dst_pid == 0 {
        return true;
    }
    // For now, map destination PIDs to capability indices:
    //   PID 1 → VGA_SEND
    //   PID 2 → KEYBOARD_SEND
    //   Others → DRIVER_SEND
    let required_cap = match dst_pid {
        1 => cap_id::VGA_SEND,
        2 => cap_id::KEYBOARD_SEND,
        _ => cap_id::DRIVER_SEND,
    };
    has(sender_caps, required_cap)
}
