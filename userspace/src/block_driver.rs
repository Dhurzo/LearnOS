//! block_driver — Phase 6 storage crate (user space).
//!
//! A *thin* wrapper over the kernel's raw-block syscalls (`BLOCK_READ` nr 24,
//! `BLOCK_WRITE` nr 25). The kernel owns the virtio-blk device and performs the
//! actual I/O; this module is the only user-space code that touches storage.
//! All higher-level filesystem logic lives in the sibling `fs` crate.
//!
//! The device is addressed as a flat array of 512-byte sectors. This module
//! exposes per-sector access plus a convenience for reading/writing arbitrary
//! byte ranges that may span sector boundaries.

use crate::syscall_nr::{BLOCK_READ, BLOCK_WRITE};
use crate::syscall::syscall3;

/// Size of one storage sector (matches `kernel/src/block_dev.rs::BLOCK_SIZE`).
pub const SECTOR_SIZE: usize = 512;

/// Read a single 512-byte sector into `buf`.
///
/// Returns `true` if the full sector was read, `false` on any failure.
#[inline]
pub unsafe fn read_sector(sector: usize, buf: &mut [u8; SECTOR_SIZE]) -> bool {
    let offset = sector * SECTOR_SIZE;
    let ret = syscall3(BLOCK_READ, offset, buf.as_mut_ptr() as usize, SECTOR_SIZE);
    ret as usize == SECTOR_SIZE
}

/// Write a single 512-byte sector from `buf`.
///
/// Returns `true` if the full sector was written, `false` on any failure.
#[inline]
pub unsafe fn write_sector(sector: usize, buf: &[u8; SECTOR_SIZE]) -> bool {
    let offset = sector * SECTOR_SIZE;
    let ret = syscall3(BLOCK_WRITE, offset, buf.as_ptr() as usize, SECTOR_SIZE);
    ret as usize == SECTOR_SIZE
}

/// Read `count` bytes starting at byte `offset` into `buf`.
///
/// Transparently crosses sector boundaries by issuing one block read per
/// affected sector. Returns the number of bytes actually read (0 on failure).
pub unsafe fn read_bytes(offset: usize, buf: &mut [u8]) -> usize {
    let mut total = 0usize;
    while total < buf.len() {
        let byte_off = offset + total;
        let sector = byte_off / SECTOR_SIZE;
        let in_sector = byte_off % SECTOR_SIZE;
        let want = core::cmp::min(SECTOR_SIZE - in_sector, buf.len() - total);

        let mut sec = [0u8; SECTOR_SIZE];
        if !read_sector(sector, &mut sec) {
            break;
        }
        buf[total..total + want].copy_from_slice(&sec[in_sector..in_sector + want]);
        total += want;
    }
    total
}

/// Write `count` bytes from `buf` starting at byte `offset`.
///
/// Writes whole sectors only (the device is sector-addressed); the final partial
/// sector is padded with zeros. Returns the number of bytes written.
pub unsafe fn write_bytes(offset: usize, buf: &[u8]) -> usize {
    let mut total = 0usize;
    while total < buf.len() {
        let byte_off = offset + total;
        let sector = byte_off / SECTOR_SIZE;
        let in_sector = byte_off % SECTOR_SIZE;
        let want = core::cmp::min(SECTOR_SIZE - in_sector, buf.len() - total);

        let mut sec = [0u8; SECTOR_SIZE];
        // Preserve any bytes already in the target sector (e.g. a prior partial
        // write) so we never zero out neighbouring data.
        read_sector(sector, &mut sec);
        sec[in_sector..in_sector + want].copy_from_slice(&buf[total..total + want]);
        if !write_sector(sector, &sec) {
            break;
        }
        total += want;
    }
    total
}

/// Read a whole file (given its byte length) into `out`, returning the bytes read.
///
/// This is the helper the `fs` crate uses to fetch file contents after it has
/// located them in the FAT chain.
#[inline]
pub unsafe fn read_file(offset: usize, out: &mut [u8]) -> usize {
    read_bytes(offset, out)
}
