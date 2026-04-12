//! LearnOS Microkernel - Main Entry Point
//!
//! This is the heart of the LearnOS microkernel. It runs after boot.S has set up
//! 64-bit mode and paging.
//!
//! =============================================================================
//! WHAT IS A MICROKERNEL?
//! =============================================================================
//!
//! A microkernel is a kernel design where ONLY essential code runs in privileged
//! mode (kernel space). Everything else runs in unprivileged user space:
//!
//!     Traditional Monolithic Kernel:
//!     =====================
//!     +------------------+
//!     |   Kernel Space  |  <-- ALL OS code runs here
//!     | - File System  |
//!     | - Network   |
//!     | - Driver   |
//!     | - IPC      |
//!     +------------------+
//!            |
//!     +------------------+
//!     |  User Space   |
//!     | - Apps     |
//!     +------------------+
//!
//!     Microkernel Design:
//!     =============
//!     +------------------+
//!     |  Kernel Space |  <-- Minimal: only scheduling, memory, IPC
//!     +------------------+
//!            |                    syscall / message passing
//!     +------------------+  +------------------+
//!     | User Space    |  | User Space    |
//!     | - File Sys Svc|  | Network Svc |
//!     | - Display   |  | Shell      |
//!     +------------------+  +------------------+
//!
//! The key difference: Services run in separate address spaces with
//! memory protection. If a service crashes, it doesn't crash the kernel!
//!
//! =============================================================================
//! KERNEL BOOT FLOW
//! =============================================================================
//!
//! 1. BIOS/bootloader loads kernel from disk
//! 2. BIOS finds PVH entry point (.note.Xen)
//! 3. CPU jumps to pvh_start (boot.S, 32-bit)
//! 4. boot.S sets up:
//!    - Page tables (virtual memory)
//!    - GDT (segment descriptors)
//!    - Enable 64-bit mode
//! 5. Jump to _start (here, 64-bit!)
//! 6. We initialize:
//!    - VGA display
//!    - IDT (interrupt descriptor table)
//!    - Process table (manages processes)
//!    - Timer (for scheduling)
//! 7. Switch to user space (init process)
//! 8. Timer interrupts trigger scheduling
//!
//! =============================================================================
//! PROCESS MANAGEMENT
//! =============================================================================
//!
//! A process is a running program. In our microkernel, each service runs
//! as a separate process with its own:
//! - Process ID (PID) - unique identifier
//! - Address space - memory it can access
//! - State - READY, RUNNING, BLOCKED, TERMINATED
//! - Registers - saved CPU state for context switching
//!
//! Process Control Block (PCB):
//! struct Process {
//!     pid: Pid,              // Unique ID (1, 2, 3, ...)
//!     state: ProcessState,  // Current state
//!     entry_point: u64,   // Where code starts
//!     registers: ...,      // Saved registers
//! }
//!
//! The process table holds all PCBs.
//!
//! =============================================================================
//! CONTEXT SWITCHING
//! =============================================================================
//!
//! Context switching is how we run multiple processes on one CPU.
//! The CPU can only run ONE process at a time, but by rapidly
//! switching between processes, it seems like they're running
//! simultaneously (time-sharing).
//!
//! Steps to switch:
//! 1. Save current process registers to its PCB
//! 2. Load next process registers from its PCB  
//! 3. Switch to next process's stack
//! 4. Jump to next process's code
//!
//! When timer fires (IRQ0), we automatically switch!
//!
//! =============================================================================
//! SYSTEM CALLS (syscall instruction)
//! =============================================================================
//!
//! User processes can't access hardware directly. To do anything
//! (write to screen, read from disk, etc.), they must ask
//! the kernel via system calls.
//!
//! The syscall instruction:
//! 1. CPU switches to kernel mode (ring 0)
//! 2. CPU jumps to IDT entry for vector 0x80
//! 3. Handler validates arguments
//! 4. Handler performs operation
//! 5. Handler returns to user space
//!
//! Syscall convention (x86-64 System V):
//! - rax: syscall number
//! - rdi, rsi, rdx, r10, r8, r9: arguments
//! - rax: return value
//!
//! Our syscalls:
//! - 0: EXIT     - terminate process
//! - 1: WRITE    - write to file descriptor
//! - 7: VGA_WRITE - write character to screen
//! - 8: VGA_CLEAR - clear screen
//! - 9: SCHEDULE - yield CPU to next process
//! - 6: GETPID   - get current process ID
//!
//! =============================================================================
//! INTERRUPTS AND THE IDT
//! =============================================================================
//!
//! Hardware can interrupt the CPU to signal events (timer tick,
//! keyboard press, disk ready, etc.). The IDT tells the CPU where
//! to jump for each interrupt type.
//!
//! IDT Entry (16 bytes):
//! struct IdtEntry {
//!     offset_low: u16,    // Handler address bits 0-15
//!     selector: u16,     // Code segment selector
//!     ist: u8,        // Stack switch table index
//!     type_attr: u8,    // Type (trap/interrupt gate) + DPL
//!     offset_mid: u16,   // Handler address bits 16-31
//!     offset_high: u32,   // Handler address bits 32-63
//!     reserved: u32,
//! }
//!
//! Vector 0x80: syscall (our syscalls)
//! Vector 0x20: timer (IRQ0, for scheduling)
//! Vector 0x21: keyboard (IRQ1)
//!
//! =============================================================================
//! MEMORY LAYOUT
//! =============================================================================
//!
//! User Space (lower half):
//! 0x0000000000400000 - Code starts here (typical ELF load)
//! 0x00007FFFFFFFE000 - Stack (grows down)
//! 0x00007FFFFFFFFFFF - User space end
//!
//! Kernel Space (upper half):
//! 0xFFFFFFFF80000000+ - Kernel code
//!
//! Physical:
//! 0x000000 - 0x200000: Kernel (identity mapped)
//! 0x0B8000: VGA text memory (0xB8000-0xB8F9F = 80x25 text)
//!
//! =============================================================================

#![no_std] // Don't use standard library (no heap, no files, etc.)
#![no_main] // Custom entry point (_start), not main()

use core::panic::PanicInfo;

// Import all kernel modules
mod device_manager;
mod elf;
mod ipc;
mod keyboard;
mod paging;
mod process;
mod syscall;
mod user_program;
mod userspace;
mod vga;

// Import boot.S (contains pvh_start, page tables, GDT)
core::arch::global_asm!(include_str!("boot.S"));

// Use the device manager for hardware access
use crate::device_manager::get_device_manager;
use crate::process::PROCESS_TABLE;

/// ============================================================================
/// KERNEL ENTRY POINT (_start)
/// ============================================================================
///
/// This is called by boot.S after setting up 64-bit mode. This is where
/// the kernel begins execution in Rust code.
///
/// # Safety
///
/// This is the first Rust code executed. It assumes:
/// - CPU is in 64-bit long mode
/// - Paging is enabled
/// - A valid stack exists
/// - This function NEVER RETURNS (it switches to user space)
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Write '1' to VGA to show we entered _start (Rust kernel entry)
    unsafe {
        core::ptr::write_volatile(0xB8000 as *mut u8, b'1');
        core::ptr::write_volatile(0xB8001 as *mut u8, 0x0A); // Bright green
    }

    // Initialize the device manager and VGA
    let device_manager = get_device_manager();

    // Write '2' to VGA after getting device manager
    unsafe {
        core::ptr::write_volatile(0xB8002 as *mut u8, b'2');
        core::ptr::write_volatile(0xB8003 as *mut u8, 0x0A);
    }

    device_manager.initialize_vga();

    // Write '3' to VGA after VGA init
    unsafe {
        core::ptr::write_volatile(0xB8004 as *mut u8, b'3');
        core::ptr::write_volatile(0xB8005 as *mut u8, 0x0A);
    }

    // ============================================================================
    // STEP 2: CONFIGURE IDT
    // ============================================================================
    unsafe {
        setup_idt();
    }

    // Write '4' to VGA after IDT setup
    unsafe {
        core::ptr::write_volatile(0xB8006 as *mut u8, b'4');
        core::ptr::write_volatile(0xB8007 as *mut u8, 0x0A);
    }

    // ============================================================================
    // STEP 3: CREATE PROCESSES
    // ============================================================================
    unsafe {
        let pt = &mut *(&raw mut PROCESS_TABLE as *mut process::ProcessTable);

        let init_entry = crate::user_program::init::init_main as u64;
        if let Some(init_pid) = pt.spawn(init_entry, "init") {
            device_manager.print_string("Init PID 1: RUNNING\n");
            pt.set_running(init_pid);
            process::set_current_pid(init_pid);
            // Write 'I' for Init
            unsafe {
                core::ptr::write_volatile(0xB8008 as *mut u8, b'I');
                core::ptr::write_volatile(0xB8009 as *mut u8, 0x0C); // Bright red
            }
        }

        let shell_entry = crate::user_program::shell::shell_main as u64;
        if let Some(shell_pid) = pt.spawn(shell_entry, "shell") {
            device_manager.print_string("Shell PID 2: READY\n");
            pt.set_ready(shell_pid);
            // Write 'S' for Shell
            unsafe {
                core::ptr::write_volatile(0xB800A as *mut u8, b'S');
                core::ptr::write_volatile(0xB800B as *mut u8, 0x0C);
            }
        }
    }

    // Write '5' to VGA after process creation
    unsafe {
        core::ptr::write_volatile(0xB800C as *mut u8, b'5');
        core::ptr::write_volatile(0xB800D as *mut u8, 0x0A);
    }

    // ============================================================================
    // STEP 4: CONFIGURE TIMER
    // ============================================================================
    setup_timer();

    // Write 'T' for Timer
    unsafe {
        core::ptr::write_volatile(0xB800E as *mut u8, b'T');
        core::ptr::write_volatile(0xB800F as *mut u8, 0x0A);
    }

    device_manager.print_string("\n[OK] IDT configured\n");
    device_manager.print_string("[OK] Timer configured\n");
    device_manager.print_string("\nSwitching to user space...\n");

    // Write 'U' for User mode
    unsafe {
        core::ptr::write_volatile(0xB8010 as *mut u8, b'U');
        core::ptr::write_volatile(0xB8011 as *mut u8, 0x0A);
    }

    // ============================================================================
    // STEP 5: SWITCH TO USER SPACE
    // ============================================================================
    process::schedule_init();

    loop {}
}

