# Architecture

## Overview

This is a minimal x86_64 operating system kernel written in Rust, targeting a `no_std` bare-metal environment. The kernel boots via PVH (modern QEMU) or a traditional BIOS boot sector, writes "Hello World!" directly to the VGA text buffer, and halts in an infinite loop.

There are no external dependencies, no allocator, and no standard library. The kernel logic is split across two Rust files (`kernel/src/main.rs` and `kernel/src/vga.rs`) plus assembly boot stubs and a linker script.

## Source Files

| File | Purpose |
|------|---------|
| `kernel/src/main.rs` | Kernel entry point, PVH note, panic handler |
| `kernel/src/vga.rs` | VGA text-mode writing logic |
| `kernel/src/boot.asm` | 16-bit NASM boot sector (BIOS fallback) |
| `kernel/src/boot.S` | Multiboot2 header (GNU as assembly) |
| `kernel/src/linker.ld` | Linker script — memory layout at 2MB, binary output format |

## Boot Sequence

### PVH Mode (experimental in this repo)

1. QEMU loads the raw kernel binary directly using the `-kernel` flag
2. QEMU detects the `.note.pvh` ELF note section and boots in PVH mode
3. Execution begins at `_start()` in `main.rs`
4. `_start()` calls `vga::print_vga("Hello World!")` which writes to VGA memory
5. The kernel enters `loop {}` and runs indefinitely

### Boot Sector Mode (recommended path in this repo)

1. BIOS loads the 512-byte boot sector from `boot.img` at address `0x7C00`
2. The boot sector (`boot.asm`) switches to VGA text mode 80x25 using BIOS interrupt `0x10`
3. It prints "Hello World!" using BIOS teletype output (INT 0x10, AH=0x0E)
4. It enters `jmp $` (infinite loop)

### Multiboot2 Header

The `boot.S` file contains a Multiboot2 header stub (magic `0xe85250d6`) in the `.multiboot_header` section for GRUB/multiboot experiments. The stable, validated path in this repository remains BIOS boot via `boot.img`.

## Core Components

### Entry Point

```rust
#[no_mangle]
pub extern "C" fn _start() -> ! {
    vga::print_vga("Hello World!");
    loop {}
}
```

- `#[no_mangle]` — preserves the symbol name so the linker/bootloader can find it
- `extern "C"` — uses the C calling convention expected by bootloaders
- `-> !` — the function never returns (kernel runs forever)

### VGA Output

The kernel writes directly to the VGA text buffer at physical address `0xb8000`:

```rust
const VGA_BUFFER: *mut u8 = 0xb8000 as *mut u8;
const VGA_COLOR_WHITE: u8 = 0x0f;
```

Each character cell is 2 bytes: one ASCII byte followed by one color attribute byte. The `print_vga` function iterates over the string bytes and writes each one with the white-on-black color attribute (0x0F).

### PVH Note

```rust
#[link_section = ".note.pvh"]
static PVH_NOTE: Elf64Note = Elf64Note { ... };
```

An ELF note structure placed in the `.note.pvh` section. QEMU scans for this note when loading a kernel with the `-kernel` flag. It tells QEMU this binary supports PVH boot. The note contains:
- Name: `"PHV\0"`
- Type: `1` (NT_PVH)
- Data: feature flags and load address (2MB)

### Panic Handler

```rust
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
```

Required by the Rust compiler in `no_std` contexts. In this minimal kernel, panics simply loop. A more complete kernel would print diagnostic information.

## Memory Layout

The linker script (`linker.ld`) places the kernel at 2MB (`0x200000`):

```
0x200000  .note.pvh           — 20 bytes, PVH ELF note
          .multiboot_header   — 24 bytes, Multiboot2 header
          .text               — kernel code + read-only data (aligned to 4KB)
          .data               — initialized globals (empty in this kernel)
          .bss                — uninitialized globals (empty in this kernel)
```

Loading at 2MB avoids the first megabyte (BIOS, bootloader, VGA memory) and provides proper alignment.

## Build Configuration

- **Toolchain**: Rust nightly (required for `-Z build-std`)
- **Target**: `x86_64-unknown-none` (bare metal, no OS)
- **Profile**: `release` with `panic = "abort"`
- **Build-std**: `core` and `compiler_builtins` recompiled from source for the bare-metal target
- **Linker flags**: `-T kernel/src/linker.ld` for custom memory layout
- **Output format**: Raw binary (`OUTPUT_FORMAT(binary)` in linker script)

## Design Philosophy

### Minimalism
- Two small source files, zero external dependencies
- Direct hardware access — no abstraction layers
- Two boot methods covering modern (PVH) and legacy (BIOS) paths

### No-std Discipline
- No heap allocator, no collections, no formatting
- All output via raw pointer writes to memory-mapped hardware
- `core::panic::PanicInfo` is the only `core` type used

### Extensibility
- The linker script reserves standard ELF sections (`.text`, `.data`, `.bss`) for future expansion
- The PVH note and multiboot header are isolated in their own sections
- Core output logic is isolated in `vga.rs`, making future drivers easier to add
