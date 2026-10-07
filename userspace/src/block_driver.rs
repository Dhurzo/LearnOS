//! ISA-style IDE hard-disk driver used by the user-space storage server.
//!
//! LESSON (microkernel boundary): the kernel grants this process only the IDE
//! register ports (0x1F0..0x1F7) via its TSS I/O bitmap. This module owns the
//! block protocol: command registers, LBA geometry, and the data-port transfer.
//! The kernel never parses FAT or touches the disk on the server's behalf.
//!
//! The classic ATA task-file interface (as emulated by QEMU `pc` + `-hda`)
//! uses eight registers at 0x1F0..0x1F7 and transfers sector bytes through the
//! 16-bit data port with IN/OUT word operations. LBAs are 0-based (LBA28).

use core::sync::atomic::{compiler_fence, Ordering};

/// Size of one disk sector.
pub const SECTOR_SIZE: usize = 512;

/// User virtual address where the kernel maps the driver's contiguous DMA area.
/// Unused by the IDE transport but kept for the kernel's storage-config ABI.
pub const DMA_QUEUE_VADDR: usize = 0x3000_0000;

/// IDE data port (first of the eight granted registers).
pub const IDE_DATA_PORT: u16 = 0x1F0;
/// Number of IDE register ports granted to this process.
pub const IDE_IO_PORT_LEN: u16 = 8;

const CMD_READ: u8 = 0x20;
const CMD_WRITE: u8 = 0x30;

/// Register offsets relative to the granted base (base + offset = port).
mod reg {
    pub const DATA: u16 = 0; // 0x1F0 data (16-bit word I/O)
    pub const ERROR: u16 = 1; // 0x1F1 error (read) / features (write)
    pub const COUNT: u16 = 2; // 0x1F2 sector count
    pub const LBA_LOW: u16 = 3; // 0x1F3 LBA[7:0]
    pub const LBA_MID: u16 = 4; // 0x1F4 LBA[15:8]
    pub const LBA_HIGH: u16 = 5; // 0x1F5 LBA[23:16]
    pub const DRIVE: u16 = 6; // 0x1F6 0xE0 | LBA[27:24] (master, LBA mode)
    pub const STATUS: u16 = 7; // 0x1F7 status (read) / command (write)
}

mod status {
    pub const BSY: u8 = 0x80;
    pub const DRQ: u8 = 0x08;
    pub const ERR: u8 = 0x01;
}

/// Reason code of the most recent failed transfer: 1 = device never became
/// ready (timeout or floating bus), 2 = device reported an error.
static mut LAST_REASON: u8 = 0;
/// Sector count reported by the kernel's storage-config message (0 = unknown).
static mut SECTOR_COUNT: u64 = 0;
/// Set once from the kernel's boot-time storage-resource IPC message.
static mut IO_BASE: u16 = 0;

/// Configure the device using kernel-granted resources. `io_base` is the first
/// granted IDE register port; `queue_phys` is unused by this transport.
pub unsafe fn init(io_base: u16, _queue_phys: u64) -> bool {
    if io_base == 0 {
        return false;
    }
    IO_BASE = io_base;
    // A missing device leaves the bus floating (0xFF). Wait briefly for BSY
    // to clear; QEMU's emulated disk is ready almost immediately.
    if !wait_not_busy() {
        LAST_REASON = 1;
        IO_BASE = 0;
        return false;
    }
    if inb(io_base + reg::STATUS) == 0xFF {
        LAST_REASON = 1;
        IO_BASE = 0;
        return false;
    }
    LAST_REASON = 0;
    true
}

/// Reason code of the most recent failed transfer.
pub unsafe fn last_reason() -> u8 {
    unsafe { LAST_REASON }
}

/// Sector count from the kernel storage config (0 if never provided).
pub unsafe fn sector_count() -> u64 {
    unsafe { SECTOR_COUNT }
}

/// Read one 512-byte sector. Returns false if uninitialized, out of range, or
/// if the device rejects the command.
pub fn read_sector(sector: usize, out: &mut [u8; SECTOR_SIZE]) -> bool {
    unsafe {
        if IO_BASE == 0 {
            return false;
        }
        if SECTOR_COUNT != 0 && sector as u64 >= SECTOR_COUNT {
            return false;
        }
        transfer(sector, out.as_mut_ptr(), CMD_READ)
    }
}

/// Write one 512-byte sector.
pub fn write_sector(sector: usize, input: &[u8; SECTOR_SIZE]) -> bool {
    unsafe {
        if IO_BASE == 0 {
            return false;
        }
        if SECTOR_COUNT != 0 && sector as u64 >= SECTOR_COUNT {
            return false;
        }
        transfer(sector, input.as_ptr(), CMD_WRITE)
    }
}

