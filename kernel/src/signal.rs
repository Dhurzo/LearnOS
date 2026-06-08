//! Signal Delivery (Phase 4 — System Hardening)
//!
//! Signals are a lightweight notification mechanism.  The kernel sets a bit
//! in the target process's `signal_pending` bitmap, and the signal is
//! delivered before the next return to user-space (or immediately if the
//! process is currently running, by altering its register state).
//!
//! Supported signals:
//!   SIGKILL  0  — Terminate process immediately
//!   SIGSEGV  1  — Invalid memory access
//!   SIGILL   2  — Illegal instruction
//!   SIGALRM  3  — Timer/ALARM notification
//!   SIGIPC   4  — IPC message delivered
//!   SIGTERM  5  — Graceful termination request
//!   SIGCHLD  6  — Child process exited

/// Maximum number of distinct signals.
pub const MAX_SIGNALS: usize = 64;

/// Well-known signal numbers.
pub mod sig_num {
    pub const SIGKILL: usize = 0;
    pub const SIGSEGV: usize = 1;
    pub const SIGILL: usize = 2;
    pub const SIGALRM: usize = 3;
    pub const SIGIPC: usize = 4;
    pub const SIGTERM: usize = 5;
    pub const SIGCHLD: usize = 6;
    pub const SIGUSER1: usize = 7;
    pub const SIGUSER2: usize = 8;
}

/// Signal action: how the process handles a signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalAction {
    /// Terminate the process.
    Kill,
    /// Ignore the signal.
    Ignore,
    /// Call the registered handler (if any), otherwise use default.
    Handler,
}

/// Default action for each signal number.
pub fn default_action(sig: usize) -> SignalAction {
    match sig {
        sig_num::SIGKILL => SignalAction::Kill,
        sig_num::SIGSEGV => SignalAction::Kill,
        sig_num::SIGILL => SignalAction::Kill,
        sig_num::SIGTERM => SignalAction::Kill,
        sig_num::SIGALRM => SignalAction::Ignore,
        sig_num::SIGIPC => SignalAction::Ignore, // handled via IPC poll
        sig_num::SIGCHLD => SignalAction::Ignore,
        _ => SignalAction::Kill,
    }
}

/// Send a signal to a process.
///
/// Sets the bit in `signal_pending` and, if the process has a registered
/// handler, marks it as ready so it will be scheduled.
///
/// # Safety
///
/// The caller must ensure the process slot index is valid.
pub unsafe fn send_signal(pid: u16, sig: usize) {
    if sig >= MAX_SIGNALS {
        return;
    }
    let table = &mut crate::process::PROCESS_TABLE;
    if let Some(proc) = table.get_mut(pid) {
        proc.signal_pending |= 1u64 << sig;

        // If process was blocked, wake it up
        if proc.state == crate::process::ProcessState::Blocked {
            proc.state = crate::process::ProcessState::Ready;
        }
    }
}

/// Deliver one pending signal to the current process (called before
/// `iretq` to user-space).  Returns `true` if a signal was handled.
///
/// If the signal action is `Kill`, the process is terminated immediately.
/// If the action is `Ignore`, the bit is cleared and we continue.
/// If the action is `Handler` and a handler is registered, we set up the
/// registers to call the handler on return to user-space.
///
/// # Safety
///
/// Must be called with interrupts disabled, inside the scheduler path.
pub unsafe fn deliver_pending(pid: u16) -> bool {
    let table = &mut crate::process::PROCESS_TABLE;
    let proc = if let Some(p) = table.get_mut(pid) {
        p
    } else {
        return false;
    };

    let pending = proc.signal_pending;
    if pending == 0 {
        return false;
    }

    // Find the lowest set bit
    let sig = pending.trailing_zeros() as usize;
    // Clear the bit
    proc.signal_pending &= !(1u64 << sig);

    // Default action
    match default_action(sig) {
        SignalAction::Kill => {
            // Terminate the process
            proc.state = crate::process::ProcessState::Terminated;
            crate::vga::serial_print("signal kill\n");
            true
        }
        SignalAction::Ignore => {
            // Nothing to do; bit already cleared
            true
        }
        SignalAction::Handler => {
            if let Some(handler_addr) = proc.signal_handlers[sig] {
                // Save the current return address (RIP) on the user stack
                // and redirect execution to the signal handler.
                // The handler can call SYS_WAIT_SIGNAL to clear the signal.
                let old_rsp = proc.registers.rsp;
                // Push old RIP onto user stack
                let new_rsp = old_rsp.wrapping_sub(8);
                // Write old RIP to user stack via the process's page table
                let old_rip = proc.registers.rip;
                // For safety, we write to the stack through the mapped page
                // (the stack is at USER_STACK_VADDR - size, and we know it's mapped)
                if new_rsp > crate::paging::USER_STACK_VADDR - 4096 * 8 {
                    let page_offset = new_rsp as usize % 4096;
                    let page_base = new_rsp - page_offset as u64;
                    // Map the stack page if needed (it should already be mapped)
                    // Write the return address directly
                    unsafe {
                        let stack_slot = new_rsp as *mut u64;
                        core::ptr::write(stack_slot, old_rip);
                    }
                    // Set up the handler frame
                    proc.registers.rsp = new_rsp;
                    proc.registers.rip = handler_addr;
                }
            } else {
                // No handler registered — use default action
                proc.state = crate::process::ProcessState::Terminated;
            }
            true
        }
    }
}
