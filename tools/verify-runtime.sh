#!/bin/bash
# HITO-4 runtime gate: boot the kernel in QEMU with an IDE FAT16 disk attached
# and assert the user-space storage server works end to end:
#   1. kernel grants the storage server the IDE register ports (TSS I/O grant)
#   2. the storage server speaks ATA PIO from ring 3 (polled, IRQ14/15 masked)
#   3. the FAT16 root directory is listed and README contents are printed
set -e
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "[1/3] building kernel"
./build.sh >/dev/null

echo "[1b/3] building hello user program for the disk image"
./userspace/build.sh hello release --out target/hello-tmp >/dev/null
cp target/hello-tmp/hello.elf target/hello.elf
rm -rf target/hello-tmp

echo "[2/3] creating FAT16 disk image"
python3 tools/make-fat16.py target/disk.img

echo "[3/3] QEMU boot with IDE disk"
OUT=target/qemu-verify.log
QEMU_TIMEOUT="${QEMU_TIMEOUT:-120}" scripts/qemu.sh \
    -kernel target/pvh-kernel -m 128M \
    -machine pc,accel=tcg -cpu qemu64 -nographic \
    -hda target/disk.img \
    >"$OUT" 2>&1 || true

echo "--- gate checks ---"
ok=1
for marker in \
    "[boot] user storage server ready" \
    "=== fs_server: FAT16 smoke test ===" \
    "README contents"; do
    if grep -qF "$marker" "$OUT"; then
        echo "PASS: $marker"
    else
        echo "FAIL: $marker"
        ok=0
    fi
done
if grep -q "VirtIO-blk initialization failed\|No block device readable\|not a FAT16 volume" "$OUT"; then
    echo "FAIL: storage server reported an error"
    ok=0
fi
[ "$ok" = 1 ] && echo "GATE HITO-4: PASS" || { echo "GATE HITO-4: FAIL"; tail -40 "$OUT"; exit 1; }
