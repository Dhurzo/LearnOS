/*
 * Linker Script for PVH x86_64 Kernel
 * 
 * This script configures the kernel's memory layout and section organization
 * for direct PVH (Para-Virtualized Hypervisor) boot with QEMU.
 * 
 * Key Concepts:
 * - PVH Boot: Modern QEMU boot method that eliminates BIOS emulation
 * - ELF to Raw Binary: Converts ELF executable to raw binary without headers
 * - Memory Layout: Organizes sections at specific memory addresses
 * - Section Ordering: Critical for QEMU to recognize PVH and multiboot headers
 * 
 * Technical Details:
 * - Entry Point: _start function (first code executed by kernel)
 * - Load Address: 2MB (0x200000) - avoids bootloader and low memory conflicts
 * - Output Format: Binary (not ELF) for direct memory loading
 * - Alignment: 4KB alignment for sections to optimize memory access
 * 
 * PVH Requirements:
 * - .note.pvh section must be at file beginning
 * - PVH signature identifies kernel for modern QEMU boot
 * - No ELF headers in final binary (stripped during build)
 * 
 * Multiboot Fallback:
 * - .multiboot_header section for backward compatibility
 * - Allows booting with older multiboot loaders
 * - Placed after PVH note in memory
 *
 * For more information:
 * - PVH Specification: https://www.qemu.org/docs/master/system/qemu-manpage.html
 * - Multiboot Specification: https://www.gnu.org/software/grub/manual/multiboot/multiboot.html
 * - ELF Specification: https://refspecs.linuxfoundation.org/elf/elfspec64.pdf
 */

/* Entry point - first function executed when kernel boots */
ENTRY(_start)

/* Set output format to binary (no ELF headers for PVH boot) */
OUTPUT_FORMAT(binary)
OUTPUT_ARCH(i386:x86-64)

/* Set memory layout
 *
 * Memory Address Space:
 * - 0x00000000-0x0009FFFF: BIOS/Bootloader area (avoid)
 * - 0x000A0000-0x000BFFFF: VGA Video Memory (don't use)
 * - 0x000C0000-0x000FFFFF: BIOS ROM Area (avoid)
 * - 0x00100000-0x001FFFFF: Available for PVH kernel (load here)
 * - 0x00200000+: Higher memory areas (also available)
 *
 * We load at 2MB (0x200000) to:
 * 1. Avoid conflicts with bootloader (first 1MB)
 * 2. Avoid conflicts with BIOS data areas
 * 3. Provide proper alignment for memory access
 * 4. Follow QEMU PVH best practices
 */
