/*
 * PVH Bootstrap (x86)
 *
 * Purpose:
 * - Expose a Xen PVH ELF note so QEMU can locate our physical 32-bit entrypoint
 * - Run minimal 32-bit setup required for long mode
 * - Transfer control to Rust `_start` in 64-bit mode
 *
 * High-level sequence:
 * 1) pvh_start (32-bit): set stack, set page tables, enable PAE + LME + PG
 * 2) load 64-bit GDT and far transfer to 64-bit code segment
 * 3) long_mode_start (64-bit): load data segments, set RSP, call `_start`
 */

.section .note.Xen,"a",@note
.align 4
/* namesz=4, descsz=4, type=18 (XEN_ELFNOTE_PHYS32_ENTRY) */
.long 4
.long 4
.long 18
.asciz "Xen"
.align 4
/* 32-bit physical entrypoint used by QEMU PVH loader */
.long pvh_start

.section .text.boot,"ax"
.code32
.global pvh_start
pvh_start:
    cli

    /* Temporary bootstrap stack in low mapped memory */
    mov esp, offset boot_stack_top

    /* Load identity-mapped page tables */
    mov eax, offset page_table_l4
    mov cr3, eax

    /* Enable PAE (required precursor for IA-32e paging) */
    mov eax, cr4
    or eax, 0x20
    mov cr4, eax

    /* Enable Long Mode (EFER.LME) */
    mov ecx, 0xC0000080
    rdmsr
    or eax, 0x00000100
    wrmsr

    /* Enable protected mode + paging */
    mov eax, cr0
    or eax, 0x80000001
    mov cr0, eax

    /* Load 64-bit GDT and far transfer to 64-bit code selector */
    lgdt gdt64_ptr
    .byte 0xEA
    .long long_mode_start
    .word 0x08

.code64
long_mode_start:
    /* Load flat data segments */
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax

    /* Switch to 64-bit stack and handoff to Rust */
    mov rsp, offset boot_stack_top
    call _start

1:
    /* `_start` is ! (never returns), but halt defensively if it does */
    hlt
    jmp 1b

.align 8
/* Minimal 64-bit GDT: null, code64, data */
gdt64:
    .quad 0x0000000000000000
    .quad 0x00AF9A000000FFFF
    .quad 0x00AF92000000FFFF
gdt64_end:

.align 4
gdt64_ptr:
    .word gdt64_end - gdt64 - 1
    .long gdt64

.align 4096
/* PML4[0] -> L3 */
page_table_l4:
    .quad page_table_l3 + 0x003
    .fill 511, 8, 0

.align 4096
/* L3[0] -> L2 */
page_table_l3:
    .quad page_table_l2 + 0x003
    .fill 511, 8, 0

.align 4096
/* Two 2 MiB identity mappings:
 * - 0x00000000..0x001FFFFF (includes VGA memory at 0xB8000)
 * - 0x00200000..0x003FFFFF (kernel region)
 */
page_table_l2:
    .quad 0x0000000000000083
    .quad 0x0000000000200083
    .fill 510, 8, 0

.align 16
boot_stack:
    .space 16384
boot_stack_top:
