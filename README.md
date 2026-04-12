# LearnOS - Educational Microkernel

A minimal x86_64 microkernel written in Rust for learning purposes.

## What is a Microkernel?

A microkernel is an operating system design where ONLY essential code runs in privileged mode (kernel space). Everything else runs in unprivileged user space:

```
┌─────────────────────────────────────────────────┐
│                    KERNEL SPACE                 │
│  ┌──────────────────────────────────────────┐  │
│  │ - Process scheduling                     │  │
│  │ - Memory management (paging)            │  │
│  │ - Inter-process communication (IPC)     │  │
│  │ - System call interface                 │  │
│  └──────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
                          │ Syscalls
          ┌───────────────┼───────────────┐
          ▼               ▼               ▼
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   Init      │  │   Shell     │  │   VGA       │
│  Process    │  │  Process    │  │   Service   │
└─────────────┘  └─────────────┘  └─────────────┘
                     Userspace
```

In a **monolithic kernel**, all OS code runs in kernel space. If any part crashes, the whole system goes down.

In a **microkernel**, each service runs in its own protected address space. If init crashes, the shell keeps running!

## Architecture

### Boot Sequence

1. **BIOS/UEFI** loads kernel from disk to memory
2. **boot.S** (assembly):
   - Enable 64-bit mode
   - Set up page tables (1GB identity mapped)
   - Load Global Descriptor Table
   - Jump to Rust `_start()`
3. **main.rs** (Rust):
   - Initialize VGA display
   - Set up IDT (interrupt handlers)
   - Create process table
   - Set up timer for scheduling
   - Switch to user space

### Key Components

| Component | File | Description |
|-----------|------|-------------|
| Bootstrap | `boot.S` | 32-bit → 64-bit transition, PVH boot |
| Main | `main.rs` | Kernel entry point, initialization |
| Process | `process.rs` | PCB, scheduling, context switch |
| Syscall | `syscall.rs` | System call handlers, IDT |
| Paging | `paging.rs` | 4-level paging, user/kernel separation |
| IPC | `ipc.rs` | Message queue (lock-free SPSC) |
| Keyboard | `keyboard.rs` | PS/2 driver (polling mode) |
| VGA | `vga.rs` | Text mode display driver |
| Device Manager | `device_manager.rs` | HAL facade for services |

### Memory Layout

```
Physical Memory:
  0x000000 - 0x40000000: Identity mapped (1GB)
  0x0B8000: VGA text buffer (80x25 color)
  0x100000: Kernel load address (PVH)
   
Virtual (User Space):
  0x400000: User code
  0x1001000: User stack
  
Virtual (Kernel Space):
  0xFFFF800000000000+: Kernel addresses
```

### Process Management

Each process has a **PCB (Process Control Block)**:
```rust
struct Process {
    pid: u16,           // Unique ID
    state: State,       // READY, RUNNING, BLOCKED
    entry_point: u64,   // Code start address
    registers: ...,     // Saved CPU state
    memory_regions: [], // Code, data, stack
}
```

### Context Switching

When the timer fires (~100 times/sec):
1. Save current process registers to its PCB
2. Find next READY process
3. Load its registers from its PCB
4. Jump to where it was!

### System Calls

User programs use the `syscall` instruction to request kernel services:

```
User mode:     rax = 7 (VGA_WRITE)
               rdi = 'A' (character)
               syscall

Kernel mode:   Handle the request
               Return to user mode
```

## System Calls Available

| NR | Name | Args | Description |
|----|------|------|-------------|
| 0 | EXIT | (code) | Terminate process |
| 6 | GETPID | () | Get current PID |
| 7 | VGA_WRITE | (byte) | Write char to screen |
| 8 | VGA_CLEAR | () | Clear screen |
| 9 | SCHEDULE | () | Yield CPU |
| 10 | WRITE | (fd, buf, len) | Write to file/stdout |

## Building

```bash
./build.sh
```

Output: `target/pvh-kernel` (67KB ELF64)

## Running

```bash
qemu-system-x86_64 -kernel target/pvh-kernel -m 128 -machine q35 -vga std -display gtk
```

Or use the wrapper:
```bash
./run.sh --gui --interactive
```

## Expected VGA Output

The kernel displays progress indicators on screen:
- `1`: Entered Rust _start
- `2`: Got device manager
- `3`: VGA initialized
- `4`: IDT configured
- `5`: Process table created
- `I`: Init process running
- `S`: Shell process ready
- `T`: Timer configured
- `U`: Switched to user mode

Then user processes (init, shell) output their own characters via syscalls.

## Files

```
kernel/src/
├── boot.S           # Bootstrap (assembly) - 32→64-bit
├── main.rs          # Kernel entry point
├── process.rs       # PCB, scheduling, context switch
├── syscall.rs       # Syscall handlers, IDT setup
├── paging.rs        # 4-level page tables
├── ipc.rs           # Message queue (lock-free)
├── keyboard.rs      # PS/2 keyboard driver
├── vga.rs           # Text mode VGA driver
├── device_manager.rs # HAL facade
├── elf.rs           # ELF loader (unused, demo)
├── userspace/       # User-space stubs
│   └── mod.rs       # VFS wrapper
├── user_program.rs  # Demo user programs
└── linker.ld        # Linker script (PVH)
```

## Is it a Real Microkernel?

**Yes.** This kernel implements core microkernel principles:

- ✅ Kernel only handles scheduling, memory, syscalls
- ✅ Services (init, shell) run in user space
- ✅ Communication via syscalls/IPC
- ✅ Memory protection via paging
- ✅ Preemptive multitasking (timer interrupt)

**Limitations** (educational):
- No CR3 switching per process (shared page table)
- Demo user programs compiled into kernel
- No real ELF loading from disk

## Learning Topics

This kernel demonstrates:

1. **Boot**: How does a CPU start in 16-bit and end up in 64-bit mode?
2. **Paging**: What are page tables and why do we need them?
3. **Processes**: How does one CPU run "multiple" programs?
4. **Context Switching**: How does the scheduler work?
5. **System Calls**: How do user programs ask the kernel for help?
6. **IDT**: What happens when a hardware interrupt fires?
7. **ELF**: What format are executables in?
8. **IPC**: How do processes communicate?

## Inline Assembly

Only 9 places use inline assembly (necessary for x86 hardware):
- Boot: First code executed (can't be Rust)
- `syscall` instruction: The only way to do syscalls in x86-64
- `in/out` instructions: Access legacy hardware (PIC, PIT, keyboard)
- `lidt`: Load interrupt descriptor table
- `jmp` for context switch: Direct control transfer

Everything else (VGA text, memory management, process table, IPC) is **100% Rust**.

## References

- [OSDev Wiki](https://wiki.osdev.org/)
- [PVH Boot Protocol](https://wiki.xenproject.org/wiki/PV_hvm_domains)
- [Rust Bare Metal](https://github.com/rust-osdev/rust-osdev.github.io)
- [Intel SDM](https://software.intel.com/en-us/articles/intel-sdm) Volume 3