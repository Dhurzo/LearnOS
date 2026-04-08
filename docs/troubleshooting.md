# Troubleshooting

This guide covers common issues when building and running the kernel.

## Build Issues

### Rust nightly not installed

**Symptoms:**
```
error: 'x86_64-unknown-none' is not a supported target
```
or
```
error: the `-Z` flag is only accepted on the nightly compiler
```

**Fix:**
```bash
rustup install nightly
rustup default nightly
rustup component add rust-src llvm-tools-preview
```

The `rust-src` component is required by `-Z build-std` to recompile core for the bare-metal target.

### x86_64-unknown-none target not available

**Symptoms:**
```
error: could not find `x86_64-unknown-none` in target list
```

**Fix:**
```bash
rustup target add x86_64-unknown-none --toolchain nightly
```

### Build fails with "can't find crate for `std`"

**Symptoms:**
```
error[E0463]: can't find crate for `std`
```

**Cause:** The `RUSTFLAGS` or `-Z build-std` flags are missing, so the compiler tries to link against the hosted standard library which doesn't exist for `x86_64-unknown-none`.

**Fix:** Use the build script instead of running `cargo build` directly:
```bash
./build.sh
```

Or ensure the full command is used:
```bash
RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld" \
    cargo +nightly build -Z build-std=core,compiler_builtins \
    --target x86_64-unknown-none --release --bin kernel
```

### Linker script not found

**Symptoms:**
```
error: linker `cc` not found
```
or linker errors about undefined sections.

**Fix:** Ensure the linker script path in `RUSTFLAGS` is relative to the workspace root:
```bash
RUSTFLAGS="-C link-arg=-Tkernel/src/linker.ld"
```

### Cargo.lock conflicts

**Fix:**
```bash
cargo clean
rm Cargo.lock
./build.sh
```

## QEMU Issues

### QEMU not installed

**Symptoms:**
```
qemu-system-x86_64: command not found
```

**Fix:**
```bash
# Ubuntu/Debian
sudo apt-get install qemu-system-x86

# Fedora
sudo dnf install qemu-system-x86

# macOS
brew install qemu
```

### QEMU exits immediately with PVH boot

**Symptoms:** QEMU prints nothing and exits right away.

**Possible causes:**
1. The kernel binary doesn't have the PVH note. Verify:
   ```bash
   readelf -n target/pvh-kernel | grep -i pvh
   ```
2. Wrong QEMU flags. Make sure to include `-m 128M` (the kernel needs some memory):
   ```bash
   qemu-system-x86_64 -kernel target/pvh-kernel -m 128M -nographic
   ```

### QEMU timeout is normal

**Symptoms:**
```
timeout: command timed out after 5 seconds
```

**This is expected behavior.** The kernel prints "Hello World!" and enters `loop {}`. It never exits on its own. The timeout just means QEMU ran for the allotted time without crashing — which is success for a minimal kernel.

To see the output interactively, run without a timeout and press Ctrl+C when done:
```bash
qemu-system-x86_64 -drive format=raw,file=target/boot.img -boot order=a -m 32 -nographic
```

### No "Hello World!" visible on screen

Behavior depends on boot path:

- **Boot image mode (`--boot-img`)**: with this project, `-nographic` usually still shows BIOS text output in terminal.
- **PVH mode (`-kernel`)**: VGA text may not be visible in `-nographic` unless you add serial output.

If you don't see output, try:

1. Remove `-nographic` and let QEMU open a graphical window:
   ```bash
   qemu-system-x86_64 -drive format=raw,file=target/boot.img -boot order=a -m 32
   ```

2. Or add serial output to the kernel and use `-serial stdio` instead of `-nographic`.

### KVM not available

**Symptoms:**
```
Could not access KVM kernel module: No such file or directory
```

**Fix:** This is a warning, not an error. QEMU falls back to TCG (software emulation) automatically. The kernel will still boot, just slower.

To enable KVM on Linux:
```bash
sudo apt-get install qemu-kvm
sudo usermod -aG kvm $USER
# Log out and back in
```

## Boot Image Issues

### Boot image not created

**Symptoms:**
```
ERROR: Boot image not found at target/boot.img
```

**Cause:** The build script skips boot image creation if NASM is not installed.

**Fix:**
```bash
# Install NASM
sudo apt-get install nasm    # Debian/Ubuntu
sudo dnf install nasm        # Fedora
brew install nasm            # macOS

# Rebuild
./build.sh
```

### Boot sector signature missing

**Check the signature:**
```bash
dd if=target/boot.img bs=1 count=2 skip=510 2>/dev/null | hexdump -e '1/1 "%02x"'
```

Should output `aa55`. If not, rebuild:
```bash
./build.sh
```

## Verification Issues

### verify.sh reports missing PVH note

**Check manually:**
```bash
readelf -n target/pvh-kernel
```

If no PVH note is listed, the `.note.pvh` section may have been stripped. Ensure the linker script includes `KEEP(*(.note.pvh))` and rebuild.

### verify.sh reports missing _start symbol

**Check:**
```bash
nm target/x86_64-unknown-none/release/kernel | grep _start
```

`target/pvh-kernel` is a raw binary (`OUTPUT_FORMAT(binary)`), so symbols are not preserved there. This is expected. Use marker-based checks in `./verify.sh` and boot tests (`./run.sh --boot-img`) instead.

### Kernel size seems wrong

**Normal size:** 4–12 KB for this minimal kernel. If it's much larger, check that unnecessary dependencies aren't being linked. If it's much smaller (under 1 KB), the build may have failed silently.

## Development Environment

### rust-analyzer shows errors in VS Code

rust-analyzer may not understand the `no_std` context. Add this to `.vscode/settings.json`:
```json
{
  "rust-analyzer.checkOnSave.extraArgs": ["--target", "x86_64-unknown-none"]
}
```

### Running tests

This kernel doesn't support `cargo test` — there's no test harness in a bare-metal `no_std` environment. Testing is done by:

1. **Build verification:** `./verify.sh`
2. **Boot testing:** `./test.sh` or `./run.sh --boot-img`
3. **Manual inspection:** `readelf`, `objdump`, `hexdump` on the kernel binary

## Quick Diagnostic Checklist

```bash
# Rust toolchain
rustc --version                    # Should be nightly
rustup target list --installed     # Should include x86_64-unknown-none
rustup component list --installed  # Should include rust-src

# Build tools
which nasm                         # Needed for boot sector
which qemu-system-x86_64           # Needed for testing

# Build artifacts
ls -la target/pvh-kernel           # Kernel binary
ls -la target/boot.img             # Boot image (requires nasm)

# Quick test
./build.sh && ./test.sh
```
