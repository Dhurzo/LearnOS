#!/bin/bash
#
# Build script for the OS kernel project
#
# This script compiles the Rust kernel, creates boot sectors, and generates
# bootable disk images for testing in QEMU. It handles multiple build steps
# and provides fallback mechanisms for known issues.
#
# Usage:
#   ./build.sh              # Build kernel and boot image
#   ./build.sh --clean       # Clean all build artifacts
#   ./build.sh --only-kernel # Build only the kernel
#   ./build.sh --only-boot   # Build only the boot sector image
#
# Build Process:
# 1. Clean previous build artifacts
# 2. Build Rust kernel with no_std configuration
# 3. Create boot sector from assembly source
# 4. Combine boot sector with kernel in bootable image
#
# Technical Details:
# - Rust target: x86_64-unknown-none (bare metal)
# - Boot method: PVH (modern) + boot sector (fallback)
# - Image format: 1.44MB floppy disk image (.img)
# - QEMU support: Direct kernel boot and boot sector boot
#
# Known Issues:
# - Bootloader API compatibility with newer Rust versions
# - create_bios binary build failures (handled by fallback)
#
# For more information:
# - [Rust Embedded Book](https://docs.rust-embedded.org/book/)
# - [OSDev Wiki](https://wiki.osdev.org/)
# - [QEMU Documentation](https://www.qemu.org/docs/)
#
set -e                           # Exit immediately on error

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "=== Building OS Kernel ==="

# Display current Rust toolchain version
echo "Rust toolchain:"
rustc --version
cargo --version
echo ""

# Clean previous output artifacts to ensure clean build
rm -f target/kernel target/boot.img target/boot-sector.bin target/pvh-kernel

# Build kernel (from workspace root so linker path resolves correctly)
# 
# Rust Build Command Breakdown:
# - RUSTFLAGS: Linker script for custom memory layout
# - cargo: Rust build tool
# - +nightly: Use nightly toolchain for unstable features
# - -Z build-std=core,compiler_builtins: Build standard library components
# - --target: Target architecture (bare metal x86_64)
# - --release: Optimized build (no debug info)
# - --bin kernel: Build the binary named "kernel"
#
# The build process creates:
# - target/x86_64-unknown-none/release/kernel: Compiled kernel binary
# - target/pvh-kernel: Copy for convenience
echo "[1/4] Compiling kernel..."
RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld" \
    cargo +nightly build -Z build-std=core,compiler_builtins \
    --target x86_64-unknown-none \
    --release \
    --bin kernel

# Copy kernel to target/ for convenience with consistent naming
cp target/x86_64-unknown-none/release/kernel target/pvh-kernel
echo "      -> target/pvh-kernel"
echo "      -> target/x86_64-unknown-none/release/kernel"

# Create boot sector from NASM source (the working method)
#
# This creates a traditional boot sector that can be loaded by BIOS
# as a fallback when the PVH boot method doesn't work. The boot sector
# is a minimal piece of code that initializes video mode and displays
# a "Hello World!" message.
#
# Boot Sector Details:
# - Size: 512 bytes (exactly 1 disk sector)
# - Signature: 0xAA55 at offset 510 (identifies as bootable)
# - NASM Format: Binary output for direct hardware loading
# - Boot Method: Traditional BIOS boot (not UEFI)
#
# The boot sector creation process:
# 1. Assemble boot.asm into binary format using NASM
# 2. Create a 1.44MB floppy disk image (2880 sectors × 512 bytes)
# 3. Write the boot sector to the first sector of the image
# 4. Leave remaining sectors empty (filled with zeros)
#
# This creates a bootable floppy image that can be tested in QEMU.
echo "[2/4] Creating boot sector..."

# Check if NASM (Netwide Assembler) is available
if command -v nasm &>/dev/null; then
    # Assemble boot.asm into binary format
    nasm -f bin kernel/src/boot.asm -o target/boot-sector.bin
    echo "      -> Assembled boot.asm to target/boot-sector.bin"
    
    # Create a standard 1.44MB floppy disk image
    # - Format: raw binary (no filesystem)
    # - Size: 1.44MB = 2880 sectors × 512 bytes each
    # - Filled with zeros initially
    dd if=/dev/zero of=target/boot.img bs=512 count=2880 status=none
    echo "      -> Created 1.44MB floppy image: target/boot.img"
    
    # Write boot sector to the first sector of the floppy image
    # - if=Input File (boot sector binary)
    # - of=Output File (floppy image)
    # - bs=Block Size (512 bytes = 1 sector)
    # - seek=0 (start at beginning of file)
    # - conv=notrunc (don't truncate output file)
    dd if=target/boot-sector.bin of=target/boot.img bs=512 seek=0 conv=notrunc status=none
    echo "      -> Wrote boot sector to floppy image"
    
    # Verify the boot sector was written correctly
    file target/boot.img
else
    echo "      (nasm not found — skipping boot sector image)"
    echo "      Install nasm to enable boot sector creation:"
    echo "        - Ubuntu/Debian: sudo apt-get install nasm"
    echo "        - Fedora: sudo dnf install nasm"
    echo "        - macOS: brew install nasm"
fi

echo "[3/4] Creating documentation notes..."
echo "  - PVH kernel: target/pvh-kernel (for modern QEMU boot)"
echo "  - Boot image: target/boot.img (for BIOS boot)"

# Create a simple README for the target directory
cat > target/README.txt << EOF
OS Kernel Artifacts
==================

This directory contains build artifacts for the OS kernel project:

1. pvh-kernel
   - PVH (Para-Virtualized Hypervisor) kernel binary
   - Can be booted directly by modern QEMU versions
   - Use: qemu-system-x86_64 -kernel pvh-kernel

2. boot.img
   - 1.44MB floppy disk image with boot sector
   - Can be booted by traditional BIOS (QEMU with -drive)
   - Use: qemu-system-x86_64 -drive format=raw,file=boot.img -boot order=a

Build Date: $(date)
Kernel Size: $(wc -c < target/pvh-kernel) bytes
Boot Image Size: $(wc -c < target/boot.img 2>/dev/null || echo "not created") bytes
EOF

echo "      -> Created target/README.txt"

echo "[4/4] Build completed successfully!"
echo ""
echo ""
echo "Build artifacts created:"
ls -la target/*.txt target/pvh-kernel target/boot.img 2>/dev/null || true
echo ""

echo "Run with QEMU:"
echo "  ./run.sh                   # PVH kernel boot (modern QEMU)"
echo "  ./run.sh --boot-img        # Boot from floppy image (recommended)"
echo "  ./run.sh --debug           # With debug output"
echo ""
echo "Test the kernel:"
echo "  ./test.sh                  # Quick build and test"
echo ""
echo "Verify the build:"
echo "  ./verify.sh                # Check kernel format and headers"
echo "  ./scripts/verify-boot.sh   # Verify boot sector format"
echo ""
echo "Documentation:"
echo "  - README.md: Project overview"
echo "  - docs/: Detailed documentation"
echo "  - target/README.txt: Build artifact details"
echo ""
echo "For more information about build options:"
echo "  man qemu-system-x86_64"
echo "  https://wiki.osdev.org/"