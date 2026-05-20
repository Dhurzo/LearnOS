//! Task State Segment (TSS)
//!
//! The TSS is used by the CPU during privilege level transitions.
//! When an interrupt fires while in ring 3, the CPU reads RSP0
//! from the TSS to find the kernel stack.
//!
//! On x86-64, the TSS is NOT used for hardware task switching
//! (that was a 32-bit feature). Instead, it provides:
//!
//! - **RSP0**: Kernel stack for ring-3→ring-0 transitions
//! - **IST[0..6]**: Interrupt Stack Table — independent stacks
//!   for specific interrupt vectors (e.g., double fault)
//!
//! ============================================================================
//! HOW INTERRUPTS FROM RING 3 WORK
//! ============================================================================
//!
//! When the CPU is executing user code (ring 3) and an interrupt fires:
//!
//! 1. CPU reads RSP0 from the TSS → gets kernel stack address
//! 2. CPU switches RSP to that kernel stack
//! 3. CPU pushes SS, old RSP, RFLAGS, CS, RIP on the kernel stack
//! 4. CPU jumps to the interrupt handler (ring 0)
//!
//! Without the TSS, the CPU would use the user's RSP to push the
//! interrupt frame — but the user's stack is not trusted!

use core::arch::asm;

/// Size of a 64-bit TSS in bytes (see Intel SDM Vol. 3, Ch. 7)
const TSS_SIZE: usize = 104;

/// A 64-bit Task State Segment.
///
/// The TSS is a static data structure in kernel memory.
/// The `ltr` instruction loads a _pointer_ to it via the GDT.
#[repr(C, align(16))]
pub struct TaskStateSegment {
    /// Raw TSS bytes
    data: [u8; TSS_SIZE],
}

impl TaskStateSegment {
    /// Create a new, zeroed TSS.
    pub const fn new() -> Self {
        Self {
            data: [0u8; TSS_SIZE],
        }
    }

    /// Set the ring-0 stack pointer (RSP0).
    ///
    /// RSP0 is at byte offset 4 in the TSS (8 bytes, little-endian).
    ///
    /// This is the stack the CPU switches to when an interrupt
    /// occurs while the CPU is in ring 3 (user mode).
    pub fn set_kernel_stack(&mut self, rsp: u64) {
        let bytes = rsp.to_le_bytes();
        self.data[4..12].copy_from_slice(&bytes);
    }

    /// Update RSP0 to point to the given process's kernel stack.
    /// Called during context switch so the CPU uses the correct
    /// kernel stack when the next process makes a syscall or is interrupted.
    pub fn set_rsp0(&mut self, rsp0: u64) {
        let bytes = rsp0.to_le_bytes();
        self.data[4..12].copy_from_slice(&bytes);
    }

    /// Set the I/O map base address.
    ///
    /// If this value is >= the TSS limit, the CPU will raise #GP
    /// on any user-mode `in`/`out` instruction. This is the
    /// secure default for a microkernel — user processes should
    /// access hardware via syscalls, not raw I/O ports.
    pub fn set_iopb(&mut self, offset: u16) {
        let bytes = offset.to_le_bytes();
        self.data[102..104].copy_from_slice(&bytes);
    }

    /// Set IST1 (Interrupt Stack Table entry 1).
    ///
    /// Used by the double-fault handler so it runs on a known-good stack.
    /// IST1 is at bytes 36-43 in the TSS.
    pub fn set_ist1(&mut self, stack_top: u64) {
        let bytes = stack_top.to_le_bytes();
        self.data[36..44].copy_from_slice(&bytes);
    }

    /// Get the base address of this TSS.
    pub fn base_address(&self) -> u64 {
        self as *const Self as u64
    }

    pub fn rsp0(&self) -> u64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.data[4..12]);
        u64::from_le_bytes(bytes)
    }

    pub fn ist1(&self) -> u64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.data[36..44]);
        u64::from_le_bytes(bytes)
    }
}

/// Global TSS instance.
///
/// Initialized during kernel boot. The kernel stack is written
/// before any interrupt can fire from user space.
pub static mut TSS: TaskStateSegment = TaskStateSegment::new();

/// A dedicated kernel stack for ring-0 interrupt/syscall handling.
///
/// When a ring-3 program makes a syscall or gets interrupted,
/// the CPU switches to this stack (via TSS.RSP0). This ensures
/// the kernel never runs on an untrusted (user) stack.
#[repr(C, align(16))]
pub struct KernelStack {
    data: [u8; 4096],
}

/// Global kernel stack instance.
pub static mut KERNEL_STACK: KernelStack = KernelStack { data: [0u8; 4096] };

/// A dedicated stack for the double-fault handler.
///
/// The double-fault handler uses IST1 (Interrupt Stack Table entry 1)
/// so it always runs on a known-good stack. Without IST, a double fault
/// caused by a corrupt stack pointer would itself triple-fault.
#[repr(C, align(16))]
pub struct DoubleFaultStack {
    data: [u8; 4096],
}

/// Global double-fault stack instance.
pub static mut DOUBLE_FAULT_STACK: DoubleFaultStack = DoubleFaultStack { data: [0u8; 4096] };

/// Kernel stack pointer for `syscall` entry.
///
/// The `syscall` instruction does NOT switch stacks (unlike `int`).
/// Our `syscall_entry` assembly loads RSP from this global.
/// Initialized during `tss::init()`.
#[no_mangle]
pub static mut KERNEL_RSP: u64 = 0;

