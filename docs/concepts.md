# OS Kernel Concepts

This document explains fundamental operating system kernel concepts through the lens of this minimal x86_64 Rust kernel. Each concept is illustrated with real code from the project.

## What is a Kernel

A kernel is the first code that runs at boot and the only code that runs with full hardware access. It's responsible for managing the CPU, memory, and devices — providing the foundation everything else builds on.

This kernel is the simplest possible example: it boots, writes to the screen, and loops forever. There's no process scheduler, no memory management, no filesystem. But every concept it demonstrates — bare-metal entry, no-std compilation, direct hardware I/O — is the same foundation a full OS kernel builds on.

## Bare-Metal Programming

### The no_std Environment

```rust
#![no_std]  // Don't link the Rust standard library
#![no_main] // Don't use the standard entry point
```

In a normal Rust program, the standard library provides heap allocation, threading, I/O, and a runtime that calls `fn main()`. None of that exists on bare metal — there's no OS underneath to provide it.

`#![no_std]` strips all of that away, leaving only the `core` library (primitives like `u8`, slices, `Option`, `Result`). `#![no_main]` tells the compiler not to generate the standard entry point — instead, we provide our own `_start` function that the bootloader calls directly.

This is fundamental to kernel development because:

- **Full control**: We decide exactly what runs and when
- **No hidden behavior**: No runtime threads, no hidden allocations, no background services
- **Zero overhead**: Only the code we write gets compiled

### The Entry Point

```rust
#[no_mangle]
pub extern "C" fn _start() -> ! {
    print_vga("Hello World!");
    loop {}
}
```

- `extern "C"` — the bootloader calls this function using the C calling convention (System V AMD64 ABI on x86_64). Rust's default calling convention is not stable, so we must use the C ABI.
- `#[no_mangle]` — prevents the compiler from renaming the symbol. The linker script names `_start` as the entry point, so the name must match exactly.
- `-> !` — this function never returns. A kernel has nowhere to return *to* — there's no OS to hand control back to. The infinite loop (`loop {}`) keeps the CPU executing (or halted, depending on implementation) indefinitely.

### The Panic Handler

```rust
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
```

Rust requires a panic handler even in `no_std` contexts. The `core::panic::PanicInfo` parameter contains the panic message, file, and line number — but in this minimal kernel, we don't have a way to display it, so we just halt. A more complete kernel would print the panic info to the screen or a serial port.

## Memory Architecture

### Physical Memory Map

On x86_64, the first megabyte of physical memory is reserved:

```
0x00000000 – 0x0009FFFF  Conventional memory (640 KB)
0x000A0000 – 0x000BFFFF  VGA text buffer and graphics memory
0x000C0000 – 0x000FFFFF  BIOS ROM and extensions
0x00100000 – 0x001FFFFF  First megabyte above the BIOS region
0x00200000+              Where this kernel loads (2 MB)
```

The kernel loads at 2MB to avoid all of these reserved regions.

### VGA Text Mode

The VGA text buffer is a memory-mapped hardware device at physical address `0xb8000`. Writing bytes to this address causes characters to appear on screen — no driver needed.

```rust
const VGA_BUFFER: *mut u8 = 0xb8000 as *mut u8;
```

The buffer is organized as 80 columns × 25 rows. Each character cell is 2 bytes:

```
Offset 0: ASCII character code
Offset 1: Color attribute byte
```

For example, to display a white `H` in the top-left corner:
- Write `0x48` (ASCII 'H') at `0xb8000`
- Write `0x0F` (white on black) at `0xb8001`

The color attribute byte format:

```
Bit  7  : Blink enable
Bits 4-6: Background color (RGB)
Bit  3  : Foreground brightness (intensity)
Bits 0-2: Foreground color (RGB)
```

Common colors: `0x0F` = bright white on black, `0x0C` = bright red on black, `0x0A` = bright green on black.

### The print_vga Function

```rust
fn print_vga(s: &str) {
    let mut i = 0;
    for &byte in s.as_bytes() {
        unsafe {
            *VGA_BUFFER.offset(i as isize * 2) = byte;
            *VGA_BUFFER.offset(i as isize * 2 + 1) = VGA_COLOR_WHITE;
        }
        i += 1;
    }
}
```

This function writes each byte of the string to the VGA buffer, interleaved with the color attribute. The `unsafe` block is required because we're dereferencing a raw pointer — the compiler can't prove the address is valid or that no other code is modifying the same memory.

Limitations of this implementation:
- No scrolling — writes past column 80 continue off-screen
- No newline handling — `\n` would print as a garbage character
- No bounds checking — a long string would write past the 4000-byte buffer
- No synchronization — only safe because this kernel is single-threaded

## Boot Methods

### PVH (Para-Virtualized Hypervisor)

PVH is a modern QEMU feature that loads a kernel directly without going through BIOS emulation. The kernel advertises PVH support via an ELF note:

```rust
#[link_section = ".note.pvh"]
static PVH_NOTE: Elf64Note = Elf64Note {
    namesz: 4,
    descsz: 16,
    type_: 1,       // NT_PVH
    name: [b'P', b'H', b'V', 0],
    data: [
        0x00, 0x00, 0x00, 0x01, // feature flags (bit 0 = 64-bit)
        0x00, 0x00, 0x00, 0x00,
        0x20, 0x00, 0x00, 0x00, // load address (0x200000 = 2MB)
        0x00, 0x00, 0x00, 0x00,
    ],
};
```

The `#[link_section = ".note.pvh"]` attribute tells the linker to place this struct in a section called `.note.pvh`. QEMU scans for this section when you use the `-kernel` flag. If found, it loads the kernel at the specified address and jumps to `_start`.

Run with: `qemu-system-x86_64 -kernel target/pvh-kernel -m 128M -nographic`

### Boot Sector (BIOS)

The boot sector (`kernel/src/boot.asm`) is a 512-byte NASM assembly program that provides a BIOS-compatible fallback. It uses BIOS interrupt `0x10` for video output — a completely independent code path from the Rust kernel's VGA writing.

```asm
mov ax, 0x0003    ; Set video mode to 80x25 color text
int 0x10          ; Call BIOS video service

mov si, message   ; Point to string
mov ah, 0x0E      ; BIOS teletype output function
print_loop:
    lodsb          ; Load byte from [SI], increment SI
    int 0x10       ; Print character in AL
    loop print_loop
```

The boot sector is written to the first sector of a 1.44MB floppy image. BIOS loads it at `0x7C00` and executes it. The final two bytes `0xAA55` are the boot signature that tells BIOS this sector is bootable.

Run with: `qemu-system-x86_64 -drive format=raw,file=target/boot.img -boot order=a -m 32 -nographic`

### Multiboot2 Header (Stub)

The `boot.S` file defines a Multiboot2 header stub for compatibility experiments with bootloaders like GRUB:

```asm
.long 0xe85250d6   # magic
.long 0x00000000   # architecture (i386)
.long 0x00000018   # header_length (24 bytes)
.long 0x17daf2b2   # checksum (-(magic + arch + length))
.long 0x00000000   # end tag type
.long 0x00000008   # end tag size
```

The checksum must satisfy: `magic + architecture + header_length + checksum == 0` (as a 32-bit unsigned sum). This is how a multiboot-compatible bootloader verifies the header is valid and not a random byte sequence.

## Linker Script

The linker script (`kernel/src/linker.ld`) controls where each section is placed in memory:

```ld
ENTRY(_start)
OUTPUT_FORMAT(binary)

SECTIONS {
    . = 2M;

    .note.pvh : ALIGN(4) { KEEP(*(.note.pvh)) }
    .multiboot_header : ALIGN(4) { KEEP(*(.multiboot_header)) }
    .text : ALIGN(4K) { *(.text .text.*) *(.rodata .rodata.*) }
    .data : ALIGN(4K) { *(.data .data.*) }
    .bss  : ALIGN(4K) { *(.bss .bss.*) *(COMMON) }
}
```

Key points:
- `OUTPUT_FORMAT(binary)` — strips ELF headers, producing a raw binary that QEMU can load directly
- `. = 2M` — starts placing sections at the 2MB mark
- `KEEP()` — prevents the linker from discarding the PVH and multiboot sections even though no code references them
- `ALIGN(4K)` — 4KB alignment for code and data sections (matches page size)

## Build System

The kernel uses Cargo's `-Z build-std` feature to recompile `core` and `compiler_builtins` for the bare-metal target:

```bash
RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld" \
    cargo +nightly build -Z build-std=core,compiler_builtins \
    --target x86_64-unknown-none \
    --release --bin kernel
```

- `RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld"` — passes the linker script to the linker
- `-Z build-std=core,compiler_builtins` — recompiles the core library from source for the bare-metal target (the precompiled version targets a hosted environment with OS support)
- `--target x86_64-unknown-none` — the bare-metal x86_64 target (no OS, no standard library)
- `--release` — optimized build
- `--bin kernel` — builds the binary named "kernel" defined in `kernel/Cargo.toml`

## Extending This Kernel

The modular structure of the linker script and the isolated boot stubs make extension straightforward:

- **Interrupt handling**: Set up IDT (Interrupt Descriptor Table) in a new `interrupts` module
- **Memory management**: Add a page allocator and virtual memory mapping in `.bss`-backed structures
- **Serial output**: Write to `0x3F8` (COM1) for debug logging that works in `-nographic` QEMU
- **Keyboard input**: Read from the keyboard controller (port `0x60`) via interrupt handler
- **Heap allocation**: Implement a global allocator to enable `alloc` types (`String`, `Vec`, `Box`)
