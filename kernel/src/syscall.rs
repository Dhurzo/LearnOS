//! System Call Interface
//!
//! This module handles all communication between user-space and the kernel.
//! User programs call `syscall` to request kernel services, and the kernel
//! dispatches them through a table of handler functions.
//!
//! =============================================================================
//! SYSCALL MECHANISM (syscall/sysret)
//! =============================================================================
//!
//! We use the x86-64 `syscall`/`sysretq` instructions (not `int 0x80`).
//!
//! On `syscall`:
//!   1. CPU saves RIP → RCX, RFLAGS → R11
//!   2. CPU loads RIP from LSTAR MSR (→ syscall_entry)
//!   3. CPU loads CS:SS from STAR MSR
//!   4. CPU clears RFLAGS bits set in SFMASK MSR (including IF)
//!   5. CPU does NOT switch RSP — we must do that ourselves
//!
//! On `sysretq`:
//!   1. CPU loads RIP from RCX, RFLAGS from R11
//!   2. CPU loads CS:SS from STAR[63:48] + 16 / + 8
//!   3. CPU returns to ring 3
//!
//! =============================================================================
//! SYSCALL CONVENTION
//! =============================================================================
//!
//! User → Kernel (syscall instruction):
//!   rax = syscall number
//!   rdi = arg1   rsi = arg2   rdx = arg3
//!   r10 = arg4   r8  = arg5   r9  = arg6
//!
//! Kernel → User (sysretq):
//!   rax = return value (negative = error)
//!
//! =============================================================================
//! TIMER INTERRUPT
//! =============================================================================
//!
//! The timer (IRQ0, vector 0x20) fires ~100 times/sec. When it fires
//! while in ring 3, the CPU uses TSS.RSP0 to switch to the kernel stack
//! and pushes an interrupt frame (SS, RSP, RFLAGS, CS, RIP). Our
//! timer_entry assembly saves all GP registers, calls
//! timer_save_and_switch to save/restore process state, then iretq.

use crate::interrupt_server::INTERRUPT_SERVER_PID;
use crate::process;
use crate::vga;

/// PID of the user-space VGA display server (0 = not yet spawned).
static mut VGA_SERVER_PID: u16 = 0;

/// PID of the user-space keyboard input server (0 = not yet spawned).
pub static mut KEYBOARD_SERVER_PID: u16 = 0;

/// PID of the user-space storage server (VirtIO protocol + FAT service).
static mut STORAGE_SERVER_PID: u16 = 0;

/// Set the PID of the VGA server (called during boot after spawning it).
pub fn set_vga_server_pid(pid: u16) {
    unsafe { VGA_SERVER_PID = pid };
}

/// Set the PID of the keyboard server (called during boot after spawning it).
pub fn set_keyboard_server_pid(pid: u16) {
    unsafe { KEYBOARD_SERVER_PID = pid };
}

/// Set the storage service endpoint after it is successfully spawned.
pub fn set_storage_server_pid(pid: u16) {
    unsafe { STORAGE_SERVER_PID = pid };
}

/// Permission denied: the caller lacks the capability for the requested
/// endpoint or operation. Returned instead of -1 so user space can tell
/// "bad arguments" apart from "not allowed".
pub const EACCES: isize = -2;

pub mod syscall_nr {
    pub const EXIT: usize = 0;
    pub const WRITE: usize = 1;
    pub const READ: usize = 2;
    pub const IPC_SEND: usize = 3;
    pub const IPC_RECV: usize = 4;
    pub const BRK: usize = 5;
    pub const GETPID: usize = 6;
    pub const VGA_WRITE: usize = 7;
    pub const VGA_CLEAR: usize = 8;
    pub const SCHEDULE: usize = 9;
    /// Get a well-known server PID by type:
    ///   1 = VGA server, 2 = keyboard server
    pub const GET_SERVER_PID: usize = 10;
    // Phase 2 — Process creation
    pub const FORK: usize = 13;
    // Phase 4 — Hardening
    pub const REGISTER_IRQ: usize = 15;
    pub const CAP_SEND: usize = 16;
    pub const REGISTER_SIGNAL: usize = 17;
    pub const WAIT_SIGNAL: usize = 18;
    /// Grant a held capability bit to another process (delegation).
    pub const GRANT_CAP: usize = 19;
    /// Send a signal to another process.
    pub const KILL: usize = 20;
    /// Replace this process's image with the ELF at the given user buffer.
    pub const EXEC: usize = 21;
}

/// The legacy INT 0x80 vector is defined here for reference.
/// All actual syscalls use the faster x86-64 SYSCALL instruction (STAR/LSTAR MSRs).
pub const SYSCALL_VECTOR: u8 = 0x80;

/// Syscall handler function pointer type.
///
/// Signature: `(a1, a2, a3, a4, a5, nr) -> isize`.
/// The first five parameters are the argument registers (rdi, rsi, rdx, r10, r8).
/// The last parameter is the syscall number itself.
pub type SyscallHandler = fn(usize, usize, usize, usize, usize, usize) -> isize;

pub struct SyscallTable {
    handlers: [Option<SyscallHandler>; 32],
}

/// Kernel RSP at `syscall_entry`, captured before `call syscall_dispatch`.
///
/// `sys_exec` needs it to rewrite the saved return state (RCX/RFLAGS slots)
/// so `sysretq` resumes the NEW image instead of the old one. Written by
/// assembly on every syscall entry; read only by `sys_exec` on its own path
/// (interrupts are masked on entry via SFMASK, so no nesting races it).
#[no_mangle]
pub static mut SYSCALL_FRAME_SAVE: u64 = 0;

impl SyscallTable {
    pub const fn new() -> Self {
        Self {
            handlers: [None; 32],
        }
    }

    /// Register a handler for the given syscall number.
    ///
    /// If `nr >= 32` the registration is silently ignored (the table has
    /// only 32 slots, matching the x86-64 SYSCALL convention which uses a
    /// fast dispatch path through MSRs).
    pub fn register(&mut self, nr: usize, handler: SyscallHandler) {
        if nr < 32 {
            self.handlers[nr] = Some(handler);
        }
    }

