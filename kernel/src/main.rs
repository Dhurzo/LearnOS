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
mod paging;
mod process;
mod syscall;
mod tss;
mod user_program;
mod vga;

// Import boot.S (contains pvh_start, page tables, GDT)
core::arch::global_asm!(include_str!("boot.S"));

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
/// Initialize serial port COM1 (0x3F8) for debug output.
/// Uses proper x86 `out` instructions (not memory-mapped I/O).
unsafe fn init_serial() {
    let port = 0x3F8u16;
    // Set DLAB=1 (divisor latch access)
    core::arch::asm!("out dx, al", in("dx") port + 3, in("al") 0x80u8, options(nostack));
    // Set divisor to 1 (115200 baud)
    core::arch::asm!("out dx, al", in("dx") port + 0, in("al") 0x01u8, options(nostack));
    core::arch::asm!("out dx, al", in("dx") port + 1, in("al") 0x00u8, options(nostack));
    // Set DLAB=0, 8n1 mode
    core::arch::asm!("out dx, al", in("dx") port + 3, in("al") 0x03u8, options(nostack));
    // Enable FIFO, clear, with 14-byte threshold
    core::arch::asm!("out dx, al", in("dx") port + 2, in("al") 0xC7u8, options(nostack));
}

/// Write a byte to serial port COM1 using x86 `out` instruction.
unsafe fn serial_out(byte: u8) {
    let port = 0x3F8u16;
    // Wait for transmitter to be ready
    loop {
        let status: u8;
        core::arch::asm!("in al, dx", out("al") status, in("dx") port + 5, options(nostack));
        if status & 0x20 != 0 {
            break;
        }
    }
    core::arch::asm!("out dx, al", in("dx") port, in("al") byte, options(nostack));
}

