//! Process Management - The Heart of Multitasking
//!
//! This module manages all processes in the system. Each process is an
//! independent execution context that can run in user space.
//!
//! =============================================================================
//! WHY DO WE NEED PROCESSES?
//! =============================================================================
//!
//! A single CPU can only execute one instruction at a time, but
//! we want to run multiple programs "simultaneously".
//!
//! Solution: Time-sharing (multitasking)
//! - Run process A for a few milliseconds
//! - Save its state, switch to process B
//! - Run process B for a few milliseconds
//! - Repeat!
//!
//! The user perceives this as all programs running at once!
//! This is called "preemptive" multitasking.
//!
//! =============================================================================
//! THE PROCESS CONTROL BLOCK (PCB)
//! =============================================================================
//!
//! Every process needs to store its state somewhere. That's the PCB:
//!
//! ```c
//! struct Process {
//!     pid: u16,          // Unique process ID (1, 2, 3, ...)
//!     state: State,     // READY, RUNNING, BLOCKED, TERMINATED
//!     entry: u64,     // Where code starts (instruction pointer)
//!     registers: ..., // Saved registers for context switch
//! }
//! ```
//!
//! When we switch processes:
//! 1. Save all registers to current PCB
//! 2. Load registers from next PCB
//! 3. Jump to where next PCB was executing
//!
//! =============================================================================
//! PROCESS STATES
//! =============================================================================
//!
//! READY: Process is waiting to run. It's in the run queue.
//! RUNNING: Process is currently executing on the CPU.
//! BLOCKED: Process is waiting for I/O (keyboard, disk, etc.)
//! TERMINATED: Process has finished and released resources.
//!
//! State transitions:
//!   READY -> RUNNING: Scheduler picks this process
//!   RUNNING -> READY: Timer interrupt causes switch
//!   RUNNING -> BLOCKED: Process requests I/O
//!   READY/YUNNING -> TERMINATED: Process exits
//!
//! =============================================================================
//! CONTEXT SWITCHING EXPLAINED
//! =============================================================================
//!
//! Context switching is how we switch between processes. Here's how it works:
//!
//! ```
//! CPU register state (simplified):
//! - RIP: Instruction pointer (what to execute next)
//! - RSP: Stack pointer (where the stack is)
//! - RAX, RBX, RCX, ...: General purpose registers
//! - RFLAGS: Status flags
//! ```
//!
//! When switching from Process A to Process B:
//! 1. Save allregisters to A's PCB in memory
//! 2. Load B's registers from B's PCB
//! 3. Jump to where B was last executing (RIP)
//! 4. Now B is running!
//!
//! This all happens in microseconds, so it's imperceptible!
//!
//! =============================================================================
//! ROUND-ROBIN SCHEDULING
//! =============================================================================
//!
//! Our scheduler is simple: round-robin. It picks processes
//! in order, giving each an equal time slice.
//!
//! With processes Init (RUNNING) and Shell (READY):
//! - Timer fires
//! - schedule_next() is called
//! - Init is RUNNING -> mark READY
//! - Find next READY process -> Shell
//! - Shell is READY -> mark RUNNING
//! - Switch complete!
//!
//! Next timer fire, Shell goes back to READY, Init becomes RUNNING again.
//!
//! =============================================================================

use crate::paging::{MemoryRegion, USER_STACK_SIZE, USER_STACK_VADDR, USER_VADDR_START};
use core::sync::atomic::{AtomicU16, Ordering};

/// ============================================================================
/// Maximum number of processes the kernel can manage
/// ============================================================================
///
/// Simple fixed-size table for now. In a real kernel,
/// this would be dynamic or much larger.
pub const MAX_PROCESSES: usize = 8;

/// ============================================================================
/// Process State
/// ============================================================================
///
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,      // Process is ready to run
    Running,    // Process is executing
    Blocked,    // Process is waiting for I/O
    Terminated, // Process has finished
}

/// ============================================================================
/// Process ID type
/// ============================================================================
///
/// Unique identifier for each process.
/// 0 is used as "no process" / error value.
pub type Pid = u16;

/// ============================================================================
/// Process Control Block (PCB)
/// ============================================================================
///
/// Holds all information about a process.
/// This is the fundamental unit of process management.
pub struct Process {
    /// Unique process ID (1, 2, 3, ...)
    pub pid: Pid,

    /// Current execution state
    pub state: ProcessState,

    /// Where execution starts (RIP initial value)
    pub entry_point: u64,

    /// Saved register state (for context switching)
    pub registers: ProcessRegisters,

