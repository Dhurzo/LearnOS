//! Capability-Based IPC (Phase 4 — System Hardening)
//!
//! Capabilities are unforgeable tokens that grant a process the right to
//! send messages to a specific IPC endpoint. Without a valid capability,
//! a process cannot communicate with the target.
//!
//! Each process holds a bitmap of capabilities it owns (stored in its PCB).
//! The kernel grants capabilities at process creation or via explicit
//! `SYS_GRANT_CAP` from a privileged process.

/// Maximum number of distinct capability types supported by the system.
pub const MAX_CAPABILITIES: usize = 64;

/// Well-known capability indices — the bit positions that correspond to
/// specific IPC endpoints or kernel operations.
pub mod cap_id {
    /// Capability bit for sending messages to PID 1 (VGA server).
    pub const VGA_SEND: usize = 0;
    /// Capability bit for sending messages to PID 2 (keyboard server).
    pub const KEYBOARD_SEND: usize = 1;
    /// Capability bit for sending IRQ registration requests.
    pub const INTERRUPT_SEND: usize = 2;
    /// Capability bit for sending messages to arbitrary driver processes.
    pub const DRIVER_SEND: usize = 3;
    /// Capability bit required to call `SYS_FORK` / create child processes.
    pub const FORK: usize = 4;
    /// Capability bit required to call `SYS_EXEC` / load executables.
    pub const EXEC: usize = 5;
    /// Capability bit required to register as an IRQ handler via interrupt server.
    pub const IRQ_REGISTER: usize = 6;
}

/// Grant a capability bit to the process's bitmap.
///
/// # Safety
///
/// The caller must be privileged (kernel or a process with `GRANT` capability).
/// Invalid bits are silently ignored.
pub unsafe fn grant(proc_caps: &mut u64, cap: usize) -> bool {
    if cap >= MAX_CAPABILITIES {
        return false;
    }
    *proc_caps |= 1u64 << cap;
    true
}

/// Revoke a capability bit from the process's bitmap.
pub unsafe fn revoke(proc_caps: &mut u64, cap: usize) -> bool {
    if cap >= MAX_CAPABILITIES {
        return false;
    }
    *proc_caps &= !(1u64 << cap);
    true
}

/// Check whether the process holds a given capability bit.
pub fn has(proc_caps: u64, cap: usize) -> bool {
    if cap >= MAX_CAPABILITIES {
        return false;
    }
    proc_caps & (1u64 << cap) != 0
}

/// Verify that a process can send an IPC message to the given endpoint.
///
/// This is called by `sys_ipc_send` during Phase 4 hardening. If the sender
/// does not hold a capability for the target endpoint type, the message
/// is rejected at the syscall boundary.
///
/// # Mapping rules
/// - PID 0 (kernel) — always permitted
/// - PID 1 → `VGA_SEND`
/// - PID 2 → `KEYBOARD_SEND`
/// - All other PIDs → `DRIVER_SEND`
pub fn check_ipc_send(sender_caps: u64, dst_pid: u16) -> bool {
    // PID 0 is kernel — always allowed
    if dst_pid == 0 {
        return true;
    }
    let required_cap = match dst_pid {
        1 => cap_id::VGA_SEND,
        2 => cap_id::KEYBOARD_SEND,
        _ => cap_id::DRIVER_SEND,
    };
    has(sender_caps, required_cap)
}