    /// Dispatch a syscall.
    ///
    /// Looks up the handler for `nr` and invokes it with the five argument
    /// registers plus the syscall number itself (convention: the last
    /// parameter of each handler receives `nr`, matching the assembly entry
    /// point which pushes the syscall number before calling into Rust).
    ///
    /// Returns `-1` if no handler is registered for that number.
    pub fn handle(
        &self,
        nr: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> isize {
        self.handlers[nr]
            .map(|h| h(a1, a2, a3, a4, a5, nr))
            .unwrap_or(-1)
    }
}

pub static SYSCALL_TABLE: SyscallTable = {
    let mut table = SyscallTable::new();
    table.handlers[syscall_nr::WRITE] = Some(sys_write);
    table.handlers[syscall_nr::EXIT] = Some(sys_exit);
    table.handlers[syscall_nr::VGA_WRITE] = Some(sys_vga_write);
    table.handlers[syscall_nr::VGA_CLEAR] = Some(sys_vga_clear);
    table.handlers[syscall_nr::SCHEDULE] = Some(sys_schedule);
    table.handlers[syscall_nr::GETPID] = Some(sys_getpid);
    table.handlers[syscall_nr::IPC_SEND] = Some(sys_ipc_send);
    table.handlers[syscall_nr::IPC_RECV] = Some(sys_ipc_recv);
    table.handlers[syscall_nr::GET_SERVER_PID] = Some(sys_get_server_pid);
    // Process creation
    table.handlers[syscall_nr::FORK] = Some(sys_fork);
    // Phase 4 — System hardening
    table.handlers[syscall_nr::REGISTER_IRQ] = Some(sys_register_irq);
    table.handlers[syscall_nr::CAP_SEND] = Some(sys_cap_send);
    table.handlers[syscall_nr::REGISTER_SIGNAL] = Some(sys_register_signal);
    table.handlers[syscall_nr::WAIT_SIGNAL] = Some(sys_wait_signal);
    table.handlers[syscall_nr::GRANT_CAP] = Some(sys_grant_cap);
    table.handlers[syscall_nr::KILL] = Some(sys_kill);
    table.handlers[syscall_nr::EXEC] = Some(sys_exec);
    table.handlers[syscall_nr::BRK] = Some(sys_brk);
    table
};

/// Handles the `WRITE` syscall.
///
/// Writes `count` bytes from the buffer at address `buf` to the
/// specified file descriptor. Only stdout (1) and stderr (2)
/// are supported. Returns the number of bytes written on success,
/// or `-1` on failure.
fn sys_write(fd: usize, buf: usize, count: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    if fd != 1 && fd != 2 {
        return -1;
    }
    if count == 0 {
        return 0;
    }
    let count = count.min(4096);
    if is_valid_user_range(buf, count, false) {
        let mut written = 0usize;
        for i in 0..count {
            let byte = unsafe { core::ptr::read((buf as *const u8).add(i)) };
            if sys_vga_write(byte as usize, 0, 0, 0, 0, syscall_nr::VGA_WRITE) < 0 {
                return if written == 0 { -1 } else { written as isize };
            }
            written += 1;
        }
        written as isize
    } else {
        -1
    }
}

/// EXIT: mark only the caller terminated. The syscall returns to the user
/// trampoline, which spins until the next timer interrupt switches away; the
/// global kernel must never busy-loop with interrupts disabled for one process.
fn sys_exit(_code: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    let current = process::get_current_pid();
    if current == 0 {
        return -1;
    }
    unsafe {
        let table = core::ptr::addr_of_mut!(process::PROCESS_TABLE);
        if let Some(p) = (*table).get_mut(current) {
            p.state = process::ProcessState::Terminated;
            return 0;
        }
    }
    -1
}

/// VGA_WRITE: Send a single ASCII byte to the VGA display server for rendering.
///
/// `a1` = ASCII code of the character to write. If the VGA server is not yet
/// running (early boot / panic path), falls back to direct kernel-side VGA output.
fn sys_vga_write(byte: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    let vga_pid = unsafe { VGA_SERVER_PID };
    if vga_pid == 0 {
        // VGA server not yet running — fall back to direct write (early boot / panic)
        vga::write_byte(byte as u8);
        return 0;
    }
    // Forward to VGA server via IPC (msg_type = 1 = MSG_VGA_PRINT)
    let msg = process::IpcMessage::new(0, 1, {
        let mut d = [0u8; 60];
        d[0] = byte as u8;
        d
    });
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        if let Some(p) = (*pt).get_mut(vga_pid) {
            if p.ipc_push(msg) {
                return 0;
            }
        }
    }
    -1
}

/// VGA_CLEAR: Clear the screen via the VGA display server.
///
/// If the VGA server is not yet running, falls back to kernel-side `vga::clear_screen()`.
fn sys_vga_clear(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    let vga_pid = unsafe { VGA_SERVER_PID };
    if vga_pid == 0 {
        vga::clear_screen();
        return 0;
    }
    // Forward to VGA server via IPC (msg_type = 2 = MSG_VGA_CLEAR)
    let msg = process::IpcMessage::new(0, 2, [0u8; 60]);
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        if let Some(p) = (*pt).get_mut(vga_pid) {
            if p.ipc_push(msg) {
                return 0;
            }
        }
    }
    -1
}

/// SCHED_YIELD: Cooperative scheduling — yield the CPU to another ready process.
///
/// The current process is moved back to the ready queue and `schedule_next()`
/// selects the next runnable process via round-robin. This only yields when
/// interrupts are enabled; preemptive scheduling happens through the timer IRQ.
fn sys_schedule(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    // The timer interrupt is the only path that currently owns a complete
    // saved user frame and can safely switch CR3/registers. Merely changing
    // CURRENT_PID here would return to this same caller while attributing its
    // next syscall to another process. Treat SCHEDULE as a safe hint/no-op;
    // preemption still happens on the next timer tick.
    0
}

/// GETPID: Return the PID of the calling process.
fn sys_getpid(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    process::get_current_pid() as isize
}

/// BRK: Adjust the process break (heap end) to `new_break`.
///
/// The heap lives in `[USER_HEAP_START, USER_HEAP_END)`. Pages are mapped on
/// demand (zero-filled) and unmapped on shrink. `new_break == 0` queries the
/// current break without changing it. Requests below the start clamp to the
/// start; requests past the end fail with -1. Returns the resulting break.
///
/// # Safety
///
/// Runs on the caller's live address space; growth maps fresh frames, shrink
/// unmaps plus `invlpg` so no stale translation survives.
fn sys_brk(new_break: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    use crate::paging::{
        alloc_frame, free_frame, invlpg, map_phys_page, munmap_range,
        page_flags, write_user_memory, USER_HEAP_END, USER_HEAP_START, PAGE_SIZE,
    };
    let current = process::get_current_pid();
    if current == 0 {
        return -1;
    }
    let (cr3, old_end) = unsafe {
        match process::PROCESS_TABLE.get(current) {
            Some(p) => (p.cr3, p.heap_end),
            None => return -1,
        }
    };
    if new_break == 0 {
        return old_end as isize; // query
    }
    let mut target = new_break as u64;
    if target < USER_HEAP_START {
        target = USER_HEAP_START;
    }
    if target > USER_HEAP_END {
        return -1;
    }
    let align_up = |v: u64| (v + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let old_page = align_up(old_end);
    let new_page = align_up(target);
    // Zero-fill in stack-friendly chunks (the kernel stack is one page).
    static ZEROS: [u8; 512] = [0; 512];
    if new_page > old_page {
        // Grow one page at a time; heap_end tracks the mapped high-water
        // mark so a mid-way OOM leaves a consistent state.
        let mut va = old_page;
        while va < new_page {
            let frame = alloc_frame();
            if frame == 0 {
                break;
            }
            let mapped = unsafe {
                map_phys_page(
                    cr3,
                    va,
                    frame,
                    page_flags::PRESENT
                        | page_flags::WRITABLE
                        | page_flags::USER_ACCESS
                        | page_flags::EXECUTABLE_DISABLE,
                )
            };
            if !mapped {
                free_frame(frame);
                break;
            }
            let mut off = 0u64;
            while off < PAGE_SIZE {
                if unsafe { write_user_memory(cr3, va + off, &ZEROS) }.is_err() {
                    break;
                }
                off += ZEROS.len() as u64;
            }
            if off < PAGE_SIZE {
                break;
            }
            va += PAGE_SIZE;
            unsafe {
                if let Some(p) = process::PROCESS_TABLE.get_mut(current) {
                    p.heap_end = va;
                }
            }
        }
        return unsafe {
            process::PROCESS_TABLE
                .get(current)
                .map(|p| p.heap_end as isize)
                .unwrap_or(-1)
        };
    } else if new_page < old_page {
        unsafe {
            munmap_range(cr3, new_page, old_page - new_page);
            let mut va = new_page;
            while va < old_page {
                invlpg(va);
                va += PAGE_SIZE;
            }
            if let Some(p) = process::PROCESS_TABLE.get_mut(current) {
                p.heap_end = target;
            }
        }
    }
    target as isize
}

/// GET_SERVER_PID: Return the PID of a well-known server process.
///
/// `server` selects which server to look up:
///   1 = VGA display server (PID 1 by convention)
///   2 = Keyboard input server (PID 0 by default, set via keyboard_irq_init)
///   3 = Interrupt server / IRQ router (PID 0 when user-space routing is off)
fn sys_get_server_pid(server: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    match server {
        1 => unsafe { VGA_SERVER_PID as isize },
        2 => unsafe { KEYBOARD_SERVER_PID as isize },
        3 => unsafe { INTERRUPT_SERVER_PID as isize },
        4 => unsafe { STORAGE_SERVER_PID as isize },
        _ => -1,
    }
}

// =============================================================================
// IPC SYSCALL HANDLERS
// =============================================================================

/// Check that every page touched by a user buffer is mapped for the requested
/// access in the *calling process's* CR3.
///
/// LESSON (safe copy boundary): a numeric range check alone is not sufficient.
/// The address may fall in an unmapped hole, cross into a supervisor page, or
/// wrap at `usize::MAX`. Walking each page-table leaf before touching it turns
/// those cases into a normal syscall error instead of a fault while CPL=0.
fn is_valid_user_range(ptr: usize, len: usize, write: bool) -> bool {
    let user_start = crate::paging::USER_VADDR_START as usize;
    let user_end = crate::paging::USER_VADDR_END as usize;
    let end = match ptr.checked_add(len) {
        Some(end) => end,
        None => return false,
    };
    if ptr < user_start || ptr >= user_end || end > user_end {
        return false;
    }
    if len == 0 {
        return true;
    }

    let current = process::get_current_pid();
    let cr3 = unsafe {
        let table = core::ptr::addr_of!(process::PROCESS_TABLE);
        match (*table).get(current) {
            Some(p) => p.cr3,
            None => return false,
        }
    };

    let mut page = (ptr as u64) & !0xFFF;
    let end = end as u64;
    while page < end {
        let page_size = match unsafe {
            crate::paging::resolve_user_page_access(cr3, page, write)
        } {
            Ok((_, size)) => size as u64,
            Err(()) => return false,
        };
        let next = (page & !(page_size - 1)).saturating_add(page_size);
        if next <= page {
            return false;
        }
        page = next;
    }
    true
}

/// Ask the kernel capability table whether the current process holds a
/// privileged operation right. User processes cannot forge this bitmap from
/// ring 3 because the PCB is in the supervisor-only identity map.
fn current_process_has_capability(cap: usize) -> bool {
    let current = process::get_current_pid();
    if current == 0 {
        return false;
    }
    unsafe {
        let table = core::ptr::addr_of!(process::PROCESS_TABLE);
        (*table)
            .get(current)
            .map(|p| crate::capability::has(p.capabilities, cap))
            .unwrap_or(false)
    }
}

/// IPC_SEND: Send a message to another process.
///
/// Arguments (syscall convention):
///   a1 = dst_pid   (target process PID)
///   a2 = msg_ptr   (user-space pointer to a 64-byte IpcMessage)
///
/// The kernel copies the message from the sender's address space into
/// the destination process's kernel-side IPC queue. The `src_pid` field
/// is overwritten with the current process's PID.
fn sys_ipc_send(dst_pid: usize, msg_ptr: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    // Validate destination PID
    if dst_pid == 0 || dst_pid > 0xFFFF {
        return -1;
    }
    // Validate the user-space pointer
    if !is_valid_user_range(msg_ptr, core::mem::size_of::<process::IpcMessage>(), false) {
        return -1;
    }

    let current = process::get_current_pid();
    if current == 0 {
        return -1;
    }

    // Phase 4 — Capability enforcement: the sender must hold a capability
    // for the destination PID's endpoint type. Denial is EACCES (-2) so
    // user space can distinguish it from malformed arguments (-1).
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        if let Some(sender) = (*pt).get_mut(current) {
            if !crate::capability::check_ipc_send(sender.capabilities, dst_pid as u16) {
                return EACCES; // No capability — rejected
            }
        }
    }

    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;

        if let Some(dst) = (*pt).get_mut(dst_pid as u16) {
            // Read the 64-byte message from user space
            let mut msg: process::IpcMessage = core::mem::zeroed();
            core::ptr::copy_nonoverlapping(
                msg_ptr as *const u8,
                &mut msg as *mut process::IpcMessage as *mut u8,
                core::mem::size_of::<process::IpcMessage>(),
            );
            // Set the sender PID (trusted kernel-side)
            msg.src_pid = current;

            // Push into destination's queue
            if dst.ipc_push(msg) {
                return 0;
            }
        }
    }
    -1
}

/// IPC_RECV: Receive a message from this process's IPC queue.
///
/// Arguments (syscall convention):
///   a1 = buf_ptr   (user-space pointer to a 64-byte buffer)
///
/// The kernel copies the oldest message from the current process's
/// kernel-side IPC queue into the user-provided buffer.
/// Returns 64 (bytes copied) on success, or -1 if the queue is empty.
fn sys_ipc_recv(buf_ptr: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    if !is_valid_user_range(buf_ptr, core::mem::size_of::<process::IpcMessage>(), true) {
        return -1;
    }

    let current = process::get_current_pid();

    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;

        if let Some(p) = (*pt).get_mut(current) {
            if let Some(msg) = p.ipc_pop() {
                // Write the message to user space
                core::ptr::copy_nonoverlapping(
                    &msg as *const process::IpcMessage as *const u8,
                    buf_ptr as *mut u8,
                    core::mem::size_of::<process::IpcMessage>(),
                );
                return core::mem::size_of::<process::IpcMessage>() as isize;
            }
        }
    }
    -1
}

// =============================================================================
// PHASE 2 — PROCESS CREATION SYSCALLS
// =============================================================================

/// FORK: Create a child process as a copy of the current one.
///
/// Returns the child's PID on success, or -1 on failure (table full).
fn sys_fork(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    let current = process::get_current_pid();
    if current == 0 || !current_process_has_capability(crate::capability::cap_id::FORK) {
        return -1; // kernel can't fork
    }
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        match (*pt).fork_process(current) {
            Some(child_pid) => {
                crate::vga::serial_print("fork: pid->");
                child_pid as isize
            }
            None => -1,
        }
    }
}

// =============================================================================
// PHASE 4 — SYSTEM HARDENING SYSCALLS
// =============================================================================

/// REGISTER_IRQ: Register a user-space IRQ handler.
///
/// a1 = IRQ vector number (e.g. 0x21 for keyboard)
/// a2 = handler PID (-1 = self)
fn sys_register_irq(vector: usize, handler_pid: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    if vector >= 256
        || !current_process_has_capability(crate::capability::cap_id::IRQ_REGISTER)
    {
        return -1;
    }
    let pid = if handler_pid == !0usize {
        process::get_current_pid()
    } else {
        handler_pid as u16
    };
    // Register with interrupt server
    crate::interrupt_server::register_handler(vector as u8, pid);
    crate::vga::serial_print("IRQ registered\n");
    0
}

/// CAP_SEND: Check capability before sending an IPC message.
///
/// a1 = destination PID
/// Returns 0 if the current process has the capability to send to that PID,
/// -1 otherwise.
fn sys_cap_send(dst_pid: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    let current = process::get_current_pid();
    if current == 0 || dst_pid > u16::MAX as usize {
        return -1;
    }
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        if let Some(p) = (*pt).get_mut(current) {
            if crate::capability::check_ipc_send(p.capabilities, dst_pid as u16) {
                return 0;
            }
        }
    }
    -1
}

/// GRANT_CAP: Delegate a held capability bit to another process.
///
/// a1 = destination PID, a2 = capability bit (0..63).
/// The caller must hold the bit itself (see `capability::can_grant`).
/// Returns 0 on success, -1 on malformed arguments or unknown PID,
/// EACCES (-2) when the caller does not hold the bit.
fn sys_grant_cap(dst_pid: usize, cap: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    let current = process::get_current_pid();
    if current == 0 || dst_pid == 0 || dst_pid > u16::MAX as usize {
        return -1;
    }
    if cap >= crate::capability::MAX_CAPABILITIES {
        return -1;
    }
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        let caller_caps = match (*pt).get(current) {
            Some(p) => p.capabilities,
            None => return -1,
        };
        if !crate::capability::can_grant(caller_caps, cap) {
            return EACCES;
        }
        match (*pt).get_mut(dst_pid as u16) {
            Some(dst) => {
                crate::capability::grant(&mut dst.capabilities, cap);
                0
            }
            None => -1,
        }
    }
}

/// KILL: Send a signal to another process.
///
/// a1 = target PID, a2 = signal number (0..63, see `signal::sig_num`).
/// Returns 0 once the signal bit is set, -1 on unknown PID or bad signal.
///
/// # Policy
///
/// Any user process may signal any other in this phase (open policy,
/// documented here rather than hidden): `SIGSTOP`/`SIGCONT`/`SIGTERM` are
/// the shell's job-control tools, and capabilities already gate IPC.
/// A per-sender kill policy can replace this check without touching callers.
fn sys_kill(pid: usize, sig: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    if pid == 0 || pid > u16::MAX as usize || sig >= crate::signal::MAX_SIGNALS {
        return -1;
    }
    let target = pid as u16;
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        if (*pt).get(target).is_none() {
            return -1;
        }
        crate::vga::serial_print("kill: pid=");
        crate::vga::serial_print_hex("", target as u64);
        crate::vga::serial_print(" sig=");
        crate::vga::serial_print_hex("", sig as u64);
        crate::vga::serial_print("\n");
        crate::signal::send_signal(target, sig);
    }
    0
}

/// EXEC: Replace this process's image with the ELF at a user buffer.
///
/// a1 = buffer pointer, a2 = byte length (1..=256 KiB).
/// The caller must hold the EXEC capability. On success this never returns:
/// the address space is swapped, the PCB registers/stack/heap/signals are
/// reset, and `sysretq` resumes the new entry point with a fresh stack.
/// Returns -1 on bad image/range/OOM (current image keeps running),
/// EACCES (-2) without the EXEC capability.
///
/// # Safety
///
/// Reads the caller's own (live) address space for the ELF bytes; builds the
/// new space before touching the old one; the kernel identity map survives
/// the CR3 switch, and interrupts are masked on syscall entry.
fn sys_exec(buf_ptr: usize, len: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    const MAX_EXEC_BYTES: usize = 256 * 1024;
    if len == 0 || len > MAX_EXEC_BYTES {
        return -1;
    }
    if !is_valid_user_range(buf_ptr, len, false) {
        return -1;
    }
    if !current_process_has_capability(crate::capability::cap_id::EXEC) {
        return EACCES;
    }
    let current = process::get_current_pid();
    if current == 0 {
        return -1;
    }
    // Borrow the ELF bytes from the caller's live mapping (single-threaded:
    // no one else can mutate them under us).
    let image: &[u8] =
        unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };

    // Build the replacement address space first; the old one stays live.
    let new_cr3 = match crate::paging::create_address_space(None) {
        Some(cr3) => cr3,
        None => return -1,
    };
    let rollback = |cr3: u64| unsafe {
        crate::paging::free_user_address_space(cr3);
    };
    let loaded = unsafe { crate::elf::load_elf(image, new_cr3) };
    let loaded = match loaded {
        Ok(l) => l,
        Err(_) => {
            rollback(new_cr3);
            return -1;
        }
    };
    if !crate::paging::map_user_stack(new_cr3) {
        rollback(new_cr3);
        return -1;
    }
    // VGA alias parity with spawn_from_elf (every process gets it).
    let vga_ok = unsafe {
        crate::paging::map_phys_page(
            new_cr3,
            crate::paging::VGA_BUFFER_VADDR,
            crate::paging::VGA_PHYS_ADDR,
            crate::paging::page_flags::PRESENT
                | crate::paging::page_flags::WRITABLE
                | crate::paging::page_flags::USER_ACCESS
                | crate::paging::page_flags::CACHE_DISABLE
                | crate::paging::page_flags::EXECUTABLE_DISABLE,
        )
    };
    if !vga_ok {
        rollback(new_cr3);
        return -1;
    }

    // Commit: swap address spaces, reset PCB state, rewrite the syscall
    // return frame. From here on nothing touches the old image.
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        let old_cr3 = match (*pt).get(current) {
            Some(p) => p.cr3,
            None => {
                rollback(new_cr3);
                return -1;
            }
        };
        if let Some(p) = (*pt).get_mut(current) {
            p.cr3 = new_cr3;
            p.entry_point = loaded.entry;
            p.registers = process::ProcessRegisters::default();
            p.registers.rip = loaded.entry;
            p.registers.rsp = loaded.stack_top;
            p.heap_end = crate::paging::USER_HEAP_START;
            p.signal_pending = 0;
            p.signal_handlers = [None; 64];
            // Capabilities and I/O grants identify the sandbox, not the
            // image: they survive exec by design.
        }
        // Kernel identity mappings exist in every space, so the stack and
        // statics stay valid across the switch.
        core::arch::asm!("mov cr3, {}", in(reg) new_cr3, options(nostack));
        crate::paging::free_user_address_space(old_cr3);
        // Rewrite the saved sysret state captured at entry: K points at the
        // 8 pushed regs whose lowest slots are the last-pushed RCX (return
        // RIP at K+0) and R11 (return RFLAGS at K+8) — see syscall_entry's
        // push/pop order. RSP for sysret comes from USER_RSP_SAVE.
        let k = SYSCALL_FRAME_SAVE;
        core::ptr::write_unaligned(k as *mut u64, loaded.entry);
        core::ptr::write_unaligned((k + 8) as *mut u64, 0x202);
        crate::tss::USER_RSP_SAVE = loaded.stack_top;
        crate::vga::serial_print("exec: new rip=");
        crate::vga::serial_print_hex("", loaded.entry);
        crate::vga::serial_print("\n");
    }
    0
}

/// REGISTER_SIGNAL: Register a signal handler function.
///
/// a1 = signal number (0-63)
/// a2 = handler address (0 = reset to default / ignore)
fn sys_register_signal(sig: usize, handler: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    if sig >= 64 {
        return -1;
    }
    let current = process::get_current_pid();
    if current == 0 {
        return -1;
    }
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        if let Some(p) = (*pt).get_mut(current) {
            if handler == 0 {
                p.signal_handlers[sig] = None;
            } else {
                p.signal_handlers[sig] = Some(handler as u64);
            }
            return 0;
        }
    }
    -1
}

/// WAIT_SIGNAL: Block the process until a signal arrives (or poll).
///
/// a1 = timeout in ticks (0 = poll, returns immediately)
/// Returns the signal number on success, or -1 if no signals pending.
fn sys_wait_signal(timeout: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    let current = process::get_current_pid();
    if current == 0 {
        return -1;
    }
    // For now, simple poll — check pending signals
    unsafe {
        let pt = &raw mut process::PROCESS_TABLE;
        if let Some(p) = (*pt).get_mut(current) {
            if p.signal_pending != 0 {
                // Find and return the first pending signal
                let sig = p.signal_pending.trailing_zeros() as usize;
                p.signal_pending &= !(1u64 << sig);
                return sig as isize;
            }
            // Until syscall-return context switching is implemented, this
            // syscall is an explicit poll. Do not mark the caller BLOCKED here:
            // SYSCALL returns to the same user instruction stream and cannot
            // switch processes by changing only CURRENT_PID. `timeout` is
            // reserved for the future blocking ABI; callers get -1 on no signal.
            let _ = timeout;
            return -1;
        }
    }
    -1
}

// =============================================================================
// SYSCALL DISPATCH (called from assembly entry)
// =============================================================================

/// Called by `syscall_entry` assembly with correct register mapping.
///
/// # Safety
///
/// Called from assembly with user-supplied arguments.
#[no_mangle]
pub unsafe extern "C" fn syscall_dispatch(
    nr: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
) -> isize {
    if nr < 32 {
        let handler = SYSCALL_TABLE.handlers[nr];
        if let Some(h) = handler {
            return h(a1, a2, a3, a4, a5, nr);
        }
    }
    -1
}

// =============================================================================
// ASSEMBLY ENTRY POINTS
// =============================================================================

