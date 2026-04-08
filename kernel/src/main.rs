//! Operating System Kernel - PVH + VGA + PS/2 Echo
//!
//! This crate contains the Rust runtime portion of a minimal x86_64 kernel.
//! QEMU enters through the PVH bootstrap in `boot.S`, which switches CPU mode
//! to 64-bit long mode and then calls this file's `_start` function.
//!
//! Runtime flow in this module:
//! 1. Clear VGA text screen
//! 2. Print greeting and prompt
//! 3. Poll PS/2 keyboard controller
//! 4. Echo submitted line back to VGA
//!
//! The implementation intentionally avoids allocators, interrupts, and schedulers
//! to keep early kernel bring-up explicit and auditable.

#![no_std] // Don't use the standard library
#![no_main] // Don't use the main function

use core::panic::PanicInfo;

mod keyboard;
mod vga;

core::arch::global_asm!(include_str!("boot.S"));

/// Kernel entry point
///
/// This is the first function called by the bootloader (or PVH hypervisor).
/// The function signature must match what the bootloader expects.
///
/// # Safety
/// This function is unsafe because it directly accesses memory and never returns.
/// It must be called only once during kernel initialization.
///
/// # Runtime Behavior
///
/// 1. Clears the VGA screen.
/// 2. Prints a small interactive prompt.
/// 3. Enters `kernel_loop`, which performs blocking keyboard reads.
///
/// The input buffer is fixed-size to avoid heap allocation in `no_std`.
#[no_mangle] // Don't mangle the symbol name
pub extern "C" fn _start() -> ! {
    const MAX_INPUT: usize = 128;

    let mut input = [0u8; MAX_INPUT];
    let mut len = 0usize;

    vga::clear_screen();
    vga::print_vga("Hello World!\n");
    vga::print_vga("Keyboard echo demo\n");
    vga::print_vga("Type text and press Enter\n");
    vga::print_vga("> ");

    kernel_loop(&mut input, &mut len)
}

/// Main interactive loop for keyboard input and line echo.
///
/// This loop is intentionally synchronous and polling-based:
/// - waits for a decoded key event,
/// - updates the input line state,
/// - renders immediate user feedback to VGA.
///
/// # Parameters
/// - `input`: fixed-size byte buffer storing the current line
/// - `len`: number of valid bytes currently stored in `input`
///
/// # Event Handling
/// - `Char`: appends to buffer and writes the character to screen
/// - `Backspace`: removes one byte from buffer and erases one screen cell
/// - `Enter`: prints `Echo: <line>`, resets input, and prints a new prompt
///
/// # Safety Model
///
/// This function itself is safe Rust. Low-level unsafety is encapsulated in:
/// - `keyboard` module (port I/O)
/// - `vga` module (memory-mapped video buffer writes)
fn kernel_loop(input: &mut [u8], len: &mut usize) -> ! {
    loop {
        match keyboard::read_key_blocking() {
            keyboard::KeyEvent::Char(byte) => {
                if *len < input.len() {
                    input[*len] = byte;
                    *len += 1;
                    vga::write_byte(byte);
                }
            }
            keyboard::KeyEvent::Backspace => {
                if *len > 0 {
                    *len -= 1;
                    vga::backspace();
                }
            }
            keyboard::KeyEvent::Enter => {
                vga::print_vga("\nEcho: ");
                for &byte in &input[..*len] {
                    vga::write_byte(byte);
                }
                vga::print_vga("\n> ");
                *len = 0;
            }
        }
    }
}

/// Panic handler for the kernel
///
/// This function is called when a panic occurs. In a minimal kernel,
/// we simply loop indefinitely. In a more advanced kernel, this might
/// dump debug information and attempt to reboot.
///
/// # Arguments
/// * `_info` - Information about the panic (unused in this minimal implementation)
///
/// # Safety
/// This function is unsafe and never returns.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // In a real kernel, we might want to:
    // 1. Print panic information to screen
    // 2. Dump registers/memory
    // 3. Attempt system reboot
    // 4. Halt the system

    // For now, just loop indefinitely
    loop {}
}