/// Temporary save slot for the user RSP during `syscall` handling.
///
/// Read by `syscall_entry` assembly: the user's RSP is saved here
/// immediately after entering the kernel, then restored just before
/// `sysretq`.
#[no_mangle]
pub static mut USER_RSP_SAVE: u64 = 0;

/// Per-process kernel stack pointer for `syscall` entry.
///
/// Updated during context switch to point to the current process's
/// kernel stack. The `syscall_entry` assembly loads RSP from this
/// global so each process uses its own kernel stack.
#[no_mangle]
pub static mut CURRENT_KERNEL_RSP: u64 = 0;

/// Get the top address of the kernel stack.
///
/// The stack grows downward, so the initial RSP points
/// to the highest byte.
pub fn kernel_stack_top() -> u64 {
    unsafe { (&KERNEL_STACK as *const KernelStack as u64) + 4096 }
}

/// ============================================================================
/// TSS DESCRIPTOR
/// ============================================================================
///
/// In the GDT, a TSS is represented by a 16-byte system descriptor
/// spanning two consecutive 8-byte GDT entries:
///
/// Entry N (first 8 bytes):
/// ┌──────┬──────┬──────┬──────┬──────┬──────┬──────┬──────┐
/// │ LIM  │ LIM  │ BASE │ BASE │ BASE │ ACC  │ LIM  │ BASE │
/// │ [7:0]│[15:8]│ [7:0]│[15:8]│[23:16]│ 0x89 │[19:16]│[31:24]│
/// └──────┴──────┴──────┴──────┴──────┴──────┴──────┴──────┘
///   0      1      2      3      4      5      6      7
///
/// Entry N+1 (second 8 bytes):
/// ┌──────┬──────┬──────┬──────┬──────┬──────┬──────┬──────┐
/// │         BASE[63:32]          │        RESERVED        │
/// └──────┴──────┴──────┴──────┴──────┴──────┴──────┴──────┘
///   8      9      10     11     12     13     14     15
///
/// Access byte (0x89):
///   Bit 7: P    = 1 (present)
///   Bits 6-5: DPL = 00 (ring 0)
///   Bit 4: S   = 0 (system descriptor)
///   Bits 3-0: type = 1001 (available 64-bit TSS)
/// → 0b1000_1001 = 0x89

/// Compute the TSS descriptor and write it into the GDT at entries 5 and 6.
///
/// # Safety
///
/// `gdt_base` must point to a valid, writable GDT with at least
/// 7 entries (indices 0-6). This must be called before `ltr`.
unsafe fn write_tss_descriptor(gdt_base: *mut u64) {
    let tss_base = TSS.base_address();
    let limit: u32 = (TSS_SIZE - 1) as u32; // 103

    // Build 16-byte descriptor
    let desc: [u8; 16] = [
        // Bytes 0-7 (GDT entry 5)
        (limit >> 0) as u8,                // [0]  Limit[7:0]
        (limit >> 8) as u8,                // [1]  Limit[15:8]
        (tss_base >> 0) as u8,             // [2]  Base[7:0]
        (tss_base >> 8) as u8,             // [3]  Base[15:8]
        (tss_base >> 16) as u8,            // [4]  Base[23:16]
        0x89u8,                              // [5]  Access byte
        ((limit >> 16) as u8) & 0x0F,       // [6]  Limit[19:16] | flags
        (tss_base >> 24) as u8,            // [7]  Base[31:24]
        // Bytes 8-15 (GDT entry 6)
        (tss_base >> 32) as u8,            // [8]  Base[39:32]
        (tss_base >> 40) as u8,            // [9]  Base[47:40]
        (tss_base >> 48) as u8,            // [10] Base[55:48]
        (tss_base >> 56) as u8,            // [11] Base[63:56]
        0u8, 0u8, 0u8, 0u8,                 // [12..15] Reserved
    ];

    let low = u64::from_le_bytes(desc[0..8].try_into().unwrap());
    let high = u64::from_le_bytes(desc[8..16].try_into().unwrap());

    gdt_base.add(5).write(low);   // GDT entry 5 (selector 0x28)
    gdt_base.add(6).write(high);  // GDT entry 6 (selector 0x30)
}

/// ============================================================================
/// TSS INITIALIZATION
/// ============================================================================
///
/// Called once during kernel boot to:
/// 1. Set the kernel stack for ring-3→ring-0 transitions
/// 2. Disable user-mode I/O port access
/// 3. Set the double-fault IST1 stack
/// 4. Write the TSS descriptor into the GDT
/// 5. Load the task register (TR) with `ltr`
///
/// # Safety
///
/// This modifies global GDT state and the TR. Must be called
/// before any interrupt can fire from user space.
pub unsafe fn init() {
    let stack_top = kernel_stack_top();

    // 1. Set RSP0 — the kernel stack for interrupt handling
    TSS.set_kernel_stack(stack_top);

    // 2. Set IOPB = TSS size to disable user I/O port access
    TSS.set_iopb(TSS_SIZE as u16);

    // 3. Set IST1 to the double-fault stack top
    let df_top = (&DOUBLE_FAULT_STACK as *const DoubleFaultStack as u64) + 4096;
    TSS.set_ist1(df_top);

    // 4. Publish kernel stack pointer for syscall entry assembly
    KERNEL_RSP = stack_top;

    // 5. Get GDT base address (exported by boot.S)
    extern "C" {
        static gdt64: u8;
    }
    let gdt_base = &gdt64 as *const u8 as *mut u64;

    // 5. Write TSS descriptor into GDT
    write_tss_descriptor(gdt_base);

    // 6. Load the task register (selector = 0x28 = GDT entry 5)
    asm!("ltr {0:x}", in(reg) 0x28u16, options(nostack));
}
