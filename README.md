# LearnOS - PVH + VGA Rust Kernel 🚀

Minimal x86_64 kernel in Rust (`no_std`) that boots in QEMU through PVH, switches to long mode in an assembly bootstrap, writes text to the classic VGA text buffer (`0xb8000`), and echoes keyboard input from the PS/2 controller.

## 📋 Project Overview

- ✅ Bare-metal Rust kernel with custom entrypoint (`_start`) and panic handler
- ✅ PVH boot through a Xen ELF note (`XEN_ELFNOTE_PHYS32_ENTRY`)
- ✅ 32-bit to 64-bit transition in `kernel/src/boot.S`
- ✅ Identity paging setup for low memory and kernel region
- ✅ Direct VGA text-mode writes (memory-mapped I/O)
- ✅ Direct keyboard polling from ports `0x64` and `0x60`

## 🚀 Current Boot Model

This repository uses one runtime path:

- `qemu-system-x86_64 -kernel target/pvh-kernel ...`

There is no BIOS boot-sector runtime path in the active workflow.

## 🏗️ Repository Layout

- `kernel/src/main.rs` - Rust kernel entry (`_start`), line editor loop, panic handler
- `kernel/src/boot.S` - PVH entry note and 32-bit bootstrap to long mode
- `kernel/src/vga.rs` - VGA text writer (clear, write byte, backspace, print)
- `kernel/src/keyboard.rs` - PS/2 scan-code polling and decode table
- `kernel/src/linker.ld` - ELF layout and section placement
- `build.sh` - Build script for PVH kernel artifact
- `run.sh` - QEMU launcher (GUI by default, interactive by default)
- `verify.sh` - ELF/PVH note quick verification
- `test.sh` - Boot smoke test with timeout

## 🛠️ Build

Prerequisites:

```bash
rustup install nightly
rustup target add x86_64-unknown-none --toolchain nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
sudo apt install qemu-system-x86_64
```

Build artifact:

```bash
./build.sh
```

Output:

- `target/pvh-kernel` (ELF64 ET_EXEC with Xen PVH note)

## 🤖 Run

Recommended interactive run (VGA window + keyboard focus):

```bash
./run.sh --interactive
```

Quick timeout run:

```bash
./run.sh --timeout 5
```

Debug run (QEMU debug flags + nographic):

```bash
./run.sh --debug --timeout 5
```

## 🔩 Run Expected Behavior

After boot handoff, the kernel clears the screen and prints:

```text
Hello World!
Keyboard echo demo
Type text and press Enter
> 
```

Typing and pressing Enter prints:

```text
Echo: <line>
> 
```

## 🧪 Verify

```bash
./verify.sh
./test.sh --timeout 5
```

## 📝 Notes

- VGA output requires the QEMU graphical window (`run.sh` defaults to GUI mode).
- In `-nographic`, VGA memory writes are not visible on terminal.
- Keyboard input is polling-based (no IRQ/IDT yet), intentionally simple for early kernel bring-up.