    /// Memory regions (code, data, stack)
    pub memory_regions: [Option<MemoryRegion>; 4],

    /// Human-readable name (for debugging)
    pub name: &'static str,
}

/// ============================================================================
/// Process Registers
/// ============================================================================
///
/// Saved CPU registers for context switching.
/// These are all the general-purpose registers plus
/// instruction pointer and flags.
///
/// When we switch away from a process, we save ALL these.
/// When we switch to a process, we restore ALL these.
///
/// Note: We use Copy so we can easily assign structs.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProcessRegisters {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,    // Instruction pointer
    pub rflags: u64, // Flags (IF, etc.)
}

impl Process {
    /// ============================================================================
    /// Create a new process
    /// ============================================================================
    ///
    /// # Arguments
    ///
    /// * `pid` - Unique process ID
    /// * `entry` - Entry point address
    /// * `name` - Process name for debugging
    ///
    /// # Returns
    ///
    /// New Process with default registers and memory regions
    pub fn new(pid: Pid, entry: u64, name: &'static str) -> Self {
        Self {
            pid,
            state: ProcessState::Ready,
            entry_point: entry,
            registers: ProcessRegisters::default(),
            memory_regions: [
                // Code region: starts at USER_VADDR_START
                Some(MemoryRegion::new(USER_VADDR_START, 0x100000, 0)),
                // Data region: after code
                Some(MemoryRegion::new(USER_VADDR_START + 0x100000, 0x100000, 0)),
                // Stack: at high address, grows down
                Some(MemoryRegion::new(
                    USER_STACK_VADDR - USER_STACK_SIZE,
                    USER_STACK_SIZE,
                    0,
                )),
                None, // Extra (future use)
            ],
            name,
        }
    }

    /// ============================================================================
    /// Initialize registers for first run
    /// ============================================================================
    ///
    /// Sets up initial stack and instruction pointer.
    pub fn init_registers(&mut self) {
        self.registers.rsp = USER_STACK_VADDR;
        self.registers.rip = self.entry_point;
        self.registers.rbp = USER_STACK_VADDR;
    }
}

/// ============================================================================
/// Default register state
/// ============================================================================
impl Default for ProcessRegisters {
    fn default() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rsp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            // RFLAGS: IF=1 (interrupts enabled)
            rflags: 0x202,
        }
    }
}

/// ============================================================================
/// Process Table - Holds all processes
/// ============================================================================
///
/// This is a fixed-size array. Each slot can hold
/// optionally a process (or None if empty).
pub struct ProcessTable {
    /// Array of processes
    processes: [Option<Process>; MAX_PROCESSES],
    /// Next PID to assign
    next_pid: Pid,
}

impl ProcessTable {
    /// ============================================================================
    /// Create empty process table
    /// ============================================================================
    pub const fn new() -> Self {
        Self {
            processes: [None, None, None, None, None, None, None, None],
            next_pid: 1,
        }
    }

    /// ============================================================================
    /// Spawn new process
    /// ============================================================================
    ///
    /// Creates a new process with given entry point.
    ///
    /// # Arguments
    ///
    /// * `entry` - Entry point virtual address
    /// * `name` - Process name
    ///
    /// # Returns
    ///
    /// The new process's PID, or None if table is full
    pub fn spawn(&mut self, entry: u64, name: &'static str) -> Option<Pid> {
        // Don't overflow PIDs
        if self.next_pid == 0 {
            return None;
        }

        let pid = self.next_pid;
        self.next_pid = self.next_pid.wrapping_add(1);

        // Find empty slot and create process
        self.processes.iter_mut().find(|p| p.is_none()).map(|slot| {
            *slot = Some(Process::new(pid, entry, name));
            let p = slot.as_mut().unwrap();
            p.init_registers();
            pid
        })
    }

    /// ============================================================================
    /// Get process by PID
    /// ============================================================================
    pub fn get(&self, pid: Pid) -> Option<&Process> {
        self.processes
            .iter()
            .find(|p| p.as_ref().map(|p| p.pid) == Some(pid))
            .and_then(|p| p.as_ref())
    }

    /// ============================================================================
    /// Get mutable process by PID
    /// ============================================================================
    pub fn get_mut(&mut self, pid: Pid) -> Option<&mut Process> {
        self.processes
            .iter_mut()
            .find(|p| p.as_ref().map(|p| p.pid) == Some(pid))
            .and_then(|p| p.as_mut())
    }

    /// ============================================================================
    /// Mark process as RUNNING
    /// ============================================================================
    pub fn set_running(&mut self, pid: Pid) {
        if let Some(p) = self.get_mut(pid) {
            p.state = ProcessState::Running;
        }
    }

