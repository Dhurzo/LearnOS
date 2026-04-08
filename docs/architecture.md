# Architecture

## Overview

This kernel is a `no_std` x86_64 Rust binary booted by QEMU through the PVH path (`-kernel`).
The assembly bootstrap (`kernel/src/boot.S`) starts in 32-bit protected mode, enables long mode, then transfers control to Rust `_start` in 64-bit mode.

Runtime model:

1. PVH loader enters `pvh_start`
2. Bootstrap configures stack, paging, and long mode
3. Control jumps to Rust `_start`
4. Rust initializes screen state and runs an interactive keyboard loop

## Source Files

| File | Responsibility |
|------|----------------|
| `kernel/src/boot.S` | PVH entry note + 32-bit to 64-bit bootstrap |
| `kernel/src/main.rs` | Kernel control flow, prompt/echo loop, panic handler |
| `kernel/src/vga.rs` | VGA text buffer writer and cursor state |
| `kernel/src/keyboard.rs` | PS/2 polling and scan-code decoding |
| `kernel/src/linker.ld` | ELF layout, section order, load addresses |

## Boot Pipeline

### 1) PVH Note Discovery

`boot.S` exposes a Xen ELF note in `.note.Xen`:

- Name: `"Xen"`
- Type: `18` (`XEN_ELFNOTE_PHYS32_ENTRY`)
- Value: address of `pvh_start`

QEMU uses this note to choose the physical 32-bit entrypoint.

### 2) 32-bit Bootstrap (`pvh_start`)

Bootstrap performs early CPU setup that Rust cannot assume exists:

- disables interrupts (`cli`)
- sets a temporary bootstrap stack
- loads identity-mapped page tables into `cr3`
- enables PAE (`cr4.PAE`)
- enables long mode via `IA32_EFER.LME`
- enables paging + protected mode bits in `cr0`
- loads 64-bit GDT and far-jumps to `long_mode_start`

### 3) 64-bit Handoff (`long_mode_start`)

The bootstrap then:

- loads flat data segment selectors
- sets 64-bit stack pointer
- calls Rust `_start`

If `_start` ever returns, bootstrap halts forever (`hlt` loop).

## Memory Layout

Defined in `kernel/src/linker.ld`:

- low region (from address 0): PVH metadata
  - `.note.pvh` (contains `.note.Xen` note)
- executable region from `0x200000`:
  - `.text` and `.rodata`
  - `.data`
  - `.bss`

Bootstrap paging identity-maps at least:

- `0x00000000..0x001FFFFF` (includes VGA memory `0xB8000`)
- `0x00200000..0x003FFFFF` (kernel region)

## Rust Runtime Behavior

`_start` in `kernel/src/main.rs`:

1. clears VGA text screen
2. prints greeting + prompt
3. enters infinite line-input loop

Loop behavior:

- printable keys: append to buffer and render
- backspace: remove one byte and erase one cell
- enter: print `Echo: <line>`, reset buffer, print prompt

## I/O Model

### VGA (Memory-Mapped I/O)

- base address: `0xB8000`
- cell format: `[ascii][attribute]`
- screen: 80x25
- writes are volatile to preserve device side effects

### Keyboard (Port-Mapped I/O)

- status port: `0x64`
- data port: `0x60`
- waits for output-buffer-full bit
- decodes scan-code set 1 make codes into high-level events

## Design Choices

- **No allocator**: fixed-size input buffer
- **No interrupts yet**: polling for deterministic early bring-up
- **No scheduler/processes**: single control loop
- **No filesystem/drivers**: focus on boot + console fundamentals

This makes the system small and auditable while keeping the critical path (boot to interactive VGA/keyboard) explicit.