core::arch::global_asm!(
    // =========================================================================
    // SYSCALL ENTRY POINT
    // =========================================================================
    //
    // This is where the `syscall` instruction jumps (via LSTAR MSR).
    // On entry:
    //   rax = syscall number
    //   rdi = arg1   rsi = arg2   rdx = arg3   r10 = arg4   r8 = arg5
    //   RCX = user RIP (return address, saved by CPU)
    //   R11 = user RFLAGS (saved by CPU)
    //
    // We must:
    //   1. Save user RSP and switch to kernel stack (syscall doesn't do this)
    //   2. Save all GP registers and RCX/R11
    //   3. Rearrange args for C calling convention (rdi, rsi, rdx, rcx, r8, r9)
    //   4. Call syscall_dispatch
    //   5. Restore RCX and R11 for sysretq, keep return value in RAX
    //   6. Restore user RSP and sysretq
    ".globl syscall_entry",
    ".type syscall_entry, @function",
    "syscall_entry:",
    // Save user RSP in global, load kernel stack
    "    mov [rip + USER_RSP_SAVE], rsp",
    "    mov rsp, [rip + CURRENT_KERNEL_RSP]",
    "",
    // Save callee-saved registers plus RCX/R11 (needed for sysretq)
    "    push r15",
    "    push r14",
    "    push r13",
    "    push r12",
    "    push rbp",
    "    push rbx",
    "    push r11",            // Saved RFLAGS (for sysretq)
    "    push rcx",            // Saved RIP (for sysretq)
    // 8 pushes = 64 bytes. 64 % 16 = 0. Stack is aligned. ✓
    "",
    // Map from syscall convention to C calling convention:
    //   syscall: rax=nr, rdi=a1, rsi=a2, rdx=a3, r10=a4, r8=a5
    //   C call:  rdi=nr, rsi=a1, rdx=a2, rcx=a3, r8=a4,  r9=a5
    "    mov r9, r8",
    "    mov r8, r10",
    "    mov rcx, rdx",
    "    mov rdx, rsi",
    "    mov rsi, rdi",
    "    mov rdi, rax",
    "",
    // Snapshot the entry RSP (points at the 8 pushed regs) so sys_exec
    // can rewrite the saved RCX/RFLAGS return slots below. Push order is
    // r15..rcx, so rcx sits at the LOWEST address: RCX slot = K+0 (saved
    // RIP), R11 slot = K+8 (saved RFLAGS) — matching the pop order below.
    "    mov [rip + SYSCALL_FRAME_SAVE], rsp",
    "    call syscall_dispatch",
    "",
    // Restore callee-saved regs (reverse order)
    "    pop rcx",             // User RIP (for sysretq)
    "    pop r11",             // User RFLAGS (for sysretq)
    "    pop rbx",
    "    pop rbp",
    "    pop r12",
    "    pop r13",
    "    pop r14",
    "    pop r15",
    "",
    // Restore user RSP and return
    "    mov rsp, [rip + USER_RSP_SAVE]",
    "    sysretq",
    "",
    // =========================================================================
    // TIMER INTERRUPT ENTRY POINT
    // =========================================================================
    //
    // This is called when the PIT timer fires (IRQ0, vector 0x20) while
    // the CPU is in user mode (ring 3). Before this runs, the CPU has:
    //   1. Read RSP0 from TSS → switched to kernel stack
    //   2. Pushed SS, user RSP, RFLAGS, CS, RIP onto kernel stack
    //   3. Jumped here via IDT
    //
    // We must:
    //   1. Save all GP registers after the CPU-pushed interrupt frame
    //   2. Call timer_save_and_switch (handles scheduling)
    //   3. Restore all GP registers
    //   4. iretq (may jump to a different process)
    ".globl timer_entry",
    ".type timer_entry, @function",
    "timer_entry:",
    // Save all GP registers on the kernel stack
    "    push rax",
    "    push rcx",
    "    push rdx",
    "    push rbx",
    "    push rbp",
    "    push rsi",
    "    push rdi",
    "    push r8",
    "    push r9",
    "    push r10",
    "    push r11",
    "    push r12",
    "    push r13",
    "    push r14",
    "    push r15",
    // 15 pushes = 120 bytes. 120 % 16 = 8. Need alignment before call.
    // But we also need to account for the CPU-pushed interrupt frame
    // (5 items = 40 bytes). 40 + 120 = 160. 160 % 16 = 0. RSP is aligned. ✓
    // Total from kernel stack top: 160 bytes. Remaining: 4096 - 160 = 3936. ✓
    "",
    // Pass RSP as arg0 (pointer to saved r15)
    "    mov rdi, rsp",
    "    call timer_save_and_switch",
    "",
    // Restore GP registers (order may have been modified by scheduler)
    "    pop r15",
    "    pop r14",
    "    pop r13",
    "    pop r12",
    "    pop r11",
    "    pop r10",
    "    pop r9",
    "    pop r8",
    "    pop rdi",
    "    pop rsi",
    "    pop rbp",
    "    pop rbx",
    "    pop rdx",
    "    pop rcx",
    "    pop rax",
    // RSP now points to interrupt frame (RIP, CS, RFLAGS, RSP, SS)
    // which may have been modified for a different process
    "    iretq",
    "",
    // =========================================================================
    // KEYBOARD INTERRUPT ENTRY POINT (IRQ1, vector 0x21)
    // =========================================================================
    //
    // The PS/2 keyboard controller generates IRQ1 every time a key is pressed
    // or released. The PIC (Programmable Interrupt Controller) delivers this
    // as interrupt vector 0x21 (after our PIC remap in main.rs).
    //
    // When this interrupt fires while the CPU is in ring 3 (user mode):
    //   1. CPU reads RSP0 from the TSS → switches to kernel stack
    //   2. CPU pushes SS, user RSP, RFLAGS, CS, RIP onto kernel stack
    //   3. CPU jumps here via IDT entry 0x21 (CS = 0x08 = ring 0 code)
    //
    // We must:
    //   1. Save all 15 general-purpose registers
    //   2. Call keyboard_irq_handler() — reads port 0x60, forwards via IPC
    //   3. Restore all GP registers
    //   4. iretq back to user mode (to whatever process was running)
    //
    // Why doesn't the IRQ handler read port 0x60 itself?
    //   The PS/2 controller has a 1-byte input buffer. Reading it from the
    //   handler would be fastest, but it keeps device I/O inside the kernel.
    //   Instead the user-space keyboard server owns the port (TSS grant for
    //   0x60/0x64, installed at spawn) and drains it when this IRQ notifies
    //   it. Port I/O (in/out) from ring 3 is possible exactly because of
    //   that per-process bitmap — no kernel mediation per keystroke.
    ".globl keyboard_entry",
    ".type keyboard_entry, @function",
    "keyboard_entry:",
    // Save all 15 GP registers on the kernel stack (same order as timer_entry)
    "    push rax",
    "    push rcx",
    "    push rdx",
    "    push rbx",
    "    push rbp",
    "    push rsi",
    "    push rdi",
    "    push r8",
    "    push r9",
    "    push r10",
    "    push r11",
    "    push r12",
    "    push r13",
    "    push r14",
    "    push r15",
    // Pass RSP as argument to the Rust handler (points to saved r15)
    // The handler reads scancode from port 0x60 and pushes IPC message
    "    mov rdi, rsp",
    "    call keyboard_irq_handler",
    // Restore GP registers (reverse order)
    "    pop r15",
    "    pop r14",
    "    pop r13",
    "    pop r12",
    "    pop r11",
    "    pop r10",
    "    pop r9",
    "    pop r8",
    "    pop rdi",
    "    pop rsi",
    "    pop rbp",
    "    pop rbx",
    "    pop rdx",
    "    pop rcx",
    "    pop rax",
    // RSP now points to interrupt frame: RIP, CS, RFLAGS, RSP, SS
    // iretq pops these and returns to ring 3
    "    iretq",

    // =========================================================================
    // #PF (PAGE FAULT, vector 0x0E)
    // =========================================================================
    // Unlike timer_entry, the CPU also pushes a page-fault error code. After the
    // 15 GP-register pushes the frame is:
    //   [0..14] saved GP regs, [15] error, [16] RIP, [17] CS,
    //   [18] RFLAGS, [19] user RSP, [20] user SS.
    // The extra error-code word changes call alignment by eight bytes.
    ".globl pf_entry",
    ".type pf_entry, @function",
    "pf_entry:",
    "    push rax",
    "    push rcx",
    "    push rdx",
    "    push rbx",
    "    push rbp",
    "    push rsi",
    "    push rdi",
    "    push r8",
    "    push r9",
    "    push r10",
    "    push r11",
    "    push r12",
    "    push r13",
    "    push r14",
    "    push r15",
    "    mov rdi, rsp",
    "    sub rsp, 8", // align the C ABI call; original frame remains in RDI
    "    call page_fault_save_and_switch",
    "    add rsp, 8",
    "    pop r15",
    "    pop r14",
    "    pop r13",
    "    pop r12",
    "    pop r11",
    "    pop r10",
    "    pop r9",
    "    pop r8",
    "    pop rdi",
    "    pop rsi",
    "    pop rbp",
    "    pop rbx",
    "    pop rdx",
    "    pop rcx",
    "    pop rax",
    "    add rsp, 8", // discard CPU-pushed error code before iretq
    "    iretq",

    // =========================================================================
    // #GP (GENERAL PROTECTION FAULT) ENTRY POINT (vector 0x0D)
    // =========================================================================
    //
    // Called when the CPU detects a protection violation. Has an error code.
    //
    // On entry (ring 3 → ring 0, CPU has switched stacks via TSS.RSP0):
    //   RSP+0: error code
    //   RSP+8: RIP
    //   RSP+16: CS
    //   RSP+24: RFLAGS
    //   RSP+32: user RSP
    //   RSP+40: user SS
    //
    // We pass RSP (the frame pointer) to the Rust handler and halt.
    ".globl gp_entry",
    ".type gp_entry, @function",
    "gp_entry:",
    "    mov rdi, rsp",
    "    call gp_handler",
    "1:  hlt",
    "    jmp 1b",

    // =========================================================================
    // DISK IRQ IGNORE STUBS (IRQ14/IRQ15, vectors 0x2E/0x2F)
    // =========================================================================
    //
    // The ATA disk raises IRQ14 (primary) / IRQ15 (secondary) when a command
    // completes. SeaBIOS leaves those PIC lines unmasked, so the interrupt
    // arrives even though no IDT entry was installed — vectoring into a null
    // descriptor raises #GP with error (vector<<3)|2 (e.g. 0x172 for 0x2E).
    //
    // The user-space storage driver polls the status register instead of
    // waiting for interrupts, so the correct handling here is acknowledge +
    // ignore: send EOI to the slave (0xA0) and master (0x20) PICs and return.
    // A later phase may forward these IRQs to the storage server via IPC.
    ".globl irq14_entry",
    ".type irq14_entry, @function",
    "irq14_entry:",
    "    push rax",
    "    mov al, 0x20",
    "    out 0xA0, al",   // EOI slave PIC (IRQ8-15 live here)
    "    out 0x20, al",   // EOI master PIC (cascade)
    "    pop rax",
    "    iretq",
    ".globl irq15_entry",
    ".type irq15_entry, @function",
    "irq15_entry:",
    "    push rax",
    "    mov al, 0x20",
    "    out 0xA0, al",
    "    out 0x20, al",
    "    pop rax",
    "    iretq",

    // =========================================================================
    // DOUBLE FAULT ENTRY POINT (vector 0x08)
    // =========================================================================
    //
    // Called when the CPU encounters a fault while trying to deliver another
    // exception. Uses IST1 (Interrupt Stack Table) so it runs on a known-good
    // stack regardless of kernel-stack state.
    //
    // Same stack layout as #GP (error code + interrupt frame).
    ".globl df_entry",
    ".type df_entry, @function",
    "df_entry:",
    "    mov rdi, rsp",
    "    call df_handler",
    "1:  hlt",
    "    jmp 1b",
);

