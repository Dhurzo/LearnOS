# Troubleshooting

This guide targets the current runtime path: **PVH + QEMU `-kernel` + VGA + PS/2 polling**.

## Build Problems

### Nightly / target errors

Symptoms:

- `-Z flag is only accepted on the nightly compiler`
- `x86_64-unknown-none not found`

Fix:

```bash
rustup install nightly
rustup target add x86_64-unknown-none --toolchain nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
```

### Linker script errors

Symptoms:

- linker fails with section/symbol placement errors

Fix:

- use `./build.sh` so `RUSTFLAGS` are applied consistently
- verify `kernel/src/linker.ld` exists and is not stale

## Runtime Problems

### QEMU window stays black / only blinking cursor

Common causes:

1. Kernel never reaches 64-bit Rust entry due to bootstrap mismatch
2. VGA text writes happen but display mode/focus is wrong
3. You are typing in the terminal instead of QEMU window

Checks:

```bash
./build.sh
./run.sh --interactive
```

Then:

- click inside QEMU window before typing
- use `--debug` when needed:

```bash
./run.sh --debug --timeout 5
```

If debug shows only firmware text and no kernel progression, inspect bootstrap and PVH note:

```bash
./verify.sh
readelf -n target/pvh-kernel
```

### "Booting from ROM.." and nothing else

This line by itself is not failure; it is firmware startup output.

If it never transitions to visible kernel text, validate:

1. Xen note present (`readelf -n target/pvh-kernel` shows owner `Xen`)
2. ELF type is executable (`ET_EXEC`)
3. bootstrap path in `kernel/src/boot.S` still calls Rust `_start`

### Keyboard does not echo

Checklist:

- ensure QEMU window has focus
- avoid `-nographic` for VGA interaction
- verify keyboard loop is active in `kernel/src/main.rs`
- confirm scan-code port polling in `kernel/src/keyboard.rs`

## Tooling Problems

### `qemu-system-x86_64: command not found`

Install QEMU:

```bash
sudo apt install qemu-system-x86
```

### KVM unavailable warnings

Not fatal. QEMU falls back to TCG.

For hardware acceleration on Linux:

```bash
sudo apt install qemu-kvm
sudo usermod -aG kvm $USER
```

Re-login afterward.

## Verification Commands

```bash
./verify.sh
./test.sh --timeout 5
```

Manual inspection:

```bash
readelf -h target/pvh-kernel
readelf -n target/pvh-kernel
nm -n target/pvh-kernel | rg "pvh_start|_start"
```

## Quick Diagnostic Script

```bash
rustc --version
cargo --version
qemu-system-x86_64 --version

./build.sh
./verify.sh
./run.sh --debug --timeout 5
```

If the last command still fails to show kernel progress, collect that output and inspect `kernel/src/boot.S` + `kernel/src/linker.ld` first.
