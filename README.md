# LearnOS - Educational Microkernel

A minimal x86_64 microkernel written in Rust, built in 4 incremental phases.
Each phase adds one capability without breaking what came before.

## What is a Microkernel?

A microkernel runs ONLY essential OS code in privileged mode (kernel space).
Everything else — drivers, filesystems, services — runs in unprivileged user space.

```
┌─────────────────────────────────────────────────┐
│                 KERNEL SPACE (ring 0)           │
│  ┌──────────────────────────────────────────┐  │
│  │ Scheduling    │ Memory Mgmt │ IPC │ IRQs │  │
│  └──────────────────────────────────────────┘  │
└──────────────────────┬──────────────────────────┘
                       │ syscalls / IPC
           ┌───────────┼──────────────┬───────────┐
           ▼           ▼              ▼           ▼
     ┌─────────┐ ┌─────────┐ ┌────────────┐ ┌─────────┐
     │  Init   │ │  Shell  │ │ VGA Server │ │Kbd Svr  │
     │  PID 3  │ │  PID 4  │ │   PID 1    │ │ PID 2   │
     └─────────┘ └─────────┘ └────────────┘ └─────────┘
                     USER SPACE (ring 3)
```

If a user-space service crashes, the kernel and other services keep running.

## Architecture

### Boot Flow

1. **PVH loader** (QEMU `-kernel`) enters 32-bit protected mode at `pvh_start`
2. **boot.S** (assembly): page tables, GDT, long-mode enable, jump to `_start`
3. **`_start`** in `main.rs` (64-bit Rust):

   ```
   ┌─ Init serial + VGA ──────────────────────────┐
   │  "1" at screen top — entered Rust             │
   ├─ Initialize frame allocator (bitmap) ────────┤
   │  32512 frames from 0x100000 to 0x8000000      │
   ├─ Initialize RAM disk (128 KiB) ──────────────┤
   ├─ Enable CR4.PGE ────────────────────────────┤
   │  (kernel page-table entries survive CR3 swap) │
   ├─ Set up TSS + syscall MSRs ─────────────────┤
   ├─ Set up IDT ────────────────────────────────┤
   │  "4" — IDT configured                        │
   ├─ Spawn user processes ──────────────────────┤
   │  "V" VGA server (PID 1)                      │
   │  "K" Keyboard server (PID 2)                 │
   │  "I" Init (PID 3)                            │
   │  "S" Shell (PID 4)                           │
   │  "5" — All processes created                 │
   ├─ Remap PIC + program PIT timer (100 Hz) ────┤
   │  "T" — Timer configured                      │
   ├─ Unmask IRQ0 (timer) + IRQ1 (keyboard) ─────┤
   └─ "U" — Switch to user space via iretq ──────┘
   ```

### Source Modules

| Module | Kernel-side | Purpose |
|--------|-------------|---------|
| `boot.S` | assembly | PVH entry note, 32→64-bit bootstrap |
| `main.rs` | Rust | Entry point, boot sequencing, hardware init |
| `paging.rs` | Rust | 4-level page tables, bitmap frame allocator, COW |
| `process.rs` | Rust | PCB, IPC queues, round-robin scheduler, fork/exec |
| `syscall.rs` | Rust | Syscall dispatch, IDT setup, MSRs, IRQ handlers |
| `tss.rs` | Rust | Task State Segment (kernel stack on ring transitions) |
| `interrupt_server.rs` | Rust | IRQ handler table, forwarding to user-space drivers |
| `vga.rs` | Rust | Fallback VGA writes, serial debug output |
| `elf.rs` | Rust | ELF-64 parser, LOAD-segment mapper |
| `filesystem.rs` | Rust | Embedded flat filesystem with open/read/close |
| `block_dev.rs` | Rust | 128 KiB RAM disk with 512-byte block interface |
| `capability.rs` | Rust | Capability-bitmap grant/revoke/check |
| `signal.rs` | Rust | Signal pending bitmap, delivery, handler dispatch |
| `user_program.rs` | Rust | Built-in user-space programs (VGA, keyboard, init, shell) |
| `linker.ld` | script | Kernel ELF layout, PVH note placement |

### Virtual Address Space (per-process)

```
       PML4[0] → PDPT[0] → PD (512 × 2 MiB entries)
  ┌──────────────────────────────────────────────┐
  │ PD[0]: 0x00000000-0x001FFFFF  Kernel text    │  U=0
  │ PD[1]: 0x00200000-0x003FFFFF  Kernel data    │  U=0
  │ PD[2]: 0x00400000-0x005FFFFF  4K PT for user  │  U=1  ← code mapped here
  │ PD[3]: 0x00600000-0x007FFFFF  VGA buffer map  │  U=1  ← VGA server
  │ PD[4-6]: not present                          │
  │ PD[7]: 0x00E00000-0x00FFFFFF  User stack       │  U=1  (2 MiB huge page)
  └──────────────────────────────────────────────┘
```

