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
     *
     * Layout (each entry is 8 bytes):
     *   [0] Null descriptor                              → selector 0x00
     *   [1] Ring 0 Code (64-bit, DPL=0)                  → selector 0x08
     *   [2] Ring 0 Data (DPL=0)                          → selector 0x10
     *   [3] Ring 3 Code (64-bit, DPL=3)                  → selector 0x18 + RPL3 = 0x1B
     *   [4] Ring 3 Data (DPL=3)                          → selector 0x20 + RPL3 = 0x23
     *   [5] TSS descriptor (low 8 bytes)                 → selector 0x28
     *   [6] TSS descriptor (high 8 bytes)                → selector 0x30
     *
     * Entries [3] and [4] are used by IRETQ when returning
     * to ring-3 user space. SYSCALL/SYSRET hardcode their
     * own descriptor caches (Intel SDM Vol 2B).
     *
     * Entry [5]+[6] form the 16-byte system descriptor for
     * the Task State Segment. The actual TSS data structure
     * is defined in Rust (tss.rs).
     */
.global gdt64
gdt64:
    .quad 0x0000000000000000       /* 0x00: Null */
    .quad 0x00AF9A000000FFFF       /* 0x08: Ring 0 Code (R/X) */
    .quad 0x00AF92000000FFFF       /* 0x10: Ring 0 Data (R/W) */
    .quad 0x00AFFA000000FFFF       /* 0x18: Ring 3 Code (R/X, DPL=3) */
    .quad 0x00AFF2000000FFFF       /* 0x20: Ring 3 Data (R/W, DPL=3) */
    .quad 0x0000000000000000       /* 0x28: TSS desc low (Rust fills in) */
    .quad 0x0000000000000000       /* 0x30: TSS desc high (Rust fills in) */
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
    .quad page_table_l3 + 0x007
    .fill 511, 8, 0

.align 4096
page_table_l3:
    .quad page_table_l2 + 0x007
    .fill 511, 8, 0

.align 4096
page_table_l2:
    // Map first 1GB (512 entries * 2MB each)
    // NOTE: We use 0x087 (Present | Writable | User | Large) so user-space
    // programs can execute from these pages when we drop to ring 3.
    // In a production kernel, user code would live on separate pages with
    // the User bit set selectively. For this educational microkernel, we
    // set it broadly and tighten it in T5 (per-process page tables).
    .quad 0x0000000000000087      // 0x00000000 - 0x200000
    .quad 0x0000000002000087      // 0x0200000 - 0x400000
    .quad 0x0000000004000087      // 0x0400000 - 0x600000
    .quad 0x0000000006000087      // 0x0600000 - 0x800000
    .quad 0x0000000008000087      // 0x0800000 - 0xA00000
    .quad 0x000000000A000087      // 0x0A00000 - 0xC00000
    .quad 0x000000000C000087      // 0x0C00000 - 0xE00000
    .quad 0x000000000E000087      // 0x0E00000 - 0x1000000
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