/// ============================================================================
/// IDT SETUP
/// ============================================================================
///
/// Configures the Interrupt Descriptor Table to handle syscalls
/// and timer interrupts.
///
/// The IDT holds 256 entries (one for each interrupt vector).
/// - Vector 0x80 (128): syscall instruction
/// - Vector 0x20 (32): Timer IRQ0
///
/// # Safety
///
/// This modifies the CPU's interrupt table. Must be done before
/// interrupts are enabled.
unsafe fn setup_idt() {
    // Call the comprehensive IDT setup in syscall.rs
    // This:
    // 1. Clears all IDT entries to null
    // 2. Sets up syscall handler at vector 0x80
    // 3. Sets up timer handler at vector 0x20
    // 4. Loads the IDT with lidt instruction
    crate::syscall::init_idt();
}

/// ============================================================================
/// TIMER SETUP
/// ============================================================================
///
/// Programs the PIT (Programmable Interval Timer) to interrupt
/// periodically for scheduling.
///
/// The PIT runs at 1,193,182 Hz (approx 1.19 MHz).
/// We divide this to get our desired interrupt rate.
///
/// # Calculation:
/// divisor = 1193182 / 100 Hz = 11931
///
/// # Safety
///
/// This writes to I/O port 0x40 (PIT channel 0).
fn setup_timer() {
    // PIT programming:
    // 1. Send mode/command byte to port 0x43
    // 2. Send divisor low byte to port 0x40
    // 3. Send divisor high byte to port 0x40
    //
    // Command byte 0x36 = 00110110b:
    // - Bits 7-6: 00 = Channel 0
    // - Bits 5-4: 11 = Load both bytes
    // - Bits 3-1: 011 = Square wave generator
    // - Bit 0: 0 = 16-bit counter
    unsafe {
        core::arch::asm!(
            "mov al, 0x36",
            "out 0x43, al",
            "mov al, 0xB3", // 11931 & 0xFF = 0xB3
            "out 0x40, al",
            "mov al, 0x2E", // 11931 >> 8 = 0x2E
            "out 0x40, al",
            options(nostack)
        );
    }
}

/// ============================================================================
/// TIMER INTERRUPT HANDLER
/// ============================================================================
///
/// Called when the timer interrupt fires (approx 100 times/sec).
/// This is where preemptive multitasking happens!
///
/// When the timer fires:
/// 1. CPU saves current process state
/// 2. CPU jumps here (via IDT)
/// 3. We call schedule_next() to pick next process
/// 4. schedule_next() switches to that process
/// 5. Return via iret (not shown here)
///
/// This happens ~100 times per second, giving each
/// process a slice of CPU time.
///
/// # Safety
///
/// Called from interrupt context. Must be careful
/// about what it calls.
#[no_mangle]
pub extern "C" fn timer_tick() {
    // Schedule the next process (round-robin)
    process::schedule_next();

    // Send End Of Interrupt to PIC (Programmable Interrupt Controller)
    // This tells the hardware we're done handling this interrupt.
    unsafe {
        core::arch::asm!(
            "mov al, 0x20", // Non-specific EOI
            "out 0x20, al", // Send to master PIC
            options(nostack)
        );
    }
}

/// ============================================================================
/// KEYBOARD INTERRUPT HANDLER
/// ============================================================================
///
/// Called when a key is pressed. In a full implementation,
/// we'd read the key and send it to the focused process.
///
/// # Safety
///
/// Called from interrupt context.
#[no_mangle]
pub extern "C" fn keyboard_irq() {
    // Read keyboard status (optional)
    // Read scan code from port 0x60

    // In a full kernel, we'd:
    // 1. Read scan code from keyboard
    // 2. Convert to key code
    // 3. Store in keyboard buffer
    // 4. Signal waiting process

    // Send EOI
    unsafe {
        core::arch::asm!("mov al, 0x20", "out 0x20, al", options(nostack));
    }
}

/// ============================================================================
/// PANIC HANDLER
/// ============================================================================
///
/// This is called if something goes wrong (panic in Rust).
/// We simply halt - in a real kernel, we'd log the crash.
///
/// # Safety
///
/// Halts the CPU permanently.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // If panicking, disable interrupts and halt
    loop {}
}
