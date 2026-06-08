//! Block Device Abstraction (Phase 3 — Block Driver)
//!
//! Provides a uniform interface for reading blocks from storage.
//! The current implementation uses a kernel-resident RAM disk backed
//! by static data.  A mature system would replace this with a user-space
//! virtio-blk driver that communicates via IPC.
//!
//! Block size is fixed at 512 bytes (standard sector size).

/// Block size in bytes.
pub const BLOCK_SIZE: usize = 512;

/// Total number of blocks in the RAM disk.
pub const NUM_BLOCKS: usize = 256; // 128 KiB

/// The RAM disk backing store.
static mut RAMDISK: [u8; BLOCK_SIZE * NUM_BLOCKS] = [0u8; BLOCK_SIZE * NUM_BLOCKS];

/// Initialize the RAM disk from the built-in file data.
///
/// Called once during boot.  Copies all embedded file data into the
/// RAM disk so that block-level reads return the file contents.
pub fn init() {
    unsafe {
        // Copy built-in file data into the RAM disk.
        // In a full implementation we would walk the FS directory table
        // and read a superblock; for now we just pre-populate so that
        // block-level reads work.
        let hello = b"Hello from the LearnOS filesystem!\nThis file is embedded in the kernel.\n";
        let readme = b"LearnOS - a minimal microkernel for learning.\n\nAvailable commands:\n  help  - show this message\n  read  - read a file by name\n  exec  - exec a built-in program\n  clear - clear the screen\n";
        let boot = b"[boot]\nkernel=learnos.bin\ninitrd=init.tar\nroot=/dev/ram0\n";

        let mut pos: usize = 0;
        // Write hello.txt starting at block 0
        for (j, &byte) in hello.iter().enumerate() {
            if pos + j < RAMDISK.len() {
                RAMDISK[pos + j] = byte;
            }
        }
        pos += hello.len();
        // Write README
        for (j, &byte) in readme.iter().enumerate() {
            if pos + j < RAMDISK.len() {
                RAMDISK[pos + j] = byte;
            }
        }
        pos += readme.len();
        // Write boot.cfg
        for (j, &byte) in boot.iter().enumerate() {
            if pos + j < RAMDISK.len() {
                RAMDISK[pos + j] = byte;
            }
        }
    }
}

/// Read one block from the RAM disk into a buffer.
///
/// `block` — block number (0-indexed).
/// `buf`   — buffer of exactly BLOCK_SIZE bytes.
///
/// Returns `true` on success, `false` if `block` is out of range.
pub fn read_block(block: usize, buf: &mut [u8; BLOCK_SIZE]) -> bool {
    if block >= NUM_BLOCKS {
        return false;
    }
    if buf.len() != BLOCK_SIZE {
        return false;
    }
    unsafe {
        let start = block * BLOCK_SIZE;
        buf.copy_from_slice(&RAMDISK[start..start + BLOCK_SIZE]);
    }
    true
}

/// Write one block to the RAM disk.
///
/// Returns `true` on success.
pub fn write_block(block: usize, buf: &[u8; BLOCK_SIZE]) -> bool {
    if block >= NUM_BLOCKS {
        return false;
    }
    unsafe {
        let start = block * BLOCK_SIZE;
        RAMDISK[start..start + BLOCK_SIZE].copy_from_slice(buf);
    }
    true
}

/// Read a range of bytes from the RAM disk at a given byte offset.
///
/// This is a convenience wrapper that reads across block boundaries.
/// Returns the number of bytes actually read.
pub fn read_bytes(offset: usize, buf: &mut [u8]) -> usize {
    let end = (offset + buf.len()).min(BLOCK_SIZE * NUM_BLOCKS);
    let actual = end - offset;
    if actual == 0 {
        return 0;
    }
    unsafe {
        let src = &RAMDISK[offset..offset + actual];
        buf[..actual].copy_from_slice(src);
    }
    actual
}
