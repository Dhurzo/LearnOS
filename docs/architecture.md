# Architecture

## Overview

LearnOS is a `no_std` x86_64 Rust binary booted by QEMU through the PVH path
(`-kernel`). The assembly bootstrap (`kernel/src/boot.S`) starts in 32-bit
protected mode, enables long mode, then transfers control to Rust `_start` in
64-bit mode. Everything after the first ~50 lines of assembly is 100% Rust.

## Design: Minimal Microkernel

LearnOS follows the Liedtke minimality principle — only code that MUST run in
privileged mode lives in the kernel:

```
┌─────────────────────────────────────────────────┐
│              KERNEL SPACE (ring 0)              │
│  ┌──────────────────────────────────────────┐  │
│  │ Scheduling (round-robin preemption)      │  │
│  │ Memory management (4-level paging)       │  │
│  │ Inter-process communication (IPC)        │  │
│  │ System call dispatch + IRQ handling      │  │
│  │ Capability enforcement + signals         │  │
│  └──────────────────────────────────────────┘  │
└──────────────────────┬──────────────────────────┘
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
- IPC message passing (kernel-copied fixed-size 64-byte messages)
- System call entry/exit (`syscall`/`sysretq`)
- Minimal hardware access that must be fast (reading PS/2 scancode from port 0x60)
- Capability-bitmap check on every IPC send
- Signal delivery on context switch
- In-memory ELF loading and embedded filesystem

### What lives in user space
- VGA display driver — writes directly to mapped video memory (0x600000)
- Keyboard driver — decodes scancodes, buffers key events in ring buffer
- Init process — prints banner
- Shell — interactive input loop requesting keys from keyboard server

## Boot Flow

### Kernel Bootstrap

1. **PVH loader** enters `pvh_start` (32-bit)
2. **Assembly bootstrap** configures stack, identity page tables, GDT, long mode
3. **Jump to Rust `_start`** (64-bit):

   ```
   a. Init serial port + VGA
   b. Init bitmap frame allocator (32512 frames, 0x100000-0x8000000)
   c. Init RAM disk (128 KiB embedded filesystem backing store)
   d. Enable CR4.PGE (global pages survive CR3 writes)
   e. Set up TSS (kernel stack for ring-3→ring-0 transitions)
   f. Configure syscall MSRs (STAR, LSTAR, SFMASK)
   g. Build IDT:
      - Vector 0x20: timer IRQ0 (PIT, 100 Hz)
      - Vector 0x21: keyboard IRQ1 (PS/2)
      - Vector 0x0D: #GP (diagnostic)
      - Vector 0x08: Double fault (IST1 stack)
   h. Spawn user processes (VGA → keyboard → init → shell)
   i. Remap PIC: IRQ0→0x20, IRQ1→0x21
   j. Program PIT timer (divisor 11931 = ~100 Hz)
   k. Unmask IRQ0 + IRQ1 in PIC
   l. Switch to user space via `iretq`
   ```

Each step writes a progress character to VGA row 0 — see README for the legend.

### User-Space Boot Sequence

Processes are spawned in order. Each gets its own 4-level page table with
the kernel identity-mapped (not accessible from ring 3) and its code page
at `0x400000`.

1. **VGA server (PID 1)**: VGA text buffer (0xB8000) mapped to 0x600000 with
   `CACHE_DISABLE`. Loops on `IPC_RECV` waiting for print/clear commands.

2. **Keyboard server (PID 2)**: Receives raw scancodes from the interrupt
   server via IPC (or directly from the kernel IRQ handler as fallback).
   Decodes PS/2 set-1 make codes to ASCII, buffers in a 16-entry ring buffer.

3. **Init (PID 3)**: Prints "Init\n" via VGA_WRITE syscall, then halts.

4. **Shell (PID 4)**: Requests keys from keyboard server via IPC, echoes typed
   characters to VGA server, processes line input.

> **Microkernel property**: Each server runs in its own address space at ring 3.
> If the VGA server crashes, the keyboard server and shell keep running. The
> kernel is unaffected.

---

## Phase 2: Process Creation (Fork + Exec)

### Why

Before Phase 2, all processes were created at boot via hardcoded `spawn()` calls
in `main.rs`. The shell couldn't create subprocesses. No dynamic process
lifecycle existed.

### Fork (`SYS_FORK`, syscall 13)

`fork_process()` in `process.rs`:

1. Extracts parent state (entry point, page table pointer, registers)
2. Allocates a new PID (monotonically increasing `next_pid`)
3. Finds a free slot in `PROCESS_TABLE`
4. Allocates a fresh kernel stack frame for the child
5. Calls `copy_page_table_cow(parent_cr3)` — see below
6. Creates a new `Process` with copied registers but `rax = 0` (child return)
7. Returns child PID to parent

### Copy-on-Write

`copy_page_table_cow()` in `paging.rs`:

1. Allocates a new PML4 frame
2. Walks the parent's page table tree
3. For every user page (U=1): copies the PTE, clears the W (writable) bit,
   sets the COW flag (PTE bit 9 — an OS-reserved bit in x86-64)
4. For kernel pages (U=0): copies PTE unchanged (shared, global)
5. Returns the new PML4 physical address

When either process writes to a COW page, the CPU raises a page fault (#PF).
The kernel's `handle_cow_fault()` in the page-fault handler:

1. Checks that the faulting address has the COW flag set
2. Allocates a new physical frame
3. Copies the 4 KiB page data
4. Updates the PTE: writable, no COW flag
5. Flushes TLB for that page

**Visual**:
```
Before fork:
  [Physical page A] ←─ Parent PTE (writable)