### Physical Memory

| Range | Use |
|-------|-----|
| `0x000000 - 0x0FFFFF` | Reserved (real mode IVT, BIOS data, EBDA) |
| `0x100000 - 0x3FFFFF` | Kernel .text, .rodata, .data, .bss (identity mapped) |
| `0x0B8000` | VGA text buffer (80×25 colour, mapped to 0x600000 for VGA server) |
| `0x400000 - 0x7FFFFFF` | Free — managed by bitmap frame allocator (32512 frames) |

## System Calls (24 total)

User programs request kernel services via the `syscall` instruction:

| NR | Name | Args | Description | Phase |
|----|------|------|-------------|-------|
| 0 | EXIT | (code) | Terminate process | 0 |
| 1 | WRITE | (fd, buf, len) | Write to stdout/stderr | 1 |
| 3 | IPC_SEND | (dst_pid, msg_ptr) | Send 64-byte IPC message | 1 |
| 4 | IPC_RECV | (buf_ptr) | Receive IPC message | 1 |
| 6 | GETPID | () | Get current process ID | 1 |
| 7 | VGA_WRITE | (byte) | Write character to screen | 1 |
| 8 | VGA_CLEAR | () | Clear VGA screen | 1 |
| 9 | SCHEDULE | () | Yield CPU cooperatively | 1 |
| 10 | GET_SERVER_PID | (type) | Get PID of well-known server | 1 |
| 13 | FORK | () | Clone current process (COW) | **2** |
| 14 | EXEC | (entry, name) | Replace with built-in program | **2** |
| 15 | REGISTER_IRQ | (vector, pid) | Register IRQ handler | **2** |
| 16 | CAP_SEND | (dst_pid) | Check IPC capability | **4** |
| 17 | REGISTER_SIGNAL | (sig, handler) | Register signal handler | **4** |
| 18 | WAIT_SIGNAL | (timeout) | Wait for signal | **4** |
| 19 | EXEC_ELF | (data, size) | Load ELF binary from memory | **3** |
| 20 | OPEN | (name, max) | Open a file from embedded FS | **3** |
| 21 | FS_READ | (fd, buf, count) | Read from open file | **3** |
| 22 | CLOSE | (fd) | Close file descriptor | **3** |
| 23 | READDIR | (buf, size) | List files in directory | **3** |

## The 4 Phases

### Phase 1 — Microkernel IPC (Interrupt Server)

**Problem**: The original keyboard handler pushed scancodes directly into the keyboard
server's kernel queue — kernel code touching user-server data structures.

**Solution**: An interrupt-server abstraction layer. When IRQ1 fires, the kernel
calls `forward_irq()`. If a user-space interrupt server is registered, the
message goes there; otherwise it falls back to direct delivery to registered
handlers. This is the "true microkernel" IRQ path.

**Key files**: `interrupt_server.rs`, `syscall.rs` (`keyboard_irq_handler` → `forward_irq`)

### Phase 2 — Process Creation

**Problem**: All processes were hardcoded at boot — no way to create new ones.

**Solution**:
- **`FORK`** (syscall 13): Clones the calling process with a COW page table.
  The child gets its own PID, a private kernel stack, and a copy of the parent's
  registers (with `rax = 0`). Parent receives the child's PID.
- **`EXEC`** (syscall 14): Replaces the current process image with a new built-in
  program. Unmaps old user pages, maps new code at `0x400000`, resets the stack.
- **COW (Copy on Write)**: `copy_page_table_cow()` marks all user pages as
  read-only with a COW flag (PTE bit 9). The first write by either process
  traps to `handle_cow_fault()`, which allocates a fresh frame, copies the data,
  and marks the page writable for the faulting process only.

**Key files**: `process.rs` (`fork_process`, `exec_builtin`), `paging.rs` (`copy_page_table_cow`, `handle_cow_fault`)

### Phase 3 — ELF Loader + Filesystem

**Problem**: The kernel could only load pre-compiled Rust functions — no
user-space program loader, no file abstraction.

**Solution**:
- **ELF loader** (`elf.rs`): Parses ELF-64 headers and program headers,
  validates magic/class/type, maps LOAD segments into the process's page table
  with correct permissions, zero-fills `.bss`. Called via `EXEC_ELF` (syscall 19)
  with an in-memory ELF image.
- **Embedded filesystem** (`filesystem.rs`): A flat directory table with 3
  built-in files (`hello.txt`, `README`, `boot.cfg`). Provides `open`/`read`/
  `close`/`readdir` via syscalls 20-23. Per-process FD table (8 FDs per process).
- **RAM disk** (`block_dev.rs`): 128 KiB backing store initialized at boot.
  512-byte block interface with cross-boundary reads.