extern "C" {
    static syscall_entry: u8;
    static timer_entry: u8;
    static keyboard_entry: u8;
    static pf_entry: u8;
    static gp_entry: u8;
    static irq14_entry: u8;
    static irq15_entry: u8;
    static df_entry: u8;
}

// =============================================================================
// SYSCALL MSR INITIALIZATION
// =============================================================================

/// Configure the CPU for `syscall`/`sysretq`.
///
/// Sets up three Model-Specific Registers:
///
/// STAR (0xC0000081)
///   [47:32] = 0x08  Kernel CS for SYSCALL entry  → CS=0x08 (R0 code), SS=0x10 (R0 data)
///   [63:48] = 0x18  SYSRET base                  → CS=0x28 (R3 code), SS=0x20 (R3 data)
///                                                   (RPL forced to 3 by CPU)
///
/// LSTAR (0xC0000082) = address of `syscall_entry`
///
/// SFMASK (0xC0000084) = 0x200  Clear IF (bit 9) on syscall entry so we
///                               handle syscalls with interrupts disabled.
///
/// # Safety
///
/// Writes to MSRs. Must be called once during boot, before any `syscall`.
pub unsafe fn init_syscall() {
    // STAR[47:32] = 0x08 (syscall CS), [63:48] = 0x18 (sysret base)
    let star = (0x08u64 << 32) | (0x18u64 << 48);
    wrmsr(0xC0000081, star);

    // LSTAR = address of syscall entry point
    let lstar = &syscall_entry as *const u8 as u64;
    wrmsr(0xC0000082, lstar);

    // SFMASK = mask IF bit (RFLAGS bit 9) on syscall entry
    wrmsr(0xC0000084, 0x200);

    // EFER (0xC0000080): Set SCE (bit 0) to enable syscall/sysret.
    // Without this, `syscall` raises #UD (undefined instruction).
    let efer = rdmsr(0xC0000080);
    wrmsr(0xC0000080, efer | 1);
}

/// Read a 64-bit value from an x86 MSR.
unsafe fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nostack)
    );
    (high as u64) << 32 | low as u64
}

/// Write a 64-bit value to an x86 MSR.
unsafe fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") low,
        in("edx") high,
        options(nostack)
    );
}

// =============================================================================
// TIMER CONTEXT SWITCH
// =============================================================================

/// Save the current process's registers, schedule the next process, and
/// load its registers — all through the stack frame created by `timer_entry`.
///
/// `frame` points to the saved-r15 slot on the kernel stack. The layout is:
///
///   frame[0]  = r15        frame[10] = rbp
///   frame[1]  = r14        frame[11] = rbx
///   frame[2]  = r13        frame[12] = rdx
///   frame[3]  = r12        frame[13] = rcx
///   frame[4]  = r11        frame[14] = rax
///   frame[5]  = r10        frame[15] = RIP  (from CPU interrupt frame)
///   frame[6]  = r9         frame[16] = CS
///   frame[7]  = r8         frame[17] = RFLAGS
///   frame[8]  = rdi        frame[18] = user RSP
///   frame[9]  = rsi        frame[19] = SS
///
/// # Safety
///
/// Called from `timer_entry` assembly. `frame` must point to valid kernel stack
/// memory with the expected layout.
#[no_mangle]
pub unsafe extern "C" fn timer_save_and_switch(frame: *mut u64) {
    use crate::process::ProcessState;

    // Ring-0 re-entrancy guard: a tick that interrupts ring-0 code (syscall
    // path, another handler) pushes only a 3-item kernel frame, not the
    // 5-item user frame below. Saving it as user state would corrupt the
    // PCB and switching CR3 would strand the kernel stack — acknowledge
    // and resume without touching anything.
    if *frame.add(16) & 3 != 3 {
        core::arch::asm!("mov al, 0x20", "out 0x20, al", options(nostack));
        return;
    }

    // This function is called from the real IDT `timer_entry`; update the
    // monotonic clock here rather than in an unused parallel handler.
    crate::timer_tick();

    let current = process::get_current_pid();

    let table = core::ptr::addr_of_mut!(process::PROCESS_TABLE);

    // === 1. Save current process state ===
    if current != 0 {
        if let Some(p) = (*table).get_mut(current) {
            p.registers.rax    = *frame.add(14);
            p.registers.rbx    = *frame.add(11);
            p.registers.rcx    = *frame.add(13);
            p.registers.rdx    = *frame.add(12);
            p.registers.rsi    = *frame.add(9);
            p.registers.rdi    = *frame.add(8);
            p.registers.rbp    = *frame.add(10);
            p.registers.rsp    = *frame.add(18);
            p.registers.r8     = *frame.add(7);
            p.registers.r9     = *frame.add(6);
            p.registers.r10    = *frame.add(5);
            p.registers.r11    = *frame.add(4);
            p.registers.r12    = *frame.add(3);
            p.registers.r13    = *frame.add(2);
            p.registers.r14    = *frame.add(1);
            p.registers.r15    = *frame.add(0);
            p.registers.rip    = *frame.add(15);
            p.registers.rflags = *frame.add(17);

            if p.state == ProcessState::Running {
                p.state = ProcessState::Ready;
            }
        }
    }

    // === 2. Send EOI to PIC ===
    core::arch::asm!("mov al, 0x20", "out 0x20, al", options(nostack));

    // === 3. Pick next process ===
    process::schedule_next();

    // === 4. Load the next process into the interrupt frame. ===
    // Deliver signals before copying registers: delivery may change RIP/RSP.
    // Also release the PCB borrow before the signal code mutates the process table.
    loop {
        let next = process::get_current_pid();
        if next == 0 {
            loop { core::arch::asm!("cli", "hlt", options(nostack)); }
        }
        crate::signal::deliver_pending(next);

        let next_state = if let Some(p) = (*table).get_mut(next) {
            // Terminated and Stopped processes are never resumed here:
            // Terminated slots are dead, Stopped ones wait for SIGCONT
            // (the scheduler never picks them; this also covers a stop
            // that `deliver_pending` just applied above).
            if p.state == ProcessState::Terminated || p.state == ProcessState::Stopped {
                None
            } else {
                p.state = ProcessState::Running;
                Some((p.registers, p.cr3, p.kernel_stack_top, p.io_port_ranges, p.io_port_range_count))
            }
        } else {
            None
        };
        let (regs, cr3, kernel_stack_top, io_ranges, io_range_count) = match next_state {
            Some(state) => state,
            None => {
                process::schedule_next();
                continue;
            }
        };

        *frame.add(14) = regs.rax;
        *frame.add(11) = regs.rbx;
        *frame.add(13) = regs.rcx;
        *frame.add(12) = regs.rdx;
        *frame.add(9) = regs.rsi;
        *frame.add(8) = regs.rdi;
        *frame.add(10) = regs.rbp;
        *frame.add(18) = regs.rsp;
        *frame.add(7) = regs.r8;
        *frame.add(6) = regs.r9;
        *frame.add(5) = regs.r10;
        *frame.add(4) = regs.r11;
        *frame.add(3) = regs.r12;
        *frame.add(2) = regs.r13;
        *frame.add(1) = regs.r14;
        *frame.add(0) = regs.r15;
        *frame.add(15) = regs.rip;
        *frame.add(16) = 0x2B;   // ring-3 code selector (GDT 0x28 | RPL3)
        *frame.add(17) = regs.rflags;
        *frame.add(18) = regs.rsp;
        *frame.add(19) = 0x23;   // ring-3 data selector (GDT 0x20 | RPL3)

        // Kernel identity mappings are supervisor-only and present in every
        // process; changing CR3 therefore preserves the active kernel stack.

        // Kernel identity mappings are supervisor-only and present in every
        // process; changing CR3 therefore preserves the active kernel stack.
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
        crate::tss::TSS.set_rsp0(kernel_stack_top);
        crate::tss::TSS.set_io_port_ranges(&io_ranges[..io_range_count]);
        crate::tss::CURRENT_KERNEL_RSP = kernel_stack_top;
        break;
    }
}

