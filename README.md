# OS Kernel Project - Hello World Kernel 🚀

An operating system kernel project that demonstrates "Hello World!" output in QEMU using Rust.

## 📋 Project Overview

This project implements a basic x86_64 kernel that:
- ✅ Compiles as a `no_std` Rust executable
- ✅ Includes PVH note for modern QEMU compatibility
- ✅ Displays "Hello World!" using VGA buffer
- ✅ Supports modern features like PVH for QEMU
- ✅ Includes verification scripts and testing utilities

## 🏗️ Kernel Architecture

### Main Components:

1. **`kernel/src/main.rs`** - Kernel entry point with multiboot header
2. **`kernel/src/linker.ld`** - Linker script defining memory layout
3. **`kernel/Cargo.toml`** - Build configuration for no_std compilation
4. **`scripts/verify-boot.sh`** - Multiboot format verification script
5. **`target/boot.img`** - Bootable disk image for QEMU

### Implemented Features:

- **Multiboot2 Stub**: Assembly header source for GRUB/multiboot experiments
- **PVH Support**: PVH note for modern QEMU support
- **VGA Output**: Direct printing to VGA buffer (0xb8000)
- **Boot Sector**: Simple boot alternative for QEMU
- **Build Pipeline**: Compilation with nightly Rust and build-std

## 🛠️ Build Instructions

### Prerequisites

```bash
# Install Rust nightly (required for no_std)
rustup install nightly
rustup default nightly

# Install required tools
sudo apt install qemu-system-x86_64 nasm
```

### Kernel Compilation

```bash
# Build the kernel (from project root)
RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld" \
    cargo +nightly build -Z build-std=core,compiler_builtins \
    --target x86_64-unknown-none \
    --release --bin kernel

# Create boot image (working boot method)
./build.sh

```

### Format Verification

```bash
# Verify raw kernel markers (PVH + multiboot magic)
./verify.sh

# Verify boot image
./scripts/verify-boot.sh
```

### Method 2: PVH (`-kernel`) (Experimental)

```bash
timeout 10s qemu-system-x86_64 \
    -kernel target/pvh-kernel \
    -m 128M \
    -machine q35,accel=kvm:tcg \
    -cpu host \
    -nographic
```

Use Method 1 for the most reliable educational demo path.

## 🚀 QEMU Execution

### Method 1: Boot Sector (Recommended)

```bash
# Run bootable disk image (shows "Hello World!")
timeout 10s qemu-system-x86_64 \
    -drive format=raw,file=target/boot.img \
    -boot order=a \
    -m 32 \
    -nographic
```

## 🔍 Project Structure

```
.
├── kernel/                    # Código fuente del kernel
│   ├── src/
│   │   ├── main.rs           # Punto de entrada con multiboot header
│   │   └── linker.ld         # Script de linker
│   └── Cargo.toml           # Configuración de Rust
├── scripts/                  # Scripts de utilidad
│   └── verify-boot.sh       # Verificación multiboot
├── target/                   # Archivos generados
│   ├── boot.img             # Imagen de disco bootable
│   ├── pvh-kernel           # Kernel con soporte PVH
│   └── boot-sector.bin      # Boot sector binario
└── README.md                # Este archivo
```

## 🧪 Testing and Verification

### Verification Commands

```bash
# Verify boot image format/signature
./scripts/verify-boot.sh

# Verify raw kernel markers
./verify.sh
```

### Expected Output

When running the kernel, you should see:
1. SeaBIOS screen at startup
2. "Hello World!" printed on screen
3. QEMU stops after 10 seconds

## 🔧 Developing with This Project

### Adding Features

1. **VGA Mode**: VGA buffer is at `0xb8000`
2. **Multiboot Stub**: Header source in `kernel/src/boot.S`
3. **PVH Support**: Note in `.note.pvh` section
4. **Entry Point**: `_start()` entry function

### Debugging

```bash
# Build with debugging information
RUSTFLAGS="-C link-arg=-Tsrc/linker.ld -g" \
    cargo +nightly build -Z build-std=core,compiler_builtins \
    --target x86_64-unknown-none --release --bin kernel
```

## 📚 Additional Resources

### Multiboot Specification
- [Multiboot Specification](https://www.gnu.org/software/grub/manual/multiboot/multiboot.html)
- [PVH Boot for QEMU](https://www.qemu.org/docs/master/system/qemu-manpage.html)

### Rust for Kernel Development
- [The Rust Embedded Book](https://docs.rust-embedded.org/book/)
- [no_std in Rust](https://doc.rust-lang.org/reference/no_std.html)

### QEMU for Kernel Development
- [QEMU System Emulation](https://www.qemu.org/docs/master/system/index.html)
- [QEMU Boot Options](https://www.qemu.org/docs/master/system/qemu-manpage.html)

## 🤝 Contributions

This project is designed as an educational resource. Feel free to:

1. Extend the kernel with more features
2. Add explanatory comments
3. Improve documentation
4. Add new examples

## 📄 License

This project is open source and intended for educational purposes. MIT Licence

---

**Happy Hacking! 🎉**