After fork:
  [Physical page A] ←─ Parent PTE (read-only, COW)
                   ←─ Child PTE  (read-only, COW)

After child writes:
  [Physical page A] ←─ Parent PTE (unchanged)
  [Physical page B] ←─ Child PTE  (writable, fresh copy)
```

### Exec (`SYS_EXEC`, syscall 14; `SYS_EXEC_ELF`, syscall 19)

`exec_builtin()` replaces the process image with a new built-in program:

1. Unmaps old user code pages
2. Allocates a fresh code frame
3. Copies the entry-point code into the new frame
4. Maps code at `USER_VADDR_LOAD` (0x400000)
5. Resets registers to default state
6. Sets `rip = entry`, `rsp = USER_STACK_VADDR`

`sys_exec_elf()` does the same but via `elf::load_elf()`, which parses ELF
program headers and maps LOAD segments with correct permissions.

---

## Phase 3: ELF Loader + Filesystem

### ELF Loader (`elf.rs`)

The ELF loader is a minimal in-kernel ELF-64 parser. It is called from
`sys_exec_elf` (syscall 19) with a pointer to in-memory ELF data.

**Steps**:

1. Read and validate the ELF header:
   - Check magic `\x7fELF`
   - Confirm 64-bit (class = 2) and executable type (e_type = 2)
2. Walk the program header table:
   - For each `PT_LOAD` segment:
     - Page-align the virtual address (round down)
     - For each 4K page in the segment:
       - Allocate a physical frame via the bitmap allocator
       - Zero-fill the frame (for `.bss` segments where `memsz > filesz`)
       - Copy segment data from the ELF image
       - Map the frame into the process's page table with correct flags
         (writable if `PF_W` set, user-accessible)
3. Return the entry point (`e_entry`) and suggested stack top

**Why in-kernel?** A true microkernel would load ELFs in user space. We keep
the loader in the kernel for simplicity — the syscall interface to pass file
data is straightforward.

### Embedded Filesystem (`filesystem.rs`)

A flat directory table with built-in files compiled into the kernel binary.
This is the "initramfs" pattern.

**Directory table**: `DirEntry[16]` — each entry has a 32-byte name and an
optional `&'static [u8]` data slice. Populated at compile time via `const fn`.

**Built-in files**:

| Name | Content |
|------|---------|
| `hello.txt` | "Hello from the LearnOS filesystem!" |
| `README` | Help text with available shell commands |
| `boot.cfg` | Sample config file |

**Per-process open-file table**: `OpenFile[8]` per process — tracks file_id
and cursor position. Max 8 simultaneous open files per process.

**Syscalls**:

| # | Name | What it does |
|---|------|-------------|
| 20 | OPEN | Look up filename in directory table, allocate FD slot |
| 21 | FS_READ | Copy data from file's static slice into user buffer at cursor |
| 22 | CLOSE | Free FD slot |
| 23 | READDIR | List filenames into a user buffer |

### Block Device (`block_dev.rs`)

A 128 KiB RAM disk backing store (`[u8; 256 * 512]`) initialized at boot.
Provides `read_block()` and `write_block()` with 512-byte sectors, plus
`read_bytes()` for cross-boundary reads.

In a mature system, this would be replaced by a user-space virtio-blk driver
communicating via IPC. The RAM disk lets us demonstrate the block-device
abstraction without real hardware.

---

## Phase 4: System Hardening

### Capability-Based IPC (`capability.rs`)

Before Phase 4, any process could send IPC to any other. This is a security
hole — a malicious process could spam the VGA server or impersonate the
keyboard server.

**Capability model**:
- Each process holds a `u64` bitmap. Bit N = process holds capability N.
- Well-known capability IDs are defined in `capability::cap_id`:

  | Bit | Name | Required to send to |
  |-----|------|---------------------|
  | 0 | VGA_SEND | VGA server (PID 1) |
  | 1 | KEYBOARD_SEND | Keyboard server (PID 2) |
  | 2 | INTERRUPT_SEND | Interrupt server |
  | 3 | DRIVER_SEND | Any other driver |

- `sys_ipc_send()` calls `capability::check_ipc_send(sender_caps, dst_pid)`
  before allowing any message through. No capability → return `-1`.
- PID 0 (kernel) is always allowed.
- Capabilities are granted at process creation by the kernel and cannot be
  forged by user code (they live in the PCB, not in user-accessible memory).

**Why a bitmap?** It's O(1), fits in a register, and is simple to reason about.
A real capability system would use cryptographic tokens — the bitmap is an
educational simplification.

### Signal Delivery (`signal.rs`)

