# Architecture

## Overview

This kernel is a `no_std` x86_64 Rust binary booted by QEMU through the PVH path (`-kernel`).
The assembly bootstrap (`kernel/src/boot.S`) starts in 32-bit protected mode, enables long mode, then transfers control to Rust `_start` in 64-bit mode.

## Design: Minimal Microkernel

LearnOS follows the Liedtke minimality principle — only code that MUST run in privileged mode lives in the kernel:

```
┌─────────────────────────────────────────────────┐
│              KERNEL SPACE (ring 0)              │
│  ┌──────────────────────────────────────────┐  │
│  │ Scheduling (round-robin preemption)      │  │
│  │ Memory management (4-level paging)       │  │
│  │ Inter-process communication (IPC)        │  │
│  │ System call dispatch + IRQ handling      │  │
│  └──────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
          │ syscall / IPC / IRQ
┌─────────┼──────────────┬──────────────┐
▼         ▼              ▼              ▼
┌──────┐ ┌──────┐ ┌──────────┐ ┌──────────┐
│ Init │ │ Shell│ │VGA Server│ │Kbd Server│
│PID 3 │ │PID 4 │ │  PID 1   │ │  PID 2   │
└──────┘ └──────┘ └──────────┘ └──────────┘
           USER SPACE (ring 3)
```

### What lives in kernel space
- Process scheduling (timer IRQ → round-robin)
- Memory management (4-level page tables, CR3 switching per process)
- IPC message passing (kernel-copied fixed-size messages)
- System call entry/exit (syscall/sysretq)
- Minimal hardware access that must be fast (reading PS/2 scancode from port 0x60 in IRQ handler)

### What lives in user space
- VGA display driver — writes directly to mapped video memory
- Keyboard driver — decodes scancodes, buffers key events
- Init process — initial bootstrap of user environment
- Shell — interactive input loop

## Boot Flow

### Kernel Bootstrap

1. **PVH loader** enters `pvh_start` (32-bit)
2. **Assembly bootstrap** configures stack, identity page tables, GDT, long mode
3. **Jump to Rust `_start`** (64-bit):
   - Initialize frame allocator and per-process page tables
   - Enable CR4.PGE (global pages for kernel mappings)
   - Set up TSS (kernel stack for ring transitions)
   - Configure syscall MSRs (STAR, LSTAR, SFMASK)
   - Set up IDT (timer at 0x20, keyboard at 0x21, syscall at 0x80)
   - Spawn user-space processes (VGA server → keyboard server → init → shell)
   - Remap PIC (IRQ0→0x20, IRQ1→0x21)
   - Program PIT timer (100Hz for scheduling)
   - Unmask timer + keyboard IRQs in PIC
   - Switch to user space via iretq

### User-Space Boot Sequence

Processes are spawned in order. Each gets its own 4-level page table with the kernel identity-mapped (not accessible from ring 3) and its code page at 0x400000.

1. **VGA server (PID 1)**: VGA text buffer (0xB8000) mapped to 0x600000 with `CACHE_DISABLE`. Loops on IPC_RECV waiting for print/clear commands.
2. **Keyboard server (PID 2)**: Receives raw scancodes from kernel IRQ handler via IPC, decodes to ASCII, buffers in ring buffer. Serves keys to clients on request.
3. **Init (PID 3)**: Prints banner, then halts.
4. **Shell (PID 4)**: Requests keys from keyboard server, processes line input, echoes to VGA server.

## IPC Protocol

Fixed-size 64-byte messages (`IpcMessage`):

```
┌─────────┬──────────┬──────────────────────────────┐
│ src_pid │ msg_type │         data[60]             │
│  u16    │   u16    │         bytes                │
└─────────┴──────────┴──────────────────────────────┘
```

### Registered Message Types

| Type | Name | Direction | Payload |
|------|------|-----------|---------|
| 1 | MSG_VGA_PRINT | Kernel → VGA Server | `data[0]` = ASCII byte |
| 2 | MSG_VGA_CLEAR | Kernel → VGA Server | (none) |
| 3 | MSG_KEY_SCANCODE | IRQ Handler → Kbd Server | `data[0]` = raw scancode |
| 4 | MSG_KEY_REQUEST | Client → Kbd Server | (none) |
| 5 | MSG_KEY_EVENT | Kbd Server → Client | `data[0]` = char, `data[1]` = 1 if available |

### Kernel→Server In-Kernel IPC

The kernel IRQ handler reads port 0x60 and forwards scancodes directly to the keyboard server's kernel-side IPC queue. No syscall needed — the handler writes to the queue directly (`ipc_push`). This is necessary because port I/O is privileged (ring 0 only) and the PS/2 controller only buffers one byte.

