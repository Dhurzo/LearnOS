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

use crate::process;
use crate::vga;

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
}

pub const SYSCALL_VECTOR: u8 = 0x80;

pub type SyscallHandler = fn(usize, usize, usize, usize, usize, usize) -> isize;

pub struct SyscallTable {
    handlers: [Option<SyscallHandler>; 16],
}

impl SyscallTable {
    pub const fn new() -> Self {
        Self {
            handlers: [None; 16],
        }
    }

    pub fn register(&mut self, nr: usize, handler: SyscallHandler) {
        if nr < 16 {
            self.handlers[nr] = Some(handler);
        }
    }

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
    table
};

fn sys_write(fd: usize, buf: usize, count: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    if (fd == 1 || fd == 2) && buf >= 0x400000 && buf < 0x800000000 {
        let slice = unsafe { core::slice::from_raw_parts(buf as *const u8, count) };
        for &byte in slice {
            vga::write_byte(byte);
        }
        count as isize
    } else {
        -1
    }
}

fn sys_exit(_code: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    loop {}
}

fn sys_vga_write(byte: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    vga::write_byte(byte as u8);
    0
}

fn sys_vga_clear(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    vga::clear_screen();
    0
}

fn sys_schedule(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    // Cooperative yield — only works when interrupts are enabled.
    // The timer handler handles preemptive scheduling.
    process::schedule_next();
    0
}

fn sys_getpid(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    process::get_current_pid() as isize
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
    if nr < 16 {
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
    "    mov rsp, [rip + KERNEL_RSP]",
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
);

extern "C" {
    static syscall_entry: u8;
    static timer_entry: u8;
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
///   [63:48] = 0x08  SYSRET base                  → CS=0x18 (R3 code), SS=0x10+8=0x10 (data)
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
    // STAR[47:32] = 0x08 (syscall CS), [63:48] = 0x08 (sysret base)
    let star = (0x08u64 << 32) | (0x08u64 << 48);
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
    use crate::process::{ProcessState, PROCESS_TABLE};

    let current = process::get_current_pid();

    // === 1. Save current process state ===
    if current != 0 {
        if let Some(p) = PROCESS_TABLE.get_mut(current) {
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

    // === 4. Load next process state into the stack frame ===
    let next = process::get_current_pid();
    if next != 0 {
        if let Some(p) = PROCESS_TABLE.get_mut(next) {
            *frame.add(14) = p.registers.rax;
            *frame.add(11) = p.registers.rbx;
            *frame.add(13) = p.registers.rcx;
            *frame.add(12) = p.registers.rdx;
            *frame.add(9)  = p.registers.rsi;
            *frame.add(8)  = p.registers.rdi;
            *frame.add(10) = p.registers.rbp;
            *frame.add(18) = p.registers.rsp;
            *frame.add(7)  = p.registers.r8;
            *frame.add(6)  = p.registers.r9;
            *frame.add(5)  = p.registers.r10;
            *frame.add(4)  = p.registers.r11;
            *frame.add(3)  = p.registers.r12;
            *frame.add(2)  = p.registers.r13;
            *frame.add(1)  = p.registers.r14;
            *frame.add(0)  = p.registers.r15;
            *frame.add(15) = p.registers.rip;
            *frame.add(17) = p.registers.rflags;

            p.state = ProcessState::Running;
        }
    }
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