SECTIONS {
    /* Load at 2MB (0x200000) for PVH boot
     * 
     * This positioning is critical for PVH operation:
     * - Avoids low memory conflicts with BIOS and bootloader
     * - Provides proper alignment for memory access
     * - Follows QEMU PVH specification requirements
     * - Leaves room for bootloader operations (first 1MB)
     */
    . = 2M;

    /* PVH Note Section - Critical for QEMU PVH Recognition
     * 
     * Purpose: Identifies this kernel as supporting PVH boot format
     * Format: ELF Note structure with type NT_PVH (0x1)
     * Size: 4-byte header + 16-byte data = 20 bytes total
     * 
     * PVH Note Structure:
     * - namesz: 4 (size of "PHV\0" including null terminator)
     * - descsz: 16 (size of PVH specific data)
     * - type: 1 (NT_PVH note type)
     * - name: "PHV\0" (4 bytes, null-terminated)
     * - data: PVH configuration data (16 bytes)
     * 
     * Data Fields:
     * - PVH_FEATURE: Feature flags (bit 0 = 64-bit support)
     * - PVH_KERNEL_ADDR: Physical address where kernel expects to load
     * - PVH_KERNEL_SIZE: Size of kernel (0 for auto-detection)
     * 
     * Important: This section MUST come first in the file for QEMU
     * to recognize the PVH format and load the kernel correctly.
     */
    .note.pvh : ALIGN(4) {
        KEEP(*(.note.pvh))  /* Keep this section even if it appears unused */
    } :note

    /* Multiboot2 Header Section - Fallback for Compatibility
     * 
     * Purpose: Allows booting with traditional multiboot loaders
     * Format: Standard multiboot2 header structure
     * Size: 16 bytes (minimum required by spec)
     * 
     * Multiboot2 Header Structure:
     * - magic: 0xe85250d6 (multiboot2 signature)
     * - architecture: 4 (x86_64 architecture)
     * - header_length: 16 (size of this header in bytes)
     * - checksum: 0 (calculated as -(magic + arch + length))
     * - flags: 0 (no optional features enabled)
     * 
     * Placement: After PVH note to maintain compatibility
     * This header is only used if PVH boot fails or
     * when using multiboot-compatible bootloaders.
     */
    .multiboot_header : ALIGN(4) {
        KEEP(*(.multiboot_header))  /* Preserve for multiboot compatibility */
    } :note

    /* Text Section - Executable Code
     * 
     * Purpose: Contains all the executable code of the kernel
     * Alignment: 4KB boundary for optimal memory access performance
     * Contents:
     * - Function implementations (main, print_vga, etc.)
     * - Read-only data (string literals, constants)
     * - ELF sections: .text, .text.*, .rodata, .rodata.*
     * 
     * Memory Layout:
     * - Follows .note.pvh and .multiboot_header sections
     * - Aligned to 4KB boundary for cache efficiency
     * - Contains all kernel executable code
     * - Includes read-only constants like messages
     */
    .text : ALIGN(4K) {
        *(.text .text.*)      /* All text sections from object files */
        *(.rodata .rodata.*)  /* Read-only data (constants, strings) */
        . = ALIGN(4096);      /* Align to 4KB boundary */
    }

    /* Data Section - Initialized Writable Data
     * 
     * Purpose: Contains initialized global and static variables
     * Alignment: 4KB boundary for memory access efficiency
     * Contents:
     * - Initialized global variables
     * - Static variables with initial values
     * - ELF sections: .data, .data.*
     * 
     * Memory Layout:
     * - Follows .text section in memory
     * - Aligned to 4KB boundary for cache line optimization
     * - Contains variables that need explicit initialization
     * - Will be copied from kernel binary to memory by bootloader
     */
    .data : ALIGN(4K) {
        *(.data .data.*)      /* All data sections from object files */
        . = ALIGN(4096);      /* Align to 4KB boundary */
    }

    /* BSS Section - Zero-initialized Data
     * 
     * Purpose: Contains uninitialized global and static variables
     * Alignment: 4KB boundary for consistent memory layout
     * Contents:
     * - Uninitialized global variables
     * - Static variables without initial values
     * - Common symbols (uninitialized globals)
     * - ELF sections: .bss, .bss.*, COMMON
     * 
     * Memory Layout:
     * - Follows .data section in memory
     * - Aligned to 4KB boundary for consistency
     * - Will be zeroed by bootloader before kernel execution
     * - Does not occupy space in kernel binary (just size info)
     * - Important: This section MUST be properly zeroed before kernel
     * execution to prevent undefined behavior in global variables.
     */
    .bss : ALIGN(4K) {
        *(.bss .bss.*)        /* All BSS sections from object files */
        *(COMMON)             /* Common symbols (uninitialized globals) */
        . = ALIGN(4096);      /* Align to 4KB boundary */
    }

    /* End Marker - Optional but Good Practice
     * 
     * Purpose: Provides a clean end boundary for the kernel image
     * Alignment: 8-byte boundary for memory address alignment
     * 
     * This ensures proper alignment for any post-processing tools
     * and provides a clear end marker for the kernel binary.
     */
    . = ALIGN(8);
}