/// Read arbitrary bytes, preserving the sector boundary and device capacity.
pub fn read_bytes(offset: usize, out: &mut [u8]) -> usize {
    let mut total = 0usize;
    while total < out.len() {
        let byte_offset = match offset.checked_add(total) {
            Some(value) => value,
            None => break,
        };
        let sector = byte_offset / SECTOR_SIZE;
        let in_sector = byte_offset % SECTOR_SIZE;
        let amount = usize_min(SECTOR_SIZE - in_sector, out.len() - total);
        let mut scratch = [0u8; SECTOR_SIZE];
        if !read_sector(sector, &mut scratch) {
            break;
        }
        unsafe { copy_bytes(&scratch[in_sector], out.as_mut_ptr().add(total), amount); }
        total += amount;
    }
    total
}

/// Write arbitrary bytes with read-modify-write on partial sectors so adjacent
/// file-system metadata is preserved.
pub fn write_bytes(offset: usize, input: &[u8]) -> usize {
    let mut total = 0usize;
    while total < input.len() {
        let byte_offset = match offset.checked_add(total) {
            Some(value) => value,
            None => break,
        };
        let sector = byte_offset / SECTOR_SIZE;
        let in_sector = byte_offset % SECTOR_SIZE;
        let amount = usize_min(SECTOR_SIZE - in_sector, input.len() - total);
        let mut scratch = [0u8; SECTOR_SIZE];
        if in_sector != 0 || amount != SECTOR_SIZE {
            if !read_sector(sector, &mut scratch) {
                break;
            }
            unsafe { copy_bytes(input.as_ptr().add(total), scratch.as_mut_ptr().add(in_sector), amount); }
        } else {
            unsafe { copy_bytes(input.as_ptr().add(total), scratch.as_mut_ptr(), amount); }
        }
        if !write_sector(sector, &scratch) {
            break;
        }
        total += amount;
    }
    total
}

/// Issue one ATA PIO command and stream the buffer through the 16-bit data
/// port. `sector` is a 0-based LBA28 sector number.
unsafe fn transfer(sector: usize, buffer: *const u8, command: u8) -> bool {
    let base = IO_BASE;
    let lba = sector as u32;
    if lba > 0x0FFF_FFFF {
        LAST_REASON = 2; // outside LBA28 range
        return false;
    }
    if !wait_not_busy() {
        LAST_REASON = 1;
        return false;
    }
    // Select master drive in LBA mode, then give the device ~400ns to latch
    // the head register before the remaining task-file bytes go out.
    outb(base + reg::DRIVE, 0xE0 | ((lba >> 24) & 0x0F) as u8);
    for _ in 0..4 {
        let _ = inb(base + reg::STATUS);
    }
    outb(base + reg::COUNT, 1);
    outb(base + reg::LBA_LOW, (lba & 0xFF) as u8);
    outb(base + reg::LBA_MID, ((lba >> 8) & 0xFF) as u8);
    outb(base + reg::LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
    outb(base + reg::STATUS, command);
    compiler_fence(Ordering::SeqCst);

    if command == CMD_READ {
        if !wait_data_ready() {
            LAST_REASON = 1;
            return false;
        }
        for i in 0..(SECTOR_SIZE / 2) {
            store_u16(buffer as usize + i * 2, inw(base + reg::DATA));
        }
        compiler_fence(Ordering::SeqCst);
        if !wait_not_busy() {
            LAST_REASON = 1;
            return false;
        }
        if inb(base + reg::STATUS) & status::ERR != 0 {
            LAST_REASON = 2;
            return false;
        }
    } else {
        if !wait_data_ready() {
            LAST_REASON = 1;
            return false;
        }
        for i in 0..(SECTOR_SIZE / 2) {
            outw(base + reg::DATA, load_u16(buffer as usize + i * 2));
        }
        compiler_fence(Ordering::SeqCst);
        if !wait_not_busy() {
            LAST_REASON = 1;
            return false;
        }
        if inb(base + reg::STATUS) & status::ERR != 0 {
            LAST_REASON = 2;
            return false;
        }
    }
    LAST_REASON = 0;
    true
}

/// Spin until BSY clears (bounded so a missing device cannot hang us).
unsafe fn wait_not_busy() -> bool {
    for _ in 0..1_000_000u32 {
        let s = inb(IO_BASE + reg::STATUS);
        if s != 0xFF && s & status::BSY == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Spin until the device requests the data transfer (DRQ with BSY clear).
unsafe fn wait_data_ready() -> bool {
    for _ in 0..1_000_000u32 {
        let s = inb(IO_BASE + reg::STATUS);
        if s & status::BSY == 0 {
            if s & status::ERR != 0 {
                return false;
            }
            if s & status::DRQ != 0 {
                return true;
            }
        }
        core::hint::spin_loop();
    }
    false
}

#[inline]
fn usize_min(a: usize, b: usize) -> usize {
    if a < b { a } else { b }
}

#[inline]
unsafe fn copy_bytes(src: *const u8, dst: *mut u8, n: usize) {
    core::ptr::copy_nonoverlapping(src, dst, n);
}

#[inline]
unsafe fn store_u16(addr: usize, value: u16) {
    core::ptr::write_unaligned(addr as *mut u16, value);
}

#[inline]
unsafe fn load_u16(addr: usize) -> u16 {
    core::ptr::read_unaligned(addr as *const u16)
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value, options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!("in ax, dx", in("dx") port, out("ax") value, options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value, options(nostack, preserves_flags));
}