/// Resolve a user COW fault or terminate the faulting process and resume the
/// next runnable process through the same hardware interrupt frame.
///
/// # Safety
/// `frame` is the stack pointer supplied by `pf_entry`: saved r15 starts at 0,
/// the CPU page-fault error code is at 15, and the iret frame starts at 16.
#[no_mangle]
pub unsafe extern "C" fn page_fault_save_and_switch(frame: *mut u64) {
    let error = *frame.add(15);
    let cs = *frame.add(17);
    if cs & 3 != 3 {
        // A kernel page fault indicates a kernel bug. Do not try to return to an
        // unknown kernel instruction with corrupted state.
        crate::vga::serial_print("kernel page fault at ");
        let fault: u64;
        core::arch::asm!("mov {}, cr2", out(reg) fault, options(nostack));
        crate::vga::serial_print_hex("", fault);
        loop {
            core::arch::asm!("cli", "hlt", options(nostack));
        }
    }

    let fault_address: u64;
    core::arch::asm!("mov {}, cr2", out(reg) fault_address, options(nostack));
    let current = process::get_current_pid();
    if current == 0 {
        loop {
            core::arch::asm!("cli", "hlt", options(nostack));
        }
    }

    let table = core::ptr::addr_of_mut!(process::PROCESS_TABLE);
    let current_cr3 = match (*table).get(current) {
        Some(p) => p.cr3,
        None => 0,
    };

    // Page-fault error bits: P=bit0, W/R=bit1, U/S=bit2. Only a user write
    // protection fault can be a COW fault; all other user faults kill that
    // process rather than executing an invalid mapping in ring 0.
    if current_cr3 != 0 && error & 0b111 == 0b111
        && crate::paging::handle_cow_fault(current_cr3, fault_address)
    {
        return; // mapping is writable now; assembly restores this frame and iretq
    }

    if let Some(p) = (*table).get_mut(current) {
        p.state = process::ProcessState::Terminated;
    }
    crate::vga::serial_print("user process fault; terminating pid ");
    crate::vga::serial_print_hex("", current as u64);

    // The faulting process is not resumed. Try each runnable process until one
    // is still alive after pending-signal delivery.
    loop {
        process::schedule_next();
        let next = process::get_current_pid();
        if next == 0 {
            loop {
                core::arch::asm!("cli", "hlt", options(nostack));
            }
        }
        crate::signal::deliver_pending(next);
        let state = match (*table).get_mut(next) {
            Some(p)
                if p.state != process::ProcessState::Terminated
                    && p.state != process::ProcessState::Stopped =>
            {
                p.state = process::ProcessState::Running;
                Some((p.registers, p.cr3, p.kernel_stack_top, p.io_port_ranges, p.io_port_range_count))
            }
            _ => None,
        };
        if let Some((regs, cr3, kernel_stack_top, io_ranges, io_range_count)) = state {
            // Page-fault frames include the error word at slot 15. The assembly
            // discards it after restoring GP registers, so the iret frame starts
            // at slot 16 instead of timer_entry's slot 15.
            *frame.add(14) = regs.rax;
            *frame.add(11) = regs.rbx;
            *frame.add(13) = regs.rcx;
            *frame.add(12) = regs.rdx;
            *frame.add(9) = regs.rsi;
            *frame.add(8) = regs.rdi;
            *frame.add(10) = regs.rbp;
            *frame.add(19) = regs.rsp;
            *frame.add(7) = regs.r8;
            *frame.add(6) = regs.r9;
            *frame.add(5) = regs.r10;
            *frame.add(4) = regs.r11;
            *frame.add(3) = regs.r12;
            *frame.add(2) = regs.r13;
            *frame.add(1) = regs.r14;
            *frame.add(0) = regs.r15;
            *frame.add(16) = regs.rip;
            *frame.add(17) = 0x2B; // ring-3 code selector
            *frame.add(18) = regs.rflags;
            *frame.add(20) = 0x23; // ring-3 data selector
            core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
            crate::tss::TSS.set_rsp0(kernel_stack_top);
            crate::tss::TSS.set_io_port_ranges(&io_ranges[..io_range_count]);
            crate::tss::CURRENT_KERNEL_RSP = kernel_stack_top;
            return;
        }
        if let Some(p) = (*table).get_mut(next) {
            p.state = process::ProcessState::Terminated;
        }
    }
}

// =============================================================================
// KEYBOARD IRQ HANDLER
// =============================================================================
//
// This is called from the keyboard_entry assembly when IRQ1 fires.
// Its job is minimal by design (microkernel philosophy):
//
//   1. Forward the IRQ notification to the user-space keyboard server via IPC
//   2. Send End-Of-Interrupt to the PIC
//
// The PS/2 data port (0x60) is deliberately NOT read here. The keyboard
// server owns that port through its TSS I/O grant and drains the scancode
// itself from ring 3 when the notification arrives — the last piece of
// device I/O moves out of the kernel (phase A1).
//
// This is the classic microkernel pattern: the kernel handles the
// time-critical notification, but the driver logic lives in a
// user-space process that could crash without bringing down the system.

/// Forward the keyboard IRQ to the keyboard server via IPC. Called from
/// `keyboard_entry` assembly (IRQ1).
///
/// A1 purification: the kernel no longer touches the PS/2 data port. The
/// scancode stays in the controller's 1-byte buffer until the user-space
/// keyboard server reads port 0x60 itself (it holds a TSS I/O grant for
/// 0x60/0x64, installed at spawn). This handler only forwards the IRQ
/// notification and the PIC EOI.
///
/// # Accepted race
///
/// If a second keystroke arrives before the server drains port 0x60, the
/// first byte is lost. At human typing speed the server (woken by this very
/// notification) always wins; only a synthetic key-burst faster than a
/// schedule slice could drop a byte.
#[no_mangle]
pub unsafe extern "C" fn keyboard_irq_handler() {
    // ── Step 1: notify, don't read ──
    // Empty payload: the scancode is NOT here. The keyboard server reads
    // port 0x60 from ring 3 when this notification arrives (via the ISR as
    // MSG_KEY_SCANCODE, or directly when user-space routing is off).
    {
        let data = [0u8; 60];
        crate::interrupt_server::forward_irq(1 /* keyboard IRQ */, &data);
    }

    // ── Step 2: Send End-Of-Interrupt to the master PIC ──
    // Writing 0x20 to port 0x20 tells the 8259A PIC that we've finished
    // handling this interrupt. Without this, the PIC won't deliver any
    // more interrupts (including timer ticks — the system freezes).
    core::arch::asm!("mov al, 0x20", "out 0x20, al", options(nostack));
}