### Phase 4 — System Hardening

**Problem**: No access control — any process could send IPC to any other,
and exceptions killed the whole system.

**Solution**:
- **Capabilities** (`capability.rs`): Each process holds a 64-bit bitmap of
  capability tokens. `SYS_IPC_SEND` now checks `capability::check_ipc_send()`
  against the destination PID. A process must hold the right capability
  (VGA_SEND, KEYBOARD_SEND, DRIVER_SEND) to send to that server.
- **Signals** (`signal.rs`): Per-process 64-bit pending-signal bitmap and
  handler-address table. `deliver_pending()` is called on every context switch.
  Actions: `Kill` (terminate), `Ignore`, or `Handler` (redirects RIP to the
  registered handler address, pushing old RIP on the user stack).
- **Syscalls**: `REGISTER_SIGNAL` (17), `WAIT_SIGNAL` (18), `CAP_SEND` (16).

## Expected VGA Output

The kernel writes progress indicators to the top-left of the screen:

```
1  2  3  4  V  K  I  S  5  T  U
│  │  │  │  │  │  │  │  │  │  └─ Switched to user space
│  │  │  │  │  │  │  │  │  └─── Timer configured
│  │  │  │  │  │  │  │  └───── All processes spawned (5)
│  │  │  │  │  │  │  └─────── Shell (PID 4) ready
│  │  │  │  │  │  └───────── Init (PID 3) running
│  │  │  │  │  └─────────── Keyboard server (PID 2)
│  │  │  │  └───────────── VGA server (PID 1)
│  │  │  └─────────────── IDT configured (4)
│  │  └───────────────── Module init passed (3)
│  └─────────────────── Entered _start (2)
└───────────────────── Entered Rust (1)
```

After boot, the shell prompt `Shell>` appears and keyboard input is echoed.

## Building

```bash
cargo build --target x86_64-unknown-none
```

Output: `target/x86_64-unknown-none/debug/kernel` (linked with `-T kernel/src/linker.ld`)

## Running

```bash
qemu-system-x86_64 -kernel target/x86_64-unknown-none/debug/kernel \
  -m 128M -machine pc-q35-9.2 -vga std -serial stdio
```

Or use the wrapper:
```bash
./run.sh --gui          # GUI window
./run.sh --nographic    # Serial console
```

## Learning Topics

| Topic | What you'll learn | Look at |
|-------|-------------------|---------|
| Boot | How PVH hands off, 32→64-bit transition | `boot.S`, `main.rs::_start` |
| Paging | 4-level page tables, CR3 switching, TLB | `paging.rs` |
| Frame allocation | Bitmap allocator, free lists, physical memory | `paging.rs::init_frame_allocator` |
| Processes | PCB design, states, scheduling | `process.rs` |
| Context switching | Saving/restoring registers, IRQ-driven | `syscall.rs::timer_save_and_switch` |
| Syscalls | `syscall`/`sysretq` MSRs, dispatch table | `syscall.rs::syscall_entry` |
| IPC | Lock-free SPSC queues, kernel-copy | `process.rs::IpcMessage` |
| IRQ handling | PIC remap, PIT timer, keyboard | `syscall.rs::keyboard_irq_handler` |
| Fork/COW | Copy-on-write, page-table cloning | `process.rs::fork_process`, `paging.rs::copy_page_table_cow` |
| ELF loading | Program headers, segment mapping | `elf.rs::load_elf` |
| Filesystem | Directory tables, open/read/close | `filesystem.rs` |
| Capabilities | Bitmap-based access control | `capability.rs` |
| Signals | Pending bitmaps, handler dispatch | `signal.rs` |

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Bitmap frame allocator | Simple, correct, supports `free_frame()` — unlike a bump allocator |
| COW on bit 9 (OS-reserved) | Works with existing x86-64 page-table entry format, no special CPU support needed |
| Fixed-size process table (8) | No allocator needed; simple for learning |
| Fixed-size IPC queues (16 msgs) | Lock-free ring buffer — no mutexes or wait queues |
| In-kernel keyboard scancode read | PS/2 controller buffers 1 byte; must read in ~µs at ring 0 |
| Interrupt server fallback | Maintains backward compatibility if no user-space ISR exists |
| Embedded filesystem | Avoids block-driver complexity; demonstrates open/read/close pattern |
| 64-bit capability bitmap | Atomic-bitmap check is O(1); simple to implement |

## References

- [OSDev Wiki](https://wiki.osdev.org/)
- [PVH Boot Protocol](https://wiki.xenproject.org/wiki/PV_hvm_domains)
- [Rust OSDev](https://github.com/rust-osdev/rust-osdev.github.io)
- [Intel SDM Volume 3](https://software.intel.com/en-us/articles/intel-sdm)
- [The L4 Microkernel Family](https://en.wikipedia.org/wiki/L4_microkernel_family)
