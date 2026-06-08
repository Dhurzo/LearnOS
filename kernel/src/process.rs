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

use crate::paging::{self, MemoryRegion, USER_STACK_SIZE, USER_STACK_VADDR, USER_VADDR_START, USER_VADDR_END};
use core::sync::atomic::{AtomicU16, Ordering};

/// User PID pool size (max concurrent user processes)
pub const USER_PIDS_COUNT: usize = 16;

/// Slot tracking: tracks which PIDs are in use
pub static mut USER_PID_POOL: [u16; USER_PIDS_COUNT] = [0; USER_PIDS_COUNT];

/// Allocate a user PID from the pool (linear scan for simplicity).
/// Returns 0 if pool is full.
pub fn alloc_user_pid() -> u16 {
    unsafe {
        for (i, slot) in USER_PID_POOL.iter_mut().enumerate() {
            if *slot == 0 {
                let pid = (i + 1) as u16;
                *slot = 1;
                return pid;
            }
        }
    }
    0
}

/// Free a user PID slot back to the pool.
pub fn free_user_pid_slot(pid: u16) {
    if pid > 0 && (pid as usize) <= USER_PIDS_COUNT {
        unsafe {
            USER_PID_POOL[pid as usize - 1] = 0;
        }
    }
}

// =============================================================================
// IPC Message and Per-Process Queue
// =============================================================================

/// Size of a single IPC message in bytes (fixed-size).
pub const IPC_MSG_SIZE: usize = 64;

/// Number of messages per-process queue.
pub const IPC_QUEUE_CAPACITY: usize = 16;

/// Ensure each IpcMessage is exactly 64 bytes.
#[allow(unused)]
const _IPC_MSG_SIZE_CHECK: [(); 64] = [(); core::mem::size_of::<IpcMessage>()];

/// A single IPC message exchanged between processes via kernel-mediated IPC.
///
/// The kernel copies messages between sender and receiver address spaces.
/// Messages are fixed-size (64 bytes) for simplicity and predictability.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IpcMessage {
    /// PID of the sending process (filled in by kernel).
    pub src_pid: u16,
    /// Message type (interpreted by the receiver).
    pub msg_type: u16,
    /// Payload bytes (60 bytes — total struct is 64).
    pub data: [u8; IPC_MSG_SIZE - 4],
}