/// Write a string to serial COM1 for debug output.
unsafe fn serial_puts(s: &str) {
    for &byte in s.as_bytes() {
        serial_out(byte);
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Init serial first thing for debug output
    unsafe {
        init_serial();
        serial_puts("S1\n");  // Signal 1: entered _start
    }

    // Write '1' to VGA to show we entered _start (Rust kernel entry)
    unsafe {
        core::ptr::write_volatile(0xB8000 as *mut u8, b'1');
        core::ptr::write_volatile(0xB8001 as *mut u8, 0x0A); // Bright green
        serial_puts("S2\n");  // Signal 2: VGA write done
    }

    // Write '2' to VGA to show we got past module init
    unsafe {
        core::ptr::write_volatile(0xB8002 as *mut u8, b'2');
        core::ptr::write_volatile(0xB8003 as *mut u8, 0x0A);
    }

    // Write '3' to VGA after VGA init
    unsafe {
        core::ptr::write_volatile(0xB8004 as *mut u8, b'3');
        core::ptr::write_volatile(0xB8005 as *mut u8, 0x0A);
    }

    // ============================================================================
    // STEP 1b: INITIALIZE PHYSICAL FRAME ALLOCATOR
    // ============================================================================
    // The frame allocator manages physical memory for page tables and
    // user-process code pages. Must be called before any spawn() or
    // create_address_space().
    paging::init_frame_allocator();

    // ============================================================================
    // STEP 1c: ENABLE PAGE GLOBAL ENABLE (CR4.PGE)
    // ============================================================================
    // This makes kernel page-table entries with the Global (G) bit survive
    // CR3 writes, so kernel TLB entries don't get flushed on every context
    // switch. Must be done once before any per-process page tables are created.
    unsafe {
        paging::enable_pge();
    }

    // ============================================================================
    // STEP 2: CONFIGURE TASK STATE SEGMENT
    // ============================================================================
    //
    // The TSS provides the kernel stack pointer (RSP0) that the CPU loads
    // when an interrupt or syscall transitions from ring 3 to ring 0.
    // Without it, ring-3->ring-0 transitions would corrupt the kernel!
    unsafe {
        crate::tss::init();
    }

    // ============================================================================
    // STEP 3: CONFIGURE SYSCALL MSRS
    // ============================================================================
    //
    // Sets up STAR, LSTAR, and SFMASK MSRs so the `syscall` instruction
    // in user programs jumps to our `syscall_entry` handler.
    unsafe {
        crate::syscall::init_syscall();
    }

    // ============================================================================
    // STEP 4: CONFIGURE IDT
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

        // --- Spawn VGA server (PID 1) before any client processes ---
        let vga_entry = crate::user_program::vga_server::vga_main as u64;
        if let Some(vga_pid) = pt.spawn(vga_entry, "vga-server") {
            // Map the physical VGA text buffer into the VGA server's address space
            if let Some(vga_proc) = pt.get_mut(vga_pid) {
                paging::map_phys_page(
                    vga_proc.cr3,
                    paging::VGA_BUFFER_VADDR,
                    paging::VGA_PHYS_ADDR,
                    paging::page_flags::PRESENT
                        | paging::page_flags::WRITABLE
                        | paging::page_flags::USER_ACCESS
                        | paging::page_flags::CACHE_DISABLE,
                );
                // Register VGA_SERVER_PID so sys_vga_write can forward via IPC
                crate::syscall::set_vga_server_pid(vga_pid);
            }
            pt.set_running(vga_pid);
            process::set_current_pid(vga_pid);
            // Write 'V' for VGA server
            unsafe {
                core::ptr::write_volatile(0xB8008 as *mut u8, b'V');
                core::ptr::write_volatile(0xB8009 as *mut u8, 0x0A); // Bright green
            }
        }

        // --- Spawn keyboard server (PID 2) ---
        let kbd_entry = crate::user_program::keyboard_server::keyboard_main as u64;
        if let Some(kbd_pid) = pt.spawn(kbd_entry, "keyboard") {
            pt.set_ready(kbd_pid);
            crate::syscall::set_keyboard_server_pid(kbd_pid);
            unsafe {
                core::ptr::write_volatile(0xB800A as *mut u8, b'K');
                core::ptr::write_volatile(0xB800B as *mut u8, 0x0A);
            }
        }

        // --- Spawn init process ---
        let init_entry = crate::user_program::init::init_main as u64;
        if let Some(init_pid) = pt.spawn(init_entry, "init") {
            vga::serial_print("Init PID 1: RUNNING\n");
            pt.set_running(init_pid);
            process::set_current_pid(init_pid);
            // Write 'I' for Init
            unsafe {
                core::ptr::write_volatile(0xB800C as *mut u8, b'I');
                core::ptr::write_volatile(0xB800D as *mut u8, 0x0C); // Bright red
            }
        }

        let shell_entry = crate::user_program::shell::shell_main as u64;
        if let Some(shell_pid) = pt.spawn(shell_entry, "shell") {
            vga::serial_print("Shell PID 2: READY\n");
            pt.set_ready(shell_pid);
            // Write 'S' for Shell
            unsafe {
                core::ptr::write_volatile(0xB800E as *mut u8, b'S');
                core::ptr::write_volatile(0xB800F as *mut u8, 0x0C);
            }
        }
    }

    // Write '5' to VGA after process creation
    unsafe {
        core::ptr::write_volatile(0xB8010 as *mut u8, b'5');
        core::ptr::write_volatile(0xB8011 as *mut u8, 0x0A);
    }

    // ============================================================================
    // STEP 4a: REMAP PIC (IRQ0→0x20, IRQ1→0x21, …)
    // ============================================================================
    //
    // By default the PIC maps IRQ0 to vector 0x08 (Double Fault!). We must remap
    // both master and slave PICs so that hardware interrupts don't collide with
    // CPU exceptions.
    unsafe {
        remap_pic();
    }

    // ============================================================================
    // STEP 4b: CONFIGURE TIMER
    // ============================================================================
    setup_timer();

    // Write 'T' for Timer
    unsafe {
        core::ptr::write_volatile(0xB8012 as *mut u8, b'T');
        core::ptr::write_volatile(0xB8013 as *mut u8, 0x0A);
    }

    // Unmask keyboard IRQ1 in the master PIC (clear bit 1)
    // Also ensure timer IRQ0 (bit 0) remains unmasked.
    unsafe {
        core::arch::asm!(
            "in al, 0x21",      // Read current mask
            "and al, 0xFC",     // Clear bits 0 (timer) and 1 (keyboard)
            "out 0x21, al",     // Write back — all other IRQs masked
            options(nostack),
        );
    }

    vga::serial_print("\n[OK] IDT configured\n");
    vga::serial_print("[OK] Timer configured\n");
    vga::serial_print("\nSwitching to user space...\n");

    // Write 'U' for User mode
    unsafe {
        core::ptr::write_volatile(0xB8014 as *mut u8, b'U');
        core::ptr::write_volatile(0xB8015 as *mut u8, 0x0A);
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
/// PIC REMAP
/// ============================================================================
///
/// Remaps the 8259A PIC (Programmable Interrupt Controller) so hardware IRQs
/// don't overlap with CPU exception vectors (0-31).
///
/// Default: IRQ0→0x08 (DOUBLE FAULT!), IRQ1→0x09, ...
/// After:   IRQ0→0x20, IRQ1→0x21, ... (safe zone)
///
/// # Safety
///
/// Writes to I/O ports 0x20/0x21 (master PIC) and 0xA0/0xA1 (slave PIC).
unsafe fn remap_pic() {
    // Save current masks (they may be set by BIOS)
    let mask_master: u8;
    let mask_slave: u8;
    core::arch::asm!(
        "in al, 0x21",
        "mov {m}, al",
        "in al, 0xA1",
        "mov {s}, al",
        m = out(reg_byte) mask_master,
        s = out(reg_byte) mask_slave,
        options(nostack),
    );

    // Start init sequence (ICW1) — both PICs
    core::arch::asm!(
        "mov al, 0x11",     // ICW4 needed, cascade mode
        "out 0x20, al",     // master PIC command port
        "out 0xA0, al",     // slave PIC command port
        options(nostack),
    );

    // ICW2 — vector bases
    core::arch::asm!(
        "mov al, 0x20",     // master base: IRQ0→INT 0x20
        "out 0x21, al",
        "mov al, 0x28",     // slave base:  IRQ8→INT 0x28
        "out 0xA1, al",
        options(nostack),
    );

    // ICW3 — cascade wiring
    core::arch::asm!(
        "mov al, 0x04",     // master: slave on IRQ2 (bit 2)
        "out 0x21, al",
        "mov al, 0x02",     // slave:  cascade identity
        "out 0xA1, al",
        options(nostack),
    );

    // ICW4 — environment
    core::arch::asm!(
        "mov al, 0x01",     // 8086 mode
        "out 0x21, al",
        "out 0xA1, al",
        options(nostack),
    );

    // Restore masks — keep everything masked for now, unmask timer later
    core::arch::asm!(
        "mov al, {m}",
        "out 0x21, al",
        "mov al, {s}",
        "out 0xA1, al",
        m = in(reg_byte) mask_master,
        s = in(reg_byte) mask_slave,
        options(nostack),
    );
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