    /// ============================================================================
    /// Mark process as BLOCKED
    /// ============================================================================
    pub fn set_blocked(&mut self, pid: Pid) {
        if let Some(p) = self.get_mut(pid) {
            p.state = ProcessState::Blocked;
        }
    }

    /// ============================================================================
    /// Mark process as READY
    /// ============================================================================
    pub fn set_ready(&mut self, pid: Pid) {
        if let Some(p) = self.get_mut(pid) {
            p.state = ProcessState::Ready;
        }
    }
}

/// ============================================================================
/// Global Process Table
/// ============================================================================
///
/// Single shared instance. In a real kernel, this would
/// be protected by synchronization primitives.
pub static mut PROCESS_TABLE: ProcessTable = ProcessTable::new();

/// ============================================================================
/// Current Running Process ID
/// ============================================================================
///
/// Tracks which process is currently executing.
/// Used by syscalls to know who's making requests.
static CURRENT_PID: AtomicU16 = AtomicU16::new(0);

/// ============================================================================
/// Get current process ID
/// ============================================================================
pub fn get_current_pid() -> Pid {
    CURRENT_PID.load(Ordering::Acquire)
}

/// ============================================================================
/// Set current process ID
/// ============================================================================
pub fn set_current_pid(pid: Pid) {
    CURRENT_PID.store(pid, Ordering::Release);
}

/// ============================================================================
/// Schedule Next Process (Round-Robin)
/// ============================================================================
///
/// This is called by the timer interrupt to implement
/// preemptive multitasking.
///
/// Algorithm:
/// 1. Mark current process as READY (if running)
/// 2. Find next READY process
/// 3. Mark it as RUNNING
/// 4. Switch to it!
///
/// If no other process is READY, keep current running.
pub fn schedule_next() {
    let current = get_current_pid();

    unsafe {
        let pt = &mut PROCESS_TABLE;

        // 1. Mark current as READY if it was RUNNING
        if current != 0 {
            if let Some(p) = pt.get_mut(current) {
                if p.state == ProcessState::Running {
                    p.state = ProcessState::Ready;
                }
            }
        }

        // 2. Find next READY process
        let mut next_pid: Pid = 0;
        for i in 0..MAX_PROCESSES {
            if let Some(ref p) = pt.processes[i] {
                if p.state == ProcessState::Ready && p.pid != current {
                    next_pid = p.pid;
                    break;
                }
            }
        }

        // 3. Switch to next process
        if next_pid != 0 {
            if let Some(p) = pt.get_mut(next_pid) {
                p.state = ProcessState::Running;
                set_current_pid(next_pid);
            }
        } else if current != 0 {
            // No other READY, keep current running
            if let Some(p) = pt.get_mut(current) {
                p.state = ProcessState::Running;
            }
        }
    }
}

/// ============================================================================
/// Initial Schedule - First Process Switch
/// ============================================================================
///
/// Called once at boot to start the first process.
/// This does the actual context switch to user space.
pub fn schedule_init() {
    unsafe {
        let pt = &mut PROCESS_TABLE;

        // Find first ready/running process
        for i in 0..MAX_PROCESSES {
            if let Some(ref p) = pt.processes[i] {
                if p.state == ProcessState::Ready || p.state == ProcessState::Running {
                    // Get process info
                    let entry = p.entry_point;
                    let stack = p.registers.rsp;
                    let pid = p.pid;

                    // Mark as running
                    if let Some(p_mut) = pt.get_mut(pid) {
                        p_mut.state = ProcessState::Running;
                    }
                    set_current_pid(pid);

                    // DO THE ACTUAL SWITCH!
                    schedule_user(entry, stack);
                }
            }
        }
    }
}

/// ============================================================================
/// Switch to User Space
/// ============================================================================
///
/// This performs the magic jump from kernel mode
/// to user mode. After this, we're executing
/// user-space code!
///
/// # Arguments
///
/// * `entry` - User process entry point
/// * `stack` - User stack address
///
/// # Safety
///
/// This modifies CPU state and never returns.
fn schedule_user(entry: u64, stack: u64) {
    unsafe {
        // Set up user stack and jump to entry point
        // Note: In a full kernel, we'd also switch CR3
        // for different address spaces
        let rsp = stack;
        let rip = entry;

        core::arch::asm!(
            "mov rsp, {0}",
            "mov rbp, {0}",
            "jmp {1}",
            in(reg) rsp,
            in(reg) rip,
            options(noreturn)
        );
    }
}
