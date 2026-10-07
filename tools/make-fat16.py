#!/usr/bin/env python3
# Build a minimal FAT16 raw image for the LearnOS user-space storage server.
#
# Layout follows the FAT16 SPEC (same geometry the userspace fs.rs parser and
# its honest fixtures encode):
#   [ boot sector | FAT #1 | FAT #2 | root dir | data clusters ]
#
# Usage: ./tools/make-fat16.py [out-path]   (default: target/disk.img)

import sys

out_path = sys.argv[1] if len(sys.argv) > 1 else "target/disk.img"

BPS = 512
SPC = 1
RESERVED = 1
NUM_FATS = 2
ROOT_ENTRIES = 16
FAT_SECTORS = 1

root_dir_sector = RESERVED + NUM_FATS * FAT_SECTORS  # 3
root_dir_sectors = (ROOT_ENTRIES * 32 + BPS - 1) // BPS
data_start = root_dir_sector + root_dir_sectors      # 4

files = [
    # NOTE: the 8.3 base field is EXACTLY 8 bytes. Longer Python literals
    # silently INSERT bytes on slice assignment (`img[off:off+8] = base`),
    # shifting every later sector and corrupting file contents (this exact
    # bug once hid behind substring-matching tests while breaking `run`).
    (b"README  ", b"   ",  # 8.3 name (6+2) + ext padding
     b"LearnOS FAT16 image.\nThis README is read by the user-space storage server\n"
     b"through the kernel-granted IDE register ports (ATA PIO from ring 3).\n"),
    (b"HELLO   ", b"TXT", b"Hello from FAT16!\n"),  # 5+3
]

# A runnable user program for the shell's `run` command (C1 `sys_exec`).
# verify-runtime.sh builds it first; the image still works without it.
import os
hello_path = os.path.join(os.path.dirname(out_path), "hello.elf")
if os.path.exists(hello_path):
    with open(hello_path, "rb") as f:
        hello_bytes = f.read()
    files.append((b"RUNME   ", b"   ", hello_bytes))  # 5+3

clusters_needed = sum((len(c) + BPS - 1) // BPS for _, _, c in files)
total_sectors = max(data_start + clusters_needed + 8, 64)  # spare space, sector-aligned

img = bytearray(total_sectors * BPS)

def w16(off, v):
    img[off] = v & 0xFF
    img[off + 1] = (v >> 8) & 0xFF

def w32(off, v):
    img[off:off + 4] = v.to_bytes(4, "little")

# --- Boot sector / BPB ---
w16(0x0B, BPS)
img[0x0D] = SPC
w16(0x0E, RESERVED)
img[0x10] = NUM_FATS
w16(0x11, ROOT_ENTRIES)
img[0x15] = 0xF8
w16(0x16, FAT_SECTORS)
w32(0x20, total_sectors)
img[510] = 0x55
img[511] = 0xAA

# --- FAT #1 (FAT #2 is a copy) ---
def fat_off(cluster):
    return RESERVED * BPS + cluster * 2

w16(RESERVED * BPS + 0, 0xFFF8)  # media descriptor
w16(RESERVED * BPS + 2, 0xFFFF)  # reserved entry 1

cluster = 2
entries = []
for base, ext, content in files:
    n = (len(content) + BPS - 1) // BPS
    first = cluster
    size = len(content)
    for i in range(n):
        nxt = 0xFFF8 if i == n - 1 else cluster + 1
        w16(fat_off(cluster), nxt)
        sector = data_start + (cluster - 2)
        chunk = content[:BPS]
        img[sector * BPS:sector * BPS + len(chunk)] = chunk
        content = content[BPS:]
        cluster += 1
    entries.append((base, ext, first, size))

# FAT #2 mirrors FAT #1
fat1 = bytes(img[RESERVED * BPS:RESERVED * BPS + FAT_SECTORS * BPS])
fat2_off = (RESERVED + FAT_SECTORS) * BPS
img[fat2_off:fat2_off + FAT_SECTORS * BPS] = fat1

# --- Root directory entries ---
rd = root_dir_sector * BPS
for i, (base, ext, first, size) in enumerate(entries):
    assert len(base) == 8 and len(ext) == 3, "8.3 fields must be exactly 8+3 bytes"
    off = rd + i * 32
    img[off:off + 8] = base
    img[off + 8:off + 11] = ext
    img[off + 0x0A] = 0x20  # archive attribute
    img[off + 0x1A] = first & 0xFF
    img[off + 0x1B] = (first >> 8) & 0xFF
    img[off + 0x1C:off + 0x20] = size.to_bytes(4, "little")

with open(out_path, "wb") as f:
    f.write(img)

print(f"wrote {out_path}: {total_sectors} sectors, {len(entries)} files")
