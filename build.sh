#!/bin/bash

# Build script for the real Rust kernel (PVH path only).

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "=== Building OS Kernel ==="
echo "Rust toolchain:"
rustc --version
cargo --version
echo ""

rm -f target/pvh-kernel

echo "[1/2] Compiling Rust kernel (ELF PVH)..."
RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld -C relocation-model=static -C link-arg=-no-pie" \
    cargo +nightly build -Z build-std=core,compiler_builtins \
    --target x86_64-unknown-none \
    --release \
    --bin kernel

cp target/x86_64-unknown-none/release/kernel target/pvh-kernel
echo "      -> target/pvh-kernel"

file target/pvh-kernel
readelf -n target/pvh-kernel || true

echo "[2/2] Build completed successfully"
echo ""
echo "Run with QEMU:"
echo "  ./run.sh --gui --interactive"