impl IpcMessage {
    /// Create a new IPC message with the given source and type.
    pub const fn new(src_pid: u16, msg_type: u16, data: [u8; IPC_MSG_SIZE - 4]) -> Self {
        Self { src_pid, msg_type, data }
    }
}

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

    /// Physical address of this process's PML4 (loaded into CR3 on context switch)
    pub cr3: u64,

    /// Top of this process's kernel stack (TSS.RSP0 is set to this on context switch)
    pub kernel_stack_top: u64,

    /// Human-readable name (for debugging)
    pub name: &'static str,

    /// Per-process IPC message queue (fixed-size ring buffer).
    pub ipc_queue: [Option<IpcMessage>; IPC_QUEUE_CAPACITY],
    /// Head of the IPC ring buffer (next slot to read).
    pub ipc_head: usize,
    /// Tail of the IPC ring buffer (next slot to write).
    pub ipc_tail: usize,

    // Phase 4 — Signal handling
    /// Bitmap of pending signals (bit N = signal N pending).
    pub signal_pending: u64,
    /// Registered signal handlers (one per signal number, max 64).
    pub signal_handlers: [Option<u64>; 64],

    // Phase 4 — Capability-based IPC
    /// Bitmap of capabilities this process holds (bit N = has capability N).
    pub capabilities: u64,
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
                Some(MemoryRegion::new(USER_VADDR_START, 0x100000, 0)),
                Some(MemoryRegion::new(USER_VADDR_START + 0x100000, 0x100000, 0)),
                Some(MemoryRegion::new(
                    USER_STACK_VADDR - (USER_STACK_SIZE as u64),
                    USER_STACK_SIZE,
                    0,
                )),
                None,
            ],
            cr3: 0,
            kernel_stack_top: 0,
            name,
            ipc_queue: [None; IPC_QUEUE_CAPACITY],
            ipc_head: 0,
            ipc_tail: 0,
            signal_pending: 0,
            signal_handlers: [None; 64],
            capabilities: 0,
        }
    }

    /// Push a message into this process's IPC queue.
    /// Returns true on success, false if queue is full.
    pub fn ipc_push(&mut self, msg: IpcMessage) -> bool {
        let next_tail = (self.ipc_tail + 1) % IPC_QUEUE_CAPACITY;
        if next_tail == self.ipc_head {
            return false;
        }
        self.ipc_queue[self.ipc_tail] = Some(msg);
        self.ipc_tail = next_tail;
        true
    }

    /// Pop a message from this process's IPC queue.
    /// Returns `None` if the queue is empty.
    pub fn ipc_pop(&mut self) -> Option<IpcMessage> {
        if self.ipc_head == self.ipc_tail {
            return None;
        }
        let msg = self.ipc_queue[self.ipc_head].take();
        self.ipc_head = (self.ipc_head + 1) % IPC_QUEUE_CAPACITY;
        msg
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
            // 1. Allocate a physical frame for the user code and copy it
            let code_frame = paging::alloc_frame();
            unsafe {
                core::ptr::copy_nonoverlapping(entry as *const u8, code_frame as *mut u8, 4096);
            }

            // 2. Create per-process page tables with the code mapped at 0x400000
            let cr3 = paging::create_address_space(code_frame);

            // 3. Allocate per-process kernel stack (4KB)
            let kstack_frame = paging::alloc_frame();
            let kernel_stack_top = kstack_frame + 4096;

            // 4. Create the process with user-space entry point
            *slot = Some(Process::new(pid, paging::USER_VADDR_START, name));
            let p = slot.as_mut().unwrap();
            p.cr3 = cr3;
            p.kernel_stack_top = kernel_stack_top;
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

    // =========================================================================
    // fork_process — Phase 2: Create a child process as a copy of the parent
    // =========================================================================
    ///
    /// Clones the calling process: creates a new PCB with copied registers,
    /// a COW copy of the page table, and a new PID.  The child gets its own
    /// kernel stack.
    ///
    /// Returns the child's PID, or `None` if the process table is full.
    pub fn fork_process(&mut self, parent_pid: Pid) -> Option<Pid> {
        // Extract parent data before any mutable access to self
        let (parent_entry, parent_name, parent_cr3, parent_regs) = {
            let parent = self.get(parent_pid)?;
            (parent.entry_point, parent.name, parent.cr3, parent.registers)
        };

        let child_pid = self.next_pid;
        self.next_pid = self.next_pid.wrapping_add(1);
        if self.next_pid == 0 {
            return None; // PID wrap-around
        }

        // Find a free slot
        let slot = self.processes.iter_mut().find(|p| p.is_none())?;

        // Allocate kernel stack for child
        let kstack_frame = paging::alloc_frame();
        if kstack_frame == 0 {
            return None;
        }
        let kernel_stack_top = kstack_frame + 4096;

        // COW copy of the page table
        let child_cr3 = unsafe { paging::copy_page_table_cow(parent_cr3) };

        let mut child = Process::new(child_pid, parent_entry, parent_name);
        child.registers = parent_regs;
        child.cr3 = child_cr3;
        child.kernel_stack_top = kernel_stack_top;
        child.state = ProcessState::Ready;

        // Child gets return value 0 from fork
        child.registers.rax = 0;

        *slot = Some(child);
        Some(child_pid)
    }

    // =========================================================================
    // exec_builtin — Phase 2: Replace process image with a built-in program
    // =========================================================================
    ///
    /// Unmaps the current user pages, allocates a fresh code frame,
    /// copies the given entry-point code into it, and resets the stack.
    pub fn exec_builtin(&mut self, pid: Pid, entry: u64, name: &'static str) -> bool {
        let proc = match self.get_mut(pid) {
            Some(p) => p,
            None => return false,
        };
        // Unmap old user pages
        unsafe {
            paging::munmap_range(proc.cr3, paging::USER_VADDR_LOAD, 0x100000);
        }
        // Allocate new code frame and copy entry code
        let code_frame = paging::alloc_frame();
        if code_frame == 0 {
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(entry as *const u8, code_frame as *mut u8, 4096);
        }
        // Remap code at 0x400000
        unsafe {
            paging::map_phys_page(
                proc.cr3,
                paging::USER_VADDR_LOAD,
                code_frame,
                paging::PTE_USER,
            );
        }
        // Reset registers
        proc.entry_point = entry;
        proc.registers = ProcessRegisters::default();
        proc.registers.rsp = paging::USER_STACK_VADDR;
        proc.registers.rip = entry;
        proc.name = name;
        true
    }

    // =========================================================================
    // load_elf_into_process — Phase 3: Load an ELF file from the embedded
    // filesystem and replace the process image with it.
    // =========================================================================
    ///
    /// Opens an ELF binary from the embedded filesystem by path, reads it
    /// into a kernel buffer, parses and maps its LOAD segments via
    /// `elf::load_elf`, then resets registers so execution starts at
    /// the ELF entry point.
    ///
    /// Returns `true` on success, `false` if the file was not found,
    /// is not a valid ELF, or memory allocation fails.
    pub fn load_elf_into_process(&mut self, pid: Pid, path: &str) -> bool {
        // 1. Open file in the embedded filesystem
        let fd = match crate::filesystem::open(pid, path) {
            Some(fd) => fd,
            None => return false,
        };

        // 2. Get file size (cap at 64 KiB for our kernel buffer)
        let size = match crate::filesystem::file_size(pid, fd) {
            Some(s) => s,
            None => {
                crate::filesystem::close(pid, fd);
                return false;
            }
        };
        let max_size = size.min(65536);

        // 3. Read entire file into a static kernel buffer
        let mut buf = [0u8; 65536];
        let n = match crate::filesystem::read_all(pid, fd, &mut buf[..max_size]) {
            Some(n) => n,
            None => {
                crate::filesystem::close(pid, fd);
                return false;
            }
        };

        // 4. Close the file (we have the data in our buffer now)
        crate::filesystem::close(pid, fd);

        // 5. Load ELF segments into the process's address space
        let proc = match self.get_mut(pid) {
            Some(p) => p,
            None => return false,
        };

        let result = match unsafe { crate::elf::load_elf(&buf[..n], proc.cr3) } {
            Ok(r) => r,
            Err(_) => return false,
        };

        // 6. Reset registers like exec_builtin does
        proc.entry_point = result.entry;
        proc.registers = ProcessRegisters::default();
        proc.registers.rsp = result.stack_top;
        proc.registers.rip = result.entry;

        true
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
                    let cr3 = p.cr3;
                    let kernel_stack_top = p.kernel_stack_top;

                    // Mark as running
                    if let Some(p_mut) = pt.get_mut(pid) {
                        p_mut.state = ProcessState::Running;
                    }
                    set_current_pid(pid);

                    // DO THE ACTUAL SWITCH!
                    schedule_user(entry, stack, cr3, kernel_stack_top);
                }
            }
        }
    }
}

