/*
 * PVH Bootstrap for LearnOS Microkernel
 * ==============================
 *
 * This file is the first code executed when the kernel boots. It runs in 32-bit 
 * protected mode and sets up the CPU for 64-bit long mode before jumping 
 * to Rust code.
 */

    /*
     * =========================================================================
     * XEN NOTE SECTION
     * =========================================================================
     */
.section .note.Xen,"a",@note
.align 4
.long 4
.long 4
.long 18
.asciz "Xen"
.align 4
.long pvh_start

    /*
     * =========================================================================
     * 32-BIT BOOTSTRAP CODE
     * =========================================================================
     */
.section .text.boot,"ax"
.code32

.global pvh_start
pvh_start:
    cli
    mov esp, offset boot_stack_top
    mov eax, offset page_table_l4
    mov cr3, eax
    mov eax, cr4
    or eax, 0x20
    mov cr4, eax
    mov ecx, 0xC0000080
    rdmsr
    or eax, 0x00000100
    wrmsr
    mov eax, cr0
    or eax, 0x80000001
    mov cr0, eax
    lgdt gdt64_ptr
    .byte 0xEA
    .long long_mode_start
    .word 0x08

    /*
     * =========================================================================
     * 64-BIT MODE ENTRY POINT
     * =========================================================================
     */
.code64
long_mode_start:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax
    mov rsp, offset boot_stack_top
    call _start
1:  hlt
    jmp 1b

    /*
     * =========================================================================
     * GLOBAL DESCRIPTOR TABLE (GDT)
     * =========================================================================
     */
.align 8
gdt64:
    .quad 0x0000000000000000
    .quad 0x00AF9A000000FFFF
    .quad 0x00AF92000000FFFF
gdt64_end:

.align 4
gdt64_ptr:
    .word gdt64_end - gdt64 - 1
    .long gdt64

    /*
     * =========================================================================
     * PAGE TABLES
     * =========================================================================
     */
.align 4096
page_table_l4:
    .quad page_table_l3 + 0x003
    .fill 511, 8, 0

.align 4096
page_table_l3:
    .quad page_table_l2 + 0x003
    .fill 511, 8, 0

.align 4096
page_table_l2:
    // Map first 1GB (512 entries * 2MB each)
    .quad 0x0000000000000083      // 0x00000000 - 0x200000
    .quad 0x0000000002000083      // 0x0200000 - 0x400000
    .quad 0x0000000004000083      // 0x0400000 - 0x600000
    .quad 0x0000000006000083      // 0x0600000 - 0x800000
    .quad 0x0000000008000083      // 0x0800000 - 0xA00000
    .quad 0x000000000A000083      // 0x0A00000 - 0xC00000
    .quad 0x000000000C000083      // 0x0C00000 - 0xE00000
    .quad 0x000000000E000083      // 0x0E00000 - 0x1000000
    .fill 504, 8, 0

    /*
     * =========================================================================
     * BOOTSTRAP STACK
     * =========================================================================
     */
.align 16
boot_stack:
    .space 16384
boot_stack_top: