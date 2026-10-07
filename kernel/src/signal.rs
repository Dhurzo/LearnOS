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
//!   SIGSTOP  9  — Suspend process (unschedule until SIGCONT)
//!   SIGCONT  10 — Resume a stopped process

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
    pub const SIGSTOP: usize = 9;
    pub const SIGCONT: usize = 10;
}

/// Signal action: how the process handles a signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalAction {
    /// Terminate the process.
    Kill,
    /// Ignore the signal.
    Ignore,
    /// Suspend the process (scheduler skips it until SIGCONT).
    Stop,
    /// Resume a stopped process (no-op otherwise).
    Continue,
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
        sig_num::SIGSTOP => SignalAction::Stop,
        sig_num::SIGCONT => SignalAction::Continue,
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

        // If process was blocked, wake it up. Signal-waiters set state=Blocked and
        // a non-zero deadline (see sys_wait_signal); any pending signal makes them
        // runnable again immediately.
        if proc.state == crate::process::ProcessState::Blocked {
            proc.state = crate::process::ProcessState::Ready;
        }
        // A stopped process is never scheduled, so `deliver_pending` would
        // never run for it: apply SIGCONT synchronously here instead.
        if sig == sig_num::SIGCONT && proc.state == crate::process::ProcessState::Stopped {
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
        SignalAction::Stop => {
            // Suspend: the scheduler's `next_ready_pid` only picks Ready
            // processes, and the resume path skips Stopped like Terminated,
            // so this process will not run again until SIGCONT.
            proc.state = crate::process::ProcessState::Stopped;
            crate::vga::serial_print("signal stop\n");
            true
        }
        SignalAction::Continue => {
            // Resume a stopped process; harmless no-op otherwise.
            if proc.state == crate::process::ProcessState::Stopped {
                proc.state = crate::process::ProcessState::Ready;
                crate::vga::serial_print("signal cont\n");
            }
            true
        }
        SignalAction::Handler => {
            if let Some(handler_addr) = proc.signal_handlers[sig] {
                // Save the current return address (RIP) on the user stack
                // and redirect execution to the signal handler.
                // The handler can call SYS_WAIT_SIGNAL to clear the signal.
                let old_rsp = proc.registers.rsp;
                // Push old RIP onto user stack (8-byte slot just below RSP).
                let new_rsp = old_rsp.wrapping_sub(8);
                let old_rip = proc.registers.rip;
                // Write old RIP through the process's OWN page tables (cr3),
                // translating the user virtual address to its physical frame.
                // This is essential: `new_rsp` is a USER vaddr, and writing to
                // it as a raw pointer from ring 0 would hit the kernel's
                // identity-mapped frame instead of the real user stack, so the
                // handler would later pop garbage RIP on return.
                // `to_le_bytes()` writes the same 8 bytes that a native u64 push
                // would (x86-64 is little-endian), but now routed through the
                // process's page tables.
                let delivered = unsafe {
                    crate::paging::write_user_memory(proc.cr3, new_rsp, &old_rip.to_le_bytes())
                };
                if delivered.is_ok() {
                    // Frame set up correctly: the handler runs on return and its
                    // `iretq` pops the old RIP we just stored.
                    proc.registers.rsp = new_rsp;
                    proc.registers.rip = handler_addr;
                } else {
                    // Target page is unmapped or not writable — treat as a fatal
                    // fault for THIS process rather than crashing the kernel
                    // (no panic). The bit was already cleared above.
                    crate::vga::serial_print("signal: bad user stack addr\n");
                    proc.state = crate::process::ProcessState::Terminated;
                }
            } else {
                // No handler registered — use default action
                proc.state = crate::process::ProcessState::Terminated;
            }
            true
        }
    }
}
