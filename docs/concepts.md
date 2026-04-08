# OS Kernel Concepts

This document explains the core low-level concepts exercised by this repository's current kernel path: **PVH boot + long-mode bootstrap + VGA/PS2 interaction**.

## 1) `no_std` Kernel Model

The kernel is compiled with:

```rust
#![no_std]
#![no_main]
```

Meaning:

- no Rust standard library runtime
- no default `main`
- explicit entry symbol (`_start`)
- explicit panic handler

The binary is fully self-contained and expects no host OS services.

## 2) PVH Entry vs Rust ABI

QEMU PVH entry (Xen note type `PHYS32_ENTRY`) transfers control in **32-bit protected mode**. Rust kernel code targets **x86_64 long mode**.

Therefore this project includes an assembly bridge:

- `pvh_start` (32-bit)
- `long_mode_start` (64-bit)
- `call _start` (Rust)

Without this bridge, Rust instructions execute under the wrong CPU mode and the kernel appears to hang or behave randomly.

## 3) CPU Mode Transition Essentials

The bootstrap enables long mode with these architectural steps:

1. Build/load page tables (`cr3`)
2. Enable PAE (`cr4.PAE`)
3. Set `IA32_EFER.LME` via `rdmsr/wrmsr`
4. Enable paging (`cr0.PG`) and keep protected mode active
5. Load a 64-bit GDT
6. Perform far control transfer to 64-bit code selector

Only after step 6 is 64-bit instruction decoding guaranteed.

## 4) Identity Mapping Strategy

The bootstrap maps low memory and kernel memory identity-wise to keep early access straightforward:

- low region includes VGA memory (`0xB8000`)
- kernel region includes `.text/.data/.bss` around `0x200000`

Identity mapping avoids early virtual/physical translation complexity during bring-up.

## 5) VGA Text Buffer I/O

VGA text mode memory:

- base physical address: `0xB8000`
- geometry: 80 columns × 25 rows
- each cell: 2 bytes (`ascii`, `attribute`)

Example conceptual write:

```text
0xB8000 <- 'H'
0xB8001 <- 0x0F  (bright white on black)
```

The writer uses volatile memory stores so compiler optimizations do not remove device writes.

## 6) Keyboard Polling Model

PS/2 controller ports:

- status: `0x64`
- data: `0x60`

Loop:

1. read `0x64`
2. wait until output-buffer-full bit is set
3. read scan code from `0x60`
4. ignore break codes (key release)
5. decode make code into `Char`, `Backspace`, `Enter`

This is polling, not IRQ-driven input.

## 7) Kernel Control Loop

In Rust `_start`:

1. clear screen
2. print greeting and prompt
3. enter `kernel_loop`

`kernel_loop` maintains a fixed-size input line buffer:

- append on char
- erase on backspace
- echo full line on enter

No allocator is required.

## 8) Linker Responsibilities

`kernel/src/linker.ld` ensures:

- note/header sections placed where QEMU can parse them
- executable sections placed at expected kernel load addresses
- section alignment suitable for paging and CPU fetch behavior

A kernel can compile successfully but still fail to boot if linker placement is wrong.

## 9) Why You See "Booting from ROM.."

QEMU still initializes firmware devices first, so ROM boot text appears before the PVH handoff. That text alone does not indicate failure.

Actual success criteria is whether execution reaches Rust `_start` and the VGA prompt appears.

## 10) Incremental Next Concepts (Future)

Natural evolution paths from this baseline:

- IDT + exception handlers
- PIC/APIC interrupt setup
- IRQ1 keyboard interrupt driver (replace polling)
- serial debug channel in parallel with VGA
- physical memory map parsing and frame allocator

Current code is intentionally minimal so each of these additions remains understandable and testable.