## Memory Layout

Defined in `kernel/src/linker.ld`:
- `.note.pvh` at low address: PVH metadata
- `.text`, `.rodata`, `.data`, `.bss` from 0x100000

### Virtual Address Space (per-process via CR3)

```
PML4[0] → PDPT[0] → PD (512 entries)
  PD[0]: 0x00000000-0x001FFFFF  Kernel code (2MB huge, U=0)
  PD[1]: 0x00200000-0x003FFFFF  Kernel data (2MB huge, U=0)
  PD[2]: 0x00400000-0x005FFFFF  → 4KB PT for user code (U=1)
  PD[3]: Available for device mappings (e.g. VGA at 0x600000)
  PD[4-6]: Not present
  PD[7]: 0x00E00000-0x00FFFFFF  User stack (2MB huge, U=1)
```

### Physical Memory
- 0x000000 - 0x3FFFFF: Identity-mapped kernel (4MB)
- 0x0B8000: VGA text buffer (80×25 colour, 4KB)
- Beyond 4MB: Frame allocator for page tables and user code pages

## Source Files

| File | Responsibility |
|------|----------------|
| `kernel/src/boot.S` | PVH entry note + 32→64-bit bootstrap |
| `kernel/src/main.rs` | Kernel entry point, boot sequencing, PIC remap, timer setup |
| `kernel/src/paging.rs` | 4-level page tables, frame allocator, `map_phys_page()` |
| `kernel/src/process.rs` | PCB, round-robin scheduler, IPC message/queue, context switch |
| `kernel/src/syscall.rs` | Syscall dispatch, IDT setup, MSR config, timer/keyboard IRQ handlers |
| `kernel/src/tss.rs` | Task State Segment for ring-0 stack on interrupts |
| `kernel/src/vga.rs` | Fallback VGA writes (early boot, panic), serial debug output |
| `kernel/src/user_program.rs` | Built-in user-space processes (VGA server, keyboard server, init, shell) |
| `kernel/src/linker.ld` | ELF layout, section order, load addresses |

## I/O Model

### VGA (Memory-Mapped I/O)
- Base address: `0xB8000`, mapped to `0x600000` for user-space VGA server
- Cell format: `[ascii][attribute]`, 80×25, 2 bytes per cell
- CACHE_DISABLE flag set in PTE (device memory, not RAM)
- Early boot: kernel writes directly via physical address
- After VGA server spawns: all writes forwarded via IPC to VGA server

### Keyboard (Port-Mapped I/O)
- IRQ1 triggers keyboard_entry assembly → keyboard_irq_handler()
- Handler reads port 0x60 (must be fast — PS/2 only buffers 1 byte)
- Raw scancode forwarded to keyboard server via in-kernel IPC
- Keyboard server decodes PS/2 set 1 make codes → ASCII
- Echo to VGA via syscall → IPC forward to VGA server
- Client processes request keys via MSG_KEY_REQUEST (type 4)
- Keyboard server replies with MSG_KEY_EVENT (type 5)

### Interrupts
- Timer (IRQ0, vector 0x20): 100Hz via PIT channel 0
- Keyboard (IRQ1, vector 0x21): Edge-triggered from PS/2 controller
- PIC: 8259A remapped to 0x20-0x2F (master) and 0x28-0x2F (slave)
- Only IRQ0 and IRQ1 unmasked; all others masked

## Context Switching

The timer interrupt fires ~100 times/sec. The CPU automatically switches to kernel stack (via TSS.RSP0), pushes interrupt frame, then saves all GP registers. `timer_save_and_switch`:

1. Saves current process registers from stack into PCB
2. Sends EOI to PIC
3. Calls `schedule_next()` for round-robin selection
4. Loads next process's registers from PCB into stack frame
5. Writes CR3 with next process's page table pointer
6. Updates TSS.RSP0 for the next process's kernel stack

The `iretq` then pops the new process's RIP/CS/RFLAGS/RSP/SS and execution continues there.

## Design Choices

- **No heap allocator**: Fixed-size process table (8 slots), fixed-size IPC queues (16 messages)
- **Interrupt-driven preemption**: Timer at 100Hz, not polling
- **Round-robin scheduling**: Simplest possible preemptive scheduler
- **Cooperative yield**: Processes can call SCHEDULE to voluntarily yield CPU
- **No filesystem**: All user programs compiled into kernel binary as `extern "C"` functions
- **Minimal kernel**: Only scheduling, memory, IPC, and IRQ dispatch in ring 0