/// ============================================================================
/// Switch to User Space (Ring 3)
/// ============================================================================
///
/// This performs the transition from kernel mode (ring 0) to user mode
/// (ring 3) using the `iretq` instruction.
///
/// How IRETQ works for ring transitions:
///   1. CPU pops RIP, CS, RFLAGS, RSP, SS from the stack
///   2. If CS.RPL != current CPL, CPU also loads SS.RPL = CS.RPL
///   3. Since new RPL=3 > current RPL=0, it's an outer-privilege jump
///   4. CPU validates segment descriptors (DPL must match RPL)
///   5. CPU sets CPL=3 and continues executing at RIP in ring 3
///
/// After `iretq`, the CPU is running at ring 3:
///   - Memory accesses check the U (user) bit in page tables
///   - Privileged instructions (wrmsr, lidt, lgdt, etc.) cause #GP
///   - `syscall` and `int` can request kernel services
///
/// # Arguments
///
/// * `entry` - User process entry point (RIP)
/// * `stack` - User stack pointer (RSP)
/// * `cr3` - Physical address of process PML4 (loaded into CR3)
/// * `kernel_stack_top` - Top of per-process kernel stack (for TSS.RSP0 and syscall)
///
/// # Safety
///
/// Modifies CPU privilege level and page tables. Never returns.
fn schedule_user(entry: u64, stack: u64, cr3: u64, kernel_stack_top: u64) {
    unsafe {
        crate::tss::TSS.set_rsp0(kernel_stack_top);
        crate::tss::CURRENT_KERNEL_RSP = kernel_stack_top;

        core::arch::asm!(
            // Load per-process page tables so the CPU uses the correct
            // code mapping at 0x400000 and user stack at 0xFFF000.
            "mov cr3, {cr3}",

            // Build iretq frame on stack (reverse order):
            "push 0x23",        // SS = ring-3 data segment | RPL3
            "push {stack}",     // User RSP
            "push {rflags}",    // RFLAGS with IF=1 for interrupts
            "push 0x1B",        // CS = ring-3 code segment | RPL3
            "push {entry}",     // User RIP

            // Set data segments for user mode
            "mov ax, 0x23",
            "mov ds, ax",
            "mov es, ax",

            // iretq → pops RIP, CS, RFLAGS, RSP, SS and enters ring 3
            "iretq",
            cr3 = in(reg) cr3,
            stack = in(reg) stack,
            entry = in(reg) entry,
            rflags = in(reg) 0x202u64,   // IF=1, reserved bits set
            options(noreturn)
        );
    }
}
