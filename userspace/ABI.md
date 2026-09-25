# Userspace ABI

Contract between a standalone `userspace` ELF and the LearnOS microkernel.
The kernel provides mechanisms (address spaces, syscalls, IPC, capabilities,
interrupt/resource grants); service policy and device protocols live in user
processes.

## ELF images and address-space layout

| Property | Value |
|---|---|
| ELF class | ELF-64, little-endian, x86-64 |
| `e_type` | `ET_EXEC` (2), not PIE/`DYN` |
| User ELF window | `[0x10000000, 0x20000000)` |
| User stack | private 64 KiB ending at `0x00007FFFFFFFF000` |
| VGA alias | `0x20000000` (user-mapped device page, NX) |
| VirtIO DMA queue alias | `0x30000000` (only mapped in the storage server) |

Every `PT_LOAD`, including any load segment containing the ELF/program headers,
must lie wholly inside the ELF window. `kernel/build.rs` checks the generated
images before embedding them. The kernel retains a supervisor-only identity map
of the first 128 MiB; no part of that mapping is user-accessible. Each process
gets separate user page frames and a private stack.

`userspace/build.sh` links with `--image-base=0x10000000 -no-pie -e _start`.
`_start` sets RSP to the private stack top passed by the address-space contract,
then calls that binary's `main`.

## Syscall convention

The `syscall` instruction uses:

| Register | Role |
|---|---|
| `rax` | syscall number and return value |
| `rdi`, `rsi`, `rdx` | arguments 1–3 |
| `r10`, `r8` | arguments 4–5 |

The instruction and kernel entry clobber `rcx`, `r11`, and caller-saved argument
registers. The Rust wrappers declare those clobbers. User pointers are accepted
only when every touched page is present, user-accessible, and has the requested
read/write permission.

## IPC message layout

`IpcMessage` is a 64-byte `repr(C)` structure shared byte-for-byte with the
kernel: `src_pid: u16`, `msg_type: u16`, and `data: [u8; 60]`. A compile-time
assertion in `src/lib.rs` protects the size. The kernel replaces `src_pid` with
the actual sender PID.

## Storage-server device grant

When a legacy VirtIO-blk PCI function is found, the kernel sends the storage
server an initial `MSG_STORAGE_CONFIG` IPC message:

- `data[0..2]`: BAR0 I/O-port base (`u16`, little-endian)
- `data[2..10]`: physical base of four contiguous DMA pages (`u64`, little-endian)

The kernel maps those pages at `DMA_QUEUE_VADDR` only in that process and grants
the BAR0 port range through the TSS I/O-permission bitmap. The process runs the
VirtIO request protocol and FAT16 parser itself. Ring 3 never receives global
IOPL=3 or access to unrelated ports.

## User-facing build check

```bash
./build.sh vga --out /tmp/learnos-images
readelf -h -l /tmp/learnos-images/vga.elf
```

The expected type is `EXEC`, and every `LOAD` virtual range must be within the
user ELF window above. The kernel build performs the same validation for all
embedded service images.