// =============================================================================
// EXCEPTION HANDLERS (diagnostic)
// =============================================================================

/// #GP handler — writes error code to VGA at known position, then halts.
///
/// We write directly to VGA memory (0xB800A+) instead of using serial,
/// because serial output may not be reliable at the point of fault.
/// The error code can be read from the QEMU monitor after the crash:
///   `xp /16bx 0xB800A`
///
/// `frame` points to the interrupt frame on the kernel stack:
///   frame[0] = error code
///   frame[1] = RIP
///   frame[2] = CS
///   frame[3] = RFLAGS
///   frame[4] = user RSP
///   frame[5] = user SS
#[no_mangle]
pub unsafe extern "C" fn gp_handler(frame: *mut u64) {
    // Write "GP" header at VGA row 0, col 5 (offset 10)
    core::ptr::write_volatile(0xB800A as *mut u8, b'G');
    core::ptr::write_volatile(0xB800B as *mut u8, 0x0C);
    core::ptr::write_volatile(0xB800C as *mut u8, b'P');
    core::ptr::write_volatile(0xB800D as *mut u8, 0x0C);

    // Write error code as hex digits at VGA offset 0x10
    let err = *frame;
    for i in 0..16 {
        let nibble = ((err >> (60 - i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'A' + nibble - 10 };
        core::ptr::write_volatile((0xB810 + i * 2) as *mut u8, c);
        core::ptr::write_volatile((0xB811 + i * 2) as *mut u8, 0x0C);
    }

    // Write RIP as hex digits at VGA offset 0x30
    let rip = *frame.add(1);
    for i in 0..16 {
        let nibble = ((rip >> (60 - i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'A' + nibble - 10 };
        core::ptr::write_volatile((0xB830 + i * 2) as *mut u8, c);
        core::ptr::write_volatile((0xB831 + i * 2) as *mut u8, 0x0C);
    }

    loop {}
}

/// Double-fault handler — writes "DF" to VGA, then halts.
#[no_mangle]
pub unsafe extern "C" fn df_handler(frame: *mut u64) {
    // Write "DF" header at VGA row 0, col 5 (offset 10)
    core::ptr::write_volatile(0xB800A as *mut u8, b'D');
    core::ptr::write_volatile(0xB800B as *mut u8, 0x0C);
    core::ptr::write_volatile(0xB800C as *mut u8, b'F');
    core::ptr::write_volatile(0xB800D as *mut u8, 0x0C);

    // Write error code as hex digits
    let err = *frame;
    for i in 0..16 {
        let nibble = ((err >> (60 - i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'A' + nibble - 10 };
        core::ptr::write_volatile((0xB810 + i * 2) as *mut u8, c);
        core::ptr::write_volatile((0xB811 + i * 2) as *mut u8, 0x0C);
    }

    // Write RIP
    let rip = *frame.add(1);
    for i in 0..16 {
        let nibble = ((rip >> (60 - i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'A' + nibble - 10 };
        core::ptr::write_volatile((0xB830 + i * 2) as *mut u8, c);
        core::ptr::write_volatile((0xB831 + i * 2) as *mut u8, 0x0C);
    }

    loop {}
}

// =============================================================================
// IDT SETUP
// =============================================================================

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct IdtEntry {
    pub offset_low: u16,
    pub selector: u16,
    pub ist: u8,
    pub type_attr: u8,
    pub offset_mid: u16,
    pub offset_high: u32,
    pub reserved: u32,
}

impl IdtEntry {
    pub fn new(handler_addr: u64, selector: u16) -> Self {
        Self {
            offset_low: handler_addr as u16,
            selector,
            ist: 0,
            // 0x8E = Present | DPL=0 | Interrupt Gate (clears IF on entry)
            type_attr: 0x8E,
            offset_mid: (handler_addr >> 16) as u16,
            offset_high: (handler_addr >> 32) as u32,
            reserved: 0,
        }
    }

    pub fn null() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }
}

// Place IDT at 0x200000 — after kernel code (.text ends ~0x10c000) and
// page tables (0x102000-0x107fff). This avoids overwriting the assembly
// code at syscall_entry (0x100000) and timer_entry (0x100049).
const IDT_ADDR: u64 = 0x200000;

pub fn get_idt_base() -> *mut IdtEntry {
    IDT_ADDR as *mut IdtEntry
}

pub unsafe fn init_idt() {
    let idt = get_idt_base();
    for i in 0..256 {
        idt.add(i).write(IdtEntry::null());
    }

    // Vector 0x80: syscall (kept for compatibility, not used with syscall instr)
    let syscall_addr = syscall_dispatch as *const () as u64;
    let idt_entry = IdtEntry::new(syscall_addr, 0x08);
    idt.add(0x80).write(idt_entry);

    // Vector 0x20: timer IRQ0 → timer_entry assembly (handles preemption)
    let timer_addr = &timer_entry as *const u8 as u64;
    let timer_entry_idt = IdtEntry::new(timer_addr, 0x08);
    idt.add(0x20).write(timer_entry_idt);

    // Vector 0x21: keyboard IRQ1 → keyboard_entry assembly
    let kbd_addr = &keyboard_entry as *const u8 as u64;
    let kbd_entry_idt = IdtEntry::new(kbd_addr, 0x08);
    idt.add(0x21).write(kbd_entry_idt);

    // Vector 0x0E: page fault. User faults are isolated to the current process;
    // write-protection faults may be resolved by the COW handler.
    let pf_addr = &pf_entry as *const u8 as u64;
    let pf_idt_entry = IdtEntry::new(pf_addr, 0x08);
    idt.add(0x0E).write(pf_idt_entry);

    // Vector 0x0D: #GP (General Protection Fault) — diagnostic
    let gp_addr = &gp_entry as *const u8 as u64;
    let gp_idt_entry = IdtEntry::new(gp_addr, 0x08);
    idt.add(0x0D).write(gp_idt_entry);

    // Vectors 0x2E/0x2F: ATA disk completion IRQs — acknowledge and ignore
    // (the user-space driver polls; see irq14_entry).
    let irq14_addr = &irq14_entry as *const u8 as u64;
    idt.add(0x2E).write(IdtEntry::new(irq14_addr, 0x08));
    let irq15_addr = &irq15_entry as *const u8 as u64;
    idt.add(0x2F).write(IdtEntry::new(irq15_addr, 0x08));

    // Vector 0x08: Double Fault — diagnostic with IST1
    let df_addr = &df_entry as *const u8 as u64;
    let mut df_idt_entry = IdtEntry::new(df_addr, 0x08);
    df_idt_entry.ist = 1; // use IST1 (our double-fault stack)
    idt.add(0x08).write(df_idt_entry);

    #[repr(C, packed)]
    struct IdtPtr {
        limit: u16,
        base: u64,
    }
    let idt_ptr = IdtPtr {
        limit: 16 * 256 - 1,
        base: idt as u64,
    };
    core::arch::asm!("lidt [{}]", in(reg) &idt_ptr, options(nostack));
}

// =============================================================================
// USER-SPACE SYSCALL STUBS
// =============================================================================
//
// These are called by user programs (which are compiled into the kernel
// for now). They set up registers in the syscall convention and execute
// `syscall`.
//
// Convention:
//   rax = syscall number
//   rdi = arg1   rsi = arg2   rdx = arg3   r10 = arg4   r8 = arg5

#[inline(always)]
pub unsafe fn syscall0(nr: usize) -> isize {
    let ret: isize;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        lateout("rax") ret,
        options(nostack)
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall1(nr: usize, a1: usize) -> isize {
    let ret: isize;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        lateout("rax") ret,
        options(nostack)
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall2(nr: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        in("rsi") a2,
        lateout("rax") ret,
        options(nostack)
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall3(nr: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rax") ret,
        options(nostack)
    );
    ret
}
