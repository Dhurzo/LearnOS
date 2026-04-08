/*
 * Linker Script for PVH ELF Kernel
 *
 * This kernel is produced as ELF64 (ET_EXEC). QEMU discovers the PVH entry
 * through the Xen note emitted by `boot.S` and then jumps to that 32-bit
 * physical entrypoint.
 *
 * Layout strategy:
 * - keep PVH metadata note in low region
 * - place executable/data sections at 2 MiB
 * - align key sections to page boundaries
 */

/* Entry point - first function executed when kernel boots */
ENTRY(_start)

/* ELF64 output consumed directly by QEMU `-kernel` */
OUTPUT_FORMAT(elf64-x86-64)
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
    . = 0;

    /* Xen PVH note section. `boot.S` contributes `.note.Xen`. */
    .note.pvh : ALIGN(4) {
        KEEP(*(.note.Xen))
        KEEP(*(.note.pvh))
    }

    /* Load executable sections at 2 MiB.
     * This avoids low-memory firmware regions and matches bootstrap mappings.
     */
    . = 2M;

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
     * - Follows .note.pvh section
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
