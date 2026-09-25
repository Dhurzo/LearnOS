//! Legacy virtio-blk driver used by the user-space storage server.
//!
//! LESSON (microkernel boundary): the kernel discovers the PCI function, grants
//! this process only its BAR0 I/O ports, and maps a pinned contiguous DMA queue.
//! This module owns the device protocol, block requests, and sector API; the
//! kernel does not parse FAT or perform disk reads/writes on the server's behalf.
//!
//! This educational driver intentionally uses one synchronous request at a
//! time. That keeps descriptor ownership easy to follow while still exercising
//! real DMA, legacy virtio ring layout, and device completion.

use core::sync::atomic::{compiler_fence, Ordering};

/// Size of one disk sector.
pub const SECTOR_SIZE: usize = 512;

/// User virtual address where the kernel maps the driver's contiguous DMA area.
pub const DMA_QUEUE_VADDR: usize = 0x3000_0000;
/// Maximum legacy queue size supported by the reference QEMU configuration.
pub const MAX_QUEUE_SIZE: usize = 256;
/// Four pages cover a 256-entry descriptor/available/used ring plus scratch
/// request, status, and 512-byte bounce-buffer storage.
pub const DMA_QUEUE_PAGES: usize = 4;

const REQ_IN: u32 = 0;
const REQ_OUT: u32 = 1;
const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;

mod reg {
    pub const GUEST_FEATURES: u16 = 0x04;
    pub const QUEUE_PFN: u16 = 0x08;
    pub const QUEUE_NUM: u16 = 0x0C;
    pub const QUEUE_SEL: u16 = 0x0E;
    pub const QUEUE_NOTIFY: u16 = 0x10;
    pub const STATUS: u16 = 0x12;
    pub const DEVICE_CONFIG: u16 = 0x14;
}

