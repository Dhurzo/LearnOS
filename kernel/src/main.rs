//! Operating System Kernel - Hello World Implementation
//!
//! This is a minimal x86_64 kernel written in Rust that demonstrates
//! fundamental operating system kernel development concepts. The kernel
//! implements a complete boot sequence from power-on to user output using
//! modern kernel development techniques.
//!
//! ## Architecture Overview
//!
//! The kernel consists of several key components working together:
//!
//! ### Boot Sequence
//! 1. **PVH Entry**: Modern QEMU boots the kernel at 2MB using PVH (Para-Virtualized Hypervisor)
//!    mode, which provides direct hardware access without BIOS emulation
//! 2. **Entry Point**: The `_start()` function is called by the bootloader/QEMU
//! 3. **VGA Output**: Direct memory writes to 0xb8000 display "Hello World!" message
//! 4. **Kernel Loop**: Infinite loop to prevent kernel exit and maintain execution
//!
//! ### Technical Components
//!
//! - **no_std Compilation**: Builds without Rust standard library for minimal footprint
//! - **PVH Support**: ELF note structure for modern QEMU compatibility
//! - **Multiboot2 Headers**: Fallback compatibility for traditional bootloaders
//! - **VGA Text Mode**: Direct video memory access at 0xb8000
//! - **Memory Management**: Carefully controlled memory layout via linker script
//! - **Panic Handling**: Proper kernel panic handler to prevent undefined behavior
//!
//! ## Memory Layout
//!
//! The kernel follows a strict memory layout defined by the linker script:
//!
//! ```
//! 0x200000 (2MB) Kernel Entry Point (_start)
//! ├── .note.pvh    : 20 bytes   - PVH identification for QEMU
//! ├── .multiboot_header : 16 bytes  - Fallback boot compatibility  
//! ├── .text        : ~200 bytes  - Executable code
//! ├── .rodata      : ~20 bytes   - Read-only data (strings)
//! ├── .data        : 0 bytes    - Initialized global variables (none in this kernel)
//! └── .bss         : 0 bytes    - Zero-initialized variables (none in this kernel)
//! ```
//!
//! ## Boot Methods
//!
//! ### PVH Mode (Recommended)
//! - Direct kernel loading without BIOS emulation
//! - Faster boot, better hardware access
//! - Required for modern QEMU versions
//! - Uses PVH ELF note in .note.pvh section
//!
//! ### Boot Sector Mode (Fallback)
//! - Traditional BIOS-based boot from floppy disk
//! - SeaBIOS initializes hardware, then loads kernel
//! - Provides maximum compatibility
//! - Implemented in NASM assembly (boot.asm)
//!
//! ## Development Notes
//!
//! This kernel serves as an educational foundation for OS development.
//! It demonstrates the absolute minimum required for a functional kernel
//! and provides a solid foundation for adding more advanced features
//! like memory management, interrupt handling, and device drivers.
//!
//! For more information about kernel development:
//! - [The Rust Embedded Book](https://docs.rust-embedded.org/book/)
//! - [OSDev Wiki](https://wiki.osdev.org/)
//! - [QEMU Documentation](https://www.qemu.org/docs/)
//!
//! ## Key Concepts Demonstrated
//!
//! 1. **Entry Point**: `_start()` function with proper calling convention
//! 2. **Memory Access**: Direct VGA buffer manipulation using unsafe Rust
//! 3. **Kernel Compilation**: no_std + build-std for minimal dependencies
//! 4. **Boot Compatibility**: PVH notes for modern, boot sector for legacy
//! 5. **Error Handling**: Panic handler prevents kernel crashes
//!
//! ## Expected Behavior
//!
//! When running in QEMU, the kernel should:
//! 1. Boot successfully using PVH or boot sector method
//! 2. Display "Hello World!" message on screen via VGA buffer
//! 3. Continue running indefinitely until QEMU timeout or manual stop
//! 4. No panics or crashes during execution
//!
//! The kernel output should be visible regardless of the boot method,
//! though the specific sequence may differ slightly between PVH and boot sector modes.

#![no_std] // Don't use the standard library
#![no_main] // Don't use the main function

use core::panic::PanicInfo;

mod vga;

core::arch::global_asm!(include_str!("boot.S"));

/// PVH (Para-Virtualized Hypervisor) note for modern QEMU support
///
/// This structure provides PVH support which allows the kernel to boot
/// efficiently in modern QEMU virtual machines. PVH eliminates the need
/// for BIOS emulation and provides direct hardware access.
///
/// Format: ELF Note with type NT_PVH (1)
#[repr(C)] // Ensure C-style layout in memory
pub(crate) struct Elf64Note {
    namesz: u32,    // Size of the name field (including null terminator)
    descsz: u32,    // Size of the descriptor field
    type_: u32,     // Note type (1 = NT_PVH)
    name: [u8; 4],  // Name field (must be null-terminated)
    data: [u8; 16], // PVH specific data
}

/// PVH note instance
///
/// This static variable contains the PVH note that QEMU looks for
/// when booting a kernel with PVH support. The note includes:
/// - PVH signature for identification
/// - Load offset where the kernel expects to be loaded
#[link_section = ".note.pvh"]
#[no_mangle] // Don't mangle the symbol name
static PVH_NOTE: Elf64Note = Elf64Note {
    namesz: 4,                   // Size of "PHV\0"
    descsz: 16,                  // Size of PVH data
    type_: 1,                    // NT_PVH type
    name: [b'P', b'H', b'V', 0], // "PHV\0"
    data: [
        0x00, 0x00, 0x00, 0x01, // PVH_FEATURE (bit 0 = 64-bit PVH)
        0x00, 0x00, 0x00, 0x00, // PVH_FEATURE (reserved)
        0x20, 0x00, 0x00, 0x00, // PVH_KERNEL_ADDR (load address)
        0x00, 0x00, 0x00, 0x00, // PVH_KERNEL_SIZE (kernel size, 0 for auto)
    ],
};

/// Kernel entry point
///
/// This is the first function called by the bootloader (or PVH hypervisor).
/// The function signature must match what the bootloader expects.
///
/// # Safety
/// This function is unsafe because it directly accesses memory and never returns.
/// It must be called only once during kernel initialization.
#[no_mangle] // Don't mangle the symbol name
pub extern "C" fn _start() -> ! {
    // Print "Hello World!" to the VGA buffer
    vga::print_vga("Hello World!");

    // Infinite loop to keep the kernel running
    // In a real kernel, this would be replaced with proper scheduling
    loop {}
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