Before Phase 4, exceptions (#GP, page faults) halted the entire system.

**Signal model**:
- Each process has a `u64` `signal_pending` bitmap and a
  `[Option<u64>; 64]` `signal_handlers` table.
- Supported signals:

  | # | Name | Default action | Purpose |
  |---|------|---------------|---------|
  | 0 | SIGKILL | Kill | Immediate termination |
  | 1 | SIGSEGV | Kill | Invalid memory access |
  | 2 | SIGILL | Kill | Illegal instruction |
  | 3 | SIGALRM | Ignore | Timer notification |
  | 4 | SIGIPC | Ignore | IPC message delivered |
  | 5 | SIGTERM | Kill | Graceful termination |
  | 6 | SIGCHLD | Ignore | Child exited |

- `send_signal(pid, sig)`: Sets the bit in the target's pending bitmap.
  Wakes the process if it was blocked.
- `deliver_pending(pid)`: Called on every context switch in
  `timer_save_and_switch()`. Finds the lowest pending bit, clears it, and
  dispatches:
  - **Kill**: Sets process state to `Terminated`
  - **Ignore**: Clears the bit, continues
  - **Handler**: If a handler address is registered, pushes the current RIP
    onto the user stack and sets RIP to the handler. The handler runs at
    ring 3 and can call `SYS_WAIT_SIGNAL` to clear the signal.

**Signal delivery timing**:
```
Timer IRQ
  → timer_entry (assembly saves registers)
  → timer_save_and_switch (Rust)
      → Save current process registers
      → schedule_next()
      → Load next process registers
      → deliver_pending(next)    ← signals delivered HERE
      → Write CR3, update TSS
  → iretq (assembly restores new process)
```

---

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
| 1 | MSG_VGA_PRINT | Any → VGA Server | `data[0]` = ASCII byte |
| 2 | MSG_VGA_CLEAR | Any → VGA Server | (none) |
| 3 | MSG_KEY_SCANCODE | IRQ Handler → Kbd Server | `data[0]` = raw scancode |
| 4 | MSG_KEY_REQUEST | Client → Kbd Server | (none) |
| 5 | MSG_KEY_EVENT | Kbd Server → Client | `data[0]` = char, `data[1]` = 1 if avail |
| 6 | MSG_HW_IRQ | Kernel → Interrupt Server | `data[0]` = IRQ number, `data[1..]` = payload |

### Kernel→Server In-Kernel IPC

When IRQ1 fires, the keyboard handler reads port 0x60 (privileged — must be
ring 0) and calls `interrupt_server::forward_irq()`. This function:

1. Checks if a user-space interrupt server is registered (`INTERRUPT_SERVER_PID`)
2. **If yes**: Sends a `MSG_HW_IRQ` message to the interrupt server, which
   then forwards to the appropriate registered handler
3. **If no** (backward-compatible fallback): Sends directly to all registered
   handlers for that IRQ number. For IRQ 1, the message type is
   `MSG_KEY_SCANCODE` so the keyboard server receives the same format as before.

This two-path design means the system works even without a user-space interrupt
server, while being ready for the full microkernel IRQ path.

### Capability Enforcement

Every `IPC_SEND` syscall now includes a capability check:

```
sys_ipc_send(dst_pid, msg_ptr):
  if !capability::check_ipc_send(current_process.capabilities, dst_pid):
      return -1   // rejected — no capability
  ... proceed with copy and delivery ...
```

---

## Memory Layout

### Physical Memory Map

| Address | Size | Content |
|---------|------|---------|
| `0x000000` | 1 MiB | Reserved: real-mode IVT, BDA, EBDA |
| `0x100000` | 3 MiB | Kernel image (.text, .rodata, .data, .bss) |
| `0x0B8000` | 4 KiB | VGA text buffer (80×25 colour) |
| `0x400000` | ~128 MiB | Free — managed by bitmap frame allocator |

### Frame Allocator

- **Range**: `PHYS_MEM_START = 0x100000` to `PHYS_MEM_END = 0x8000000`
- **Frame size**: 4 KiB
- **Total frames**: `(0x8000000 - 0x100000) / 4096 = 32512`
- **Bitmap**: `508 × u64` words (`32512 / 64 = 508`)
- **API**: `alloc_frame()` → physical address (or 0 if full),
  `free_frame(phys_addr)` → clears bitmap bit

### Virtual Address Space (per-process)

```
PML4[0] → PDPT[0] → PD (512 × 2 MiB entries)

PD[0]: 0x00000000-0x001FFFFF  Kernel code   (U=0, G=1, 2M huge)
PD[1]: 0x00200000-0x003FFFFF  Kernel data   (U=0, G=1, 2M huge)
PD[2]: 0x00400000-0x005FFFFF  → 4K page table for user code (U=1)
                                 Page 0: 0x400000-0x400FFF  User code
PD[3]: 0x00600000-0x007FFFFF  → 4K page table for VGA map (U=1)
                                 Page 0: 0x600000  VGA buffer
PD[7]: 0x00E00000-0x00FFFFFF  User stack    (U=1, 2M huge)
```

### Page Table Entry Flags

| Bit | Name | Meaning |
|-----|------|---------|
| 0 | P | Present |
| 1 | R/W | Writable |
| 2 | U/S | User-accessible |
| 3 | PWT | Write-through |
| 4 | PCD | Cache disable |
| 5 | A | Accessed |
| 6 | D | Dirty |
| 8 | G | Global (survives CR3 write) |
| **9** | **COW** | **OS-reserved — Copy-on-Write flag** |
| 63 | NX | No-execute (if EFER.NXE set) |

Bit 9 is defined by Intel as "available for OS use". We use it to mark
COW pages after fork.

---

## Context Switching

The timer interrupt fires at 100 Hz. The CPU automatically switches to kernel
stack (via TSS.RSP0), pushes an interrupt frame (SS, RSP, RFLAGS, CS, RIP),
then jumps to `timer_entry` assembly which saves all 15 GP registers.

`timer_save_and_switch` (Rust) does:

```
1. Save current process's GP registers from stack → PCB
2. Send EOI to PIC
3. schedule_next() — round-robin: advance to next Ready process
4. Load next process's GP registers from PCB → stack frame
5. deliver_pending(next) — deliver any pending signals
6. Write CR3 = next process's PML4 physical address
7. Update TSS.RSP0 = next process's kernel stack top
8. Update CURRENT_KERNEL_RSP (syscall MSR save area)
```

The `iretq` instruction then pops RIP, CS, RFLAGS, RSP, SS from the stack
and execution continues in the next process.

---

## I/O Model

### VGA (Memory-Mapped I/O)
- Physical base: `0xB8000`, mapped to `0x600000` for VGA server
- Cell format: `[ascii][attribute]`, 80×25, 2 bytes per cell = 4000 bytes
- `CACHE_DISABLE` flag in PTE (device memory, not RAM)
- Early boot: kernel writes directly via physical address
- After VGA server spawns: all writes forwarded via IPC

### Keyboard (Port-Mapped I/O)
- IRQ1 triggers keyboard_entry → `keyboard_irq_handler()`
- Handler reads port `0x60` (must be fast — PS/2 only buffers 1 byte)
- Forwards scancode via `interrupt_server::forward_irq(1, data)`
- Keyboard server decodes PS/2 set 1 make codes → ASCII
- Client processes request keys via `MSG_KEY_REQUEST` (type 4)

### Interrupt Controller
- 8259A PIC remapped: master base 0x20, slave 0x28
- Only IRQ0 (timer) and IRQ1 (keyboard) unmasked
- EOI written to master PIC port 0x20 after each IRQ

---

## Source Files

| File | Responsibility |
|------|----------------|
| `kernel/src/boot.S` | PVH entry note + 32→64-bit bootstrap |
| `kernel/src/main.rs` | Kernel entry point, boot sequencing, PIC remap, timer setup |
| `kernel/src/paging.rs` | 4-level page tables, bitmap frame allocator, COW, `map_phys_page()` |
| `kernel/src/process.rs` | PCB, round-robin scheduler, IPC message/queue, fork/exec |
| `kernel/src/syscall.rs` | Syscall dispatch (24 handlers), IDT setup, MSR config, IRQ handlers |
| `kernel/src/tss.rs` | Task State Segment for ring-0 stack on interrupts |
| `kernel/src/interrupt_server.rs` | IRQ handler table, `forward_irq()`, registration helpers |
| `kernel/src/vga.rs` | Fallback VGA writes (early boot), serial debug output |
| `kernel/src/elf.rs` | ELF-64 parser, program-header walker, segment mapper |
| `kernel/src/filesystem.rs` | Embedded flat filesystem, directory table, open/read/close |
| `kernel/src/block_dev.rs` | 128 KiB RAM disk with 512-byte block read/write |
| `kernel/src/capability.rs` | Capability bitmap, `check_ipc_send()`, `grant()`/`revoke()` |
| `kernel/src/signal.rs` | Signal pending bitmap, `send_signal()`, `deliver_pending()` |
| `kernel/src/user_program.rs` | Built-in user-space processes (VGA, keyboard, init, shell) |
| `kernel/src/linker.ld` | ELF layout, section order, PVH note placement |

## Design Decisions

- **Bitmap frame allocator** over bump: enables `free_frame()` for process exit
- **COW bit 9**: uses an OS-reserved PTE bit — no special CPU feature required
- **Fixed-size process table** (8 slots): no heap allocator, simple for learning
- **Fixed-size IPC queues** (16 messages): lock-free SPSC ring buffer
- **In-kernel scancode read**: PS/2 hardware constraint (1-byte buffer)
- **Two-path interrupt forwarding**: backward compatible when no ISR exists
- **Embedded filesystem**: initramfs pattern — built-in files survive until
  a real block driver replaces the RAM disk
- **64-bit capability bitmap**: O(1) check, fits in register, simple to audit
- **Static PID pool**: `next_pid` counter wraps at `u16::MAX`; slots are
  reclaimed by marking the PCB as `None`

## Comparison: Before and After

| Aspect | Before (Phase 0-1) | After (Phase 2-4) |
|--------|-------------------|-------------------|
| Process creation | Hardcoded at boot | Fork + exec syscalls |
| Memory sharing | Private per-process | COW after fork |
| Program loading | Compiled into kernel | ELF binary from memory |
| File abstraction | None | open/read/close + RAM disk |
| IPC access | Any→any | Capability-gated |
| Exceptions | System halt | Signal delivery to process |
| IRQ routing | Direct to keyboard server | Through interrupt server |
| Physical memory | Bump allocator (no free) | Bitmap allocator with free |