mod status {
    pub const ACKNOWLEDGE: u8 = 1;
    pub const DRIVER: u8 = 2;
    pub const DRIVER_OK: u8 = 4;
    pub const FEATURES_OK: u8 = 8;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
struct BlockRequest {
    request_type: u32,
    reserved: u32,
    sector: u64,
}

#[derive(Clone, Copy)]
struct QueueLayout {
    avail_off: usize,
    avail_ring_off: usize,
    used_off: usize,
    request_off: usize,
    status_off: usize,
    data_off: usize,
}

/// Compute the legacy ring offsets from the device's actual queue size.
///
/// The used ring starts on a 4 KiB boundary; scratch request data lives after
/// the full used ring, not over unused-looking slots the device may still DMA.
fn queue_layout(queue_size: usize) -> Option<QueueLayout> {
    if !(3..=MAX_QUEUE_SIZE).contains(&queue_size) {
        return None;
    }
    let avail_off = queue_size.checked_mul(core::mem::size_of::<Descriptor>())?;
    let avail_ring_off = avail_off.checked_add(4)?; // flags + idx
    let avail_end = avail_ring_off.checked_add(queue_size.checked_mul(2)?)?;
    let used_off = avail_end.checked_add(4095)? & !4095;
    let used_end = used_off.checked_add(4)?.checked_add(queue_size.checked_mul(8)?)?;
    let scratch_off = used_end.checked_add(4095)? & !4095;
    let request_off = scratch_off;
    let status_off = request_off.checked_add(core::mem::size_of::<BlockRequest>())?;
    let data_off = status_off.checked_add(1)?;
    if data_off.checked_add(SECTOR_SIZE)? > DMA_QUEUE_PAGES * 4096 {
        return None;
    }
    Some(QueueLayout {
        avail_off,
        avail_ring_off,
        used_off,
        request_off,
        status_off,
        data_off,
    })
}

/// Set once from the kernel's boot-time storage-resource IPC message. The
/// storage-server process is single-threaded and issues synchronous requests.
static mut IO_BASE: u16 = 0;
static mut QUEUE_PHYS: u64 = 0;
static mut SECTOR_COUNT: u64 = 0;
static mut QUEUE_SIZE: usize = 0;
static mut AVAIL_INDEX: u16 = 0;
static mut LAST_USED_INDEX: u16 = 0;

/// Configure the legacy device and queue using kernel-granted resources.
///
/// `io_base` is a BAR0 base, not a PCI config address. `queue_phys` names the
/// first of four contiguous pages, mapped for this process at
/// `DMA_QUEUE_VADDR`. The hardware sees physical addresses from descriptors;
/// the process writes the same memory through its user mapping.
pub unsafe fn init(io_base: u16, queue_phys: u64) -> bool {
    if io_base == 0 || queue_phys == 0 || queue_phys & 0xFFF != 0 {
        return false;
    }

    outb(io_base + reg::STATUS, 0); // reset
    outb(io_base + reg::STATUS, status::ACKNOWLEDGE | status::DRIVER);

    // Negotiate no optional features: basic 512-byte read/write requests only.
    outl(io_base + reg::GUEST_FEATURES, 0);
    outb(
        io_base + reg::STATUS,
        status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK,
    );
    if inb(io_base + reg::STATUS) & status::FEATURES_OK == 0 {
        return false;
    }

    outw(io_base + reg::QUEUE_SEL, 0);
    let queue_size = inw(io_base + reg::QUEUE_NUM) as usize;
    if queue_size < 3 || queue_size > MAX_QUEUE_SIZE {
        return false;
    }

    // Legacy capacity is a little-endian u64 at the device config area.
    let sectors = inl(io_base + reg::DEVICE_CONFIG) as u64
        | ((inl(io_base + reg::DEVICE_CONFIG + 4) as u64) << 32);
    if sectors == 0 {
        return false;
    }

    // The driver only uses descriptor heads 0..2; the full ring is nevertheless
    // laid out according to the device's actual queue size (not an assumed size).
    outl(io_base + reg::QUEUE_PFN, (queue_phys >> 12) as u32);
    outb(
        io_base + reg::STATUS,
        status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK | status::DRIVER_OK,
    );

    IO_BASE = io_base;
    QUEUE_PHYS = queue_phys;
    SECTOR_COUNT = sectors;
    QUEUE_SIZE = queue_size;
    AVAIL_INDEX = 0;
    LAST_USED_INDEX = 0;
    true
}

/// Read one 512-byte sector. Returns false if uninitialized, out of range, or
/// if the device reports an error/does not complete in time.
pub fn read_sector(sector: usize, out: &mut [u8; SECTOR_SIZE]) -> bool {
    unsafe {
        if IO_BASE == 0 || sector as u64 >= SECTOR_COUNT {
            return false;
        }
        transfer(sector, out.as_mut_ptr(), REQ_IN)
    }
}

/// Write one 512-byte sector.
pub fn write_sector(sector: usize, input: &[u8; SECTOR_SIZE]) -> bool {
    unsafe {
        if IO_BASE == 0 || sector as u64 >= SECTOR_COUNT {
            return false;
        }
        transfer(sector, input.as_ptr(), REQ_OUT)
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
        let amount = core::cmp::min(SECTOR_SIZE - in_sector, out.len() - total);
        let mut scratch = [0u8; SECTOR_SIZE];
        if !read_sector(sector, &mut scratch) {
            break;
        }
        out[total..total + amount].copy_from_slice(&scratch[in_sector..in_sector + amount]);
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
        let amount = core::cmp::min(SECTOR_SIZE - in_sector, input.len() - total);
        let mut scratch = [0u8; SECTOR_SIZE];
        if (in_sector != 0 || amount != SECTOR_SIZE) && !read_sector(sector, &mut scratch) {
            break;
        }
        scratch[in_sector..in_sector + amount].copy_from_slice(&input[total..total + amount]);
        if !write_sector(sector, &scratch) {
            break;
        }
        total += amount;
    }
    total
}

unsafe fn transfer(sector: usize, caller_data: *const u8, request_type: u32) -> bool {
    let base = IO_BASE;
    let queue_va = DMA_QUEUE_VADDR;
    let queue_pa = QUEUE_PHYS as usize;
    let qsize = QUEUE_SIZE;

    let layout = match queue_layout(qsize) {
        Some(layout) => layout,
        None => return false,
    };
    let desc_off = 0usize;

    let request = BlockRequest {
        request_type,
        reserved: 0,
        sector: sector as u64,
    };
    core::ptr::write_unaligned((queue_va + layout.request_off) as *mut BlockRequest, request);
    core::ptr::write_unaligned((queue_va + layout.status_off) as *mut u8, 0xFF);

    if request_type == REQ_OUT {
        core::ptr::copy_nonoverlapping(caller_data as *const u8, (queue_va + layout.data_off) as *mut u8, SECTOR_SIZE);
    }

    let descriptors = [
        Descriptor {
            addr: (queue_pa + layout.request_off) as u64,
            len: core::mem::size_of::<BlockRequest>() as u32,
            flags: DESC_F_NEXT,
            next: 1,
        },
        Descriptor {
            addr: (queue_pa + layout.data_off) as u64,
            len: SECTOR_SIZE as u32,
            flags: DESC_F_NEXT | if request_type == REQ_IN { DESC_F_WRITE } else { 0 },
            next: 2,
        },
        Descriptor {
            addr: (queue_pa + layout.status_off) as u64,
            len: 1,
            flags: DESC_F_WRITE,
            next: 0,
        },
    ];
    for (index, descriptor) in descriptors.iter().enumerate() {
        core::ptr::write_unaligned(
            (queue_va + desc_off + index * core::mem::size_of::<Descriptor>()) as *mut Descriptor,
            *descriptor,
        );
    }

    let slot = (AVAIL_INDEX as usize) % qsize;
    core::ptr::write_unaligned((queue_va + layout.avail_ring_off + slot * 2) as *mut u16, 0);
    compiler_fence(Ordering::SeqCst);
    AVAIL_INDEX = AVAIL_INDEX.wrapping_add(1);
    core::ptr::write_unaligned((queue_va + layout.avail_off + 2) as *mut u16, AVAIL_INDEX);

    let before = LAST_USED_INDEX;
    outw(base + reg::QUEUE_NOTIFY, 0);
    let mut completed = false;
    for _ in 0..1_000_000u32 {
        let now = core::ptr::read_unaligned((queue_va + layout.used_off + 2) as *const u16);
        if now != before {
            LAST_USED_INDEX = now;
            completed = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !completed || core::ptr::read_unaligned((queue_va + layout.status_off) as *const u8) != 0 {
        return false;
    }
    compiler_fence(Ordering::SeqCst);
    if request_type == REQ_IN {
        core::ptr::copy_nonoverlapping(
            (queue_va + layout.data_off) as *const u8,
            caller_data as *mut u8,
            SECTOR_SIZE,
        );
    }
    true
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

#[inline]
unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    core::arch::asm!("in eax, dx", in("dx") port, out("eax") value, options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outl(port: u16, value: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nostack, preserves_flags));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn largest_supported_legacy_queue_fits_dma_allocation() {
        let layout = queue_layout(MAX_QUEUE_SIZE).expect("reference queue fits");
        assert_eq!(layout.used_off, 0x2000);
        assert!(layout.data_off + SECTOR_SIZE <= DMA_QUEUE_PAGES * 4096);
    }

    #[test]
    fn queue_layout_rejects_unsupported_sizes() {
        assert!(queue_layout(0).is_none());
        assert!(queue_layout(2).is_none());
        assert!(queue_layout(MAX_QUEUE_SIZE + 1).is_none());
    }
}
