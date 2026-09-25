//! Block Device Abstraction (Phase 3 → Phase 6)
//!
//! Provides a uniform interface for reading/writing 512-byte storage blocks.
//!
//! Two backends are supported, selected at boot:
//!   1. **virtio-blk (0.95 legacy)** — the real device on QEMU `virt`. Driven
//!      entirely in kernel space because SeaBIOS presents virtio-blk as a
//!      *legacy PCI* device whose BAR0 is an I/O region, which cannot be mapped
//!      into user address space (see DECISIONS.md [Phase 6]). The kernel does the
//!      raw block transfer; all filesystem logic lives in user space and reaches
//!      storage through `BLOCK_READ`/`BLOCK_WRITE` syscalls.
//!   2. **RAM disk** — a static backing store used as a fallback when no
//!      virtio-blk device is present (or its init fails), so the kernel always
//!      has *some* block source and never dead-locks at boot.
//!
//! `read_block` / `write_block` transparently dispatch to whichever backend was
//! initialised first.

/// Block size in bytes (standard 512-byte sector).
pub const BLOCK_SIZE: usize = 512;

// =============================================================================
// BACKEND SELECTION
// =============================================================================

enum Backend {
    Virtio,
    Ram,
}

static mut BACKEND: Option<Backend> = None;

// =============================================================================
// RAM DISK (fallback)
// =============================================================================

/// Total number of blocks in the RAM disk.
pub const NUM_BLOCKS: usize = 256; // 128 KiB

/// The RAM disk backing store.
static mut RAMDISK: [u8; BLOCK_SIZE * NUM_BLOCKS] = [0u8; BLOCK_SIZE * NUM_BLOCKS];

/// Initialise the RAM disk from the built-in file data.
pub fn init_ram_disk() {
    unsafe {
        let hello = b"Hello from the LearnOS filesystem!\nThis file is embedded in the kernel.\n";
        let readme = b"LearnOS - a minimal microkernel for learning.\n\nAvailable commands:\n  help  - show this message\n  read  - read a file by name\n  exec  - exec a built-in program\n  clear - clear the screen\n";
        let boot = b"[boot]\nkernel=learnos.bin\ninitrd=init.tar\nroot=/dev/ram0\n";

        let mut pos: usize = 0;
        for data in [hello.as_slice(), readme.as_slice(), boot.as_slice()] {
            for &byte in data.iter() {
                if pos < RAMDISK.len() {
                    RAMDISK[pos] = byte;
                    pos += 1;
                }
            }
        }
    }
}

/// Read one block from the RAM disk into a buffer.
pub fn ram_read_block(block: usize, buf: &mut [u8; BLOCK_SIZE]) -> bool {
    if block >= NUM_BLOCKS || buf.len() != BLOCK_SIZE {
        return false;
    }
    unsafe {
        let start = block * BLOCK_SIZE;
        buf.copy_from_slice(&RAMDISK[start..start + BLOCK_SIZE]);
    }
    true
}

/// Write one block to the RAM disk.
pub fn ram_write_block(block: usize, buf: &[u8; BLOCK_SIZE]) -> bool {
    if block >= NUM_BLOCKS {
        return false;
    }
    unsafe {
        let start = block * BLOCK_SIZE;
        RAMDISK[start..start + BLOCK_SIZE].copy_from_slice(buf);
    }
    true
}

// =============================================================================
// PCI ENUMERATION (needed to locate the legacy virtio-blk device)
// =============================================================================

// =============================================================================
// PCI CONFIGURATION SPACE (mechanism #1: I/O ports 0xCF8 / 0xCFC)
// =============================================================================
//
// LESSON: how the CPU talks to PCI devices on x86.
//
// A PCI device is addressed by a triple: (bus, device/slot, function). To read
// one of its 32-bit configuration registers the CPU performs two port-I/O
// steps:
//
//   1. Write a 32-bit ADDRESS word to I/O port 0xCF8.
//   2. Read the 32-bit DATA word back from I/O port 0xCFC.
//
// Address-word layout (PCI spec):
//
//   bit 31      = enable bit (must be 1, otherwise the access is ignored)
//   bits 30:24  = reserved (0)
//   bits 23:16  = bus number
//   bits 15:11  = device number ("slot", 0-31)
//   bits 10:8   = function number (0-7)
//   bits 7:2    = register offset, DWORD-ALIGNED (the low 2 bits are always 0)
//   bits 1:0    = 0
//
// ⚠️ BUG WE FIXED HERE (educational!): an earlier revision of this function
// used `out dx, al` (an 8-bit write!) with `addr as u8`. That sent only the
// LOW BYTE of the address — the register offset — while bus/device/function
// silently stayed zero. Worse, the bit shifts were wrong too
// (`bus << 24`, `slot << 19`, `reg << 16`). The result: every probe read
// device 0:0.0's register 0x00 instead of the device we asked for, so the
// virtio-blk card was never found. Port 0xCF8 needs the FULL 32-bit word via
// `out dx, eax`, with each field shifted to its spec position.
unsafe fn pci_config_read(bus: u8, device: u8, func: u8, offset: u8) -> u32 {
    let addr = (1u32 << 31)
        | ((bus as u32) << 16)
        | (((device & 0x1F) as u32) << 11)
        | (((func & 0x07) as u32) << 8)
        | ((offset & 0xFC) as u32); // mask to a dword boundary (bits 7:2)
    core::arch::asm!(
        "out dx, eax", // 32-bit OUT: the whole address word goes to 0xCF8
        in("dx") 0xCF8u16,
        in("eax") addr,
        options(nostack, preserves_flags),
    );
    let value: u32;
    core::arch::asm!(
        "in eax, dx", // 32-bit IN: the register contents come back from 0xCFC
        out("eax") value,
        in("dx") 0xCFCu16,
        options(nostack, preserves_flags),
    );
    value
}

/// PCI vendor ID shared by ALL VirtIO devices (assigned to the VirtIO project).
const VIRTIO_PCI_VENDOR_ID: u16 = 0x1AF4;

/// PCI device ID of the virtio-blk storage device ("transitional"/legacy ID).
const VIRTIO_BLK_DEVICE_ID: u16 = 0x1001;

// =============================================================================
// VIRTIO-BLK 0.95 LEGACY DRIVER
// =============================================================================

/// Request types (virtio-blk spec §2.1).
const REQ_IN: u32 = 0; // read from device
const REQ_OUT: u32 = 1; // write to device

/// Descriptor flags (virtio spec §1.1.3).
const DESC_F_NEXT: u16 = 0x01; // next descriptor follows in the chain
const DESC_F_WRITE: u16 = 0x02; // device writes this buffer (a read request)

/// Legacy virtio-pci I/O register map (offsets from the BAR0 base port).
///
/// LESSON: "legacy" (virtio 0.9.5, what SeaBIOS exposes) vs "modern"
/// (virtio 1.0+). They are DIFFERENT register layouts! Legacy devices are
/// driven with `in`/`out` port instructions on an I/O BAR; modern devices use
//  memory-mapped registers discovered via PCI capabilities. Mixing the two
/// maps is a classic bug — an earlier revision of this driver used modern-style
/// offsets (QUEUE_DESC_LO at 0x50, ...) on a legacy device and talked to
/// thin air. The true legacy map (virtio 0.9.5, appendix "PCI Device Discovery")
/// is a compact 20-byte header followed by device-specific config:
//
//   +0x00 u32  HOST_FEATURES   (device -> driver: what the device offers)
//   +0x04 u32  GUEST_FEATURES  (driver -> device: what we accept)
//   +0x08 u32  QUEUE_PFN       (driver -> device: page frame no. of the queue)
//   +0x0C u16  QUEUE_NUM       (device -> driver: max queue size, read-only)
//   +0x0E u16  QUEUE_SEL       (driver -> device: which queue we program)
//   +0x10 u16  QUEUE_NOTIFY    (driver -> device: "kick" the selected queue)
//   +0x12 u8   STATUS          (driver -> device: our lifecycle stage)
//   +0x13 u8   ISR             (device -> driver: interrupt reason, read-clears)
///   +0x14 ... device-specific config (for virtio-blk: capacity u64, ...).
mod legacy_reg {
    pub const HOST_FEATURES: u16 = 0x00;
    pub const GUEST_FEATURES: u16 = 0x04;
    pub const QUEUE_PFN: u16 = 0x08;
    pub const QUEUE_NUM: u16 = 0x0C;
    pub const QUEUE_SEL: u16 = 0x0E;
    pub const QUEUE_NOTIFY: u16 = 0x10;
    pub const STATUS: u16 = 0x12;
    pub const ISR: u16 = 0x13;
    /// Start of the device-specific config area (20-byte common header above).
    pub const DEV_CONFIG: u16 = 0x14;
}

/// Device STATUS bits (written by the driver to STATUS during initialisation).
/// The sequence RESET -> ACK -> DRIVER -> FEATURES_OK -> DRIVER_OK is the
/// device's "handshake": it tells the card which stage of setup we reached.
mod dev_status {
    pub const ACKNOWLEDGE: u8 = 1; // "we found you"
    pub const DRIVER: u8 = 2; // "a driver is attached"
    pub const DRIVER_OK: u8 = 4; // "setup complete, ready for I/O"
    pub const FEATURES_OK: u8 = 8; // "feature negotiation finished"
    pub const FAILED: u8 = 0x80; // "something went wrong, stop"
}

/// A single virtio queue descriptor (16 bytes: addr u64 + len u32 + flags u16
/// + next u16). A request is a CHAIN of descriptors linked by `next`:
///
///   desc[0]: request header (driver -> device)
///   desc[1]: data buffer    (direction depends on read vs write)
///   desc[2]: status byte    (device -> driver: 0 = success)
#[repr(C)]
#[derive(Clone, Copy)]
struct Desc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

/// The "available" ring header: buffers the driver offers to the device.
/// Followed in memory by `QUEUE_LEN` u16 ring slots (head indices).
#[repr(C)]
struct AvailHeader {
    flags: u16,
    idx: u16,
}

/// One "used" ring element: a completed buffer reported by the device.
#[repr(C)]
#[derive(Clone, Copy)]
struct UsedElem {
    id: u32, // head descriptor index of the finished chain
    len: u32, // bytes written (for reads)
}

/// The "used" ring header, followed by `QUEUE_LEN` UsedElem slots. It lives on
/// its OWN 4KB page: the legacy spec places the used ring at the next 4KB
/// boundary after the available ring, and the device computes that address
/// itself — so our queue memory must honour the alignment, not just the order.
#[repr(C)]
struct UsedHeader {
    flags: u16,
    idx: u16,
}

/// virtio_blk_request header (16 bytes): what operation, on which sector.
#[repr(C)]
struct BlkRequest {
    type_: u32,
    reserved: u32,
    sector: u64,
}

/// How many queue descriptors we use. Capped at 32: with 16-byte descriptors
/// plus the avail ring, request header, status byte and a 512-byte data
/// "bounce" buffer, everything except the used ring still fits in ONE page,
/// and the used ring fits in a second page (see QUEUE LAYOUT below).
const QUEUE_LEN: usize = 32;

/// QUEUE MEMORY LAYOUT (two contiguous physical pages at VIRTIO_QUEUE_PHYS):
///
///   Page 0:
///     +0x0000  desc[QUEUE_LEN]      (QUEUE_LEN * 16 bytes)
///     +desc_end avail {flags, idx}  (AvialHeader: 4 bytes)
///     +...     avail.ring           (QUEUE_LEN * u16 head indices)
///     +...     (pad to 8 bytes) req BlkRequest (16 bytes, 8-aligned)
///     +...     status byte          (device writes 0 on success)
///     +...     data bounce buffer   (512 bytes, see DMA LESSON below)
///   Page 1 (offset +0x1000, 4KB-aligned as the spec demands):
///     used {flags, idx} + used.ring (QUEUE_LEN * UsedElem)
///
/// LESSON (DMA): a virtio descriptor's `addr` must be a PHYSICAL address the
/// DEVICE can reach on the bus — not a Rust reference, not a virtual address!
/// An earlier revision passed `data.as_ptr() as u64` (a kernel-virtual stack
/// address) straight to the card. That only works if virtual == physical,
/// i.e. inside the kernel's low identity-mapped region, which a syscall stack
/// buffer is NOT. The correct pattern is a BOUNCE buffer: a staging area
/// inside queue memory whose physical address we know exactly. For writes we
/// copy the sector into the bounce buffer before kicking the device; for reads
/// we copy it out after completion. Slower than zero-copy DMA, but always
/// correct — and the standard trick in educational drivers.
const QUEUE_PAGES: usize = 2;
const QUEUE_BYTES: usize = QUEUE_PAGES * 4096;

/// BAR0 base port of the discovered virtio-blk device (0 = not present).
static mut VIRTIO_BASE: u16 = 0;
/// Device capacity in 512-byte sectors (from the config area).
static mut VIRTIO_SECTORS: u64 = 0;
/// Physical address of the two contiguous queue pages (0 = not allocated).
/// Reached by the CPU through the kernel identity map (frames are < 4 MiB,
/// verified at init) and by the DEVICE as raw physical addresses.
static mut VIRTIO_QUEUE_PHYS: u64 = 0;
/// Next available-ring slot to submit (monotonic submission counter).
static mut AVAIL_IDX: u16 = 0;
/// Last used-ring index we consumed (monotonic completion counter).
static mut SEEN_USED: u16 = 0;

/// Read one block from the virtio-blk device. Returns false on any failure.
unsafe fn virtio_read_block(block: usize, buf: &mut [u8; BLOCK_SIZE]) -> bool {
    if VIRTIO_BASE == 0 || block as u64 >= VIRTIO_SECTORS {
        return false;
    }
    virtio_transfer(block, buf.as_mut_ptr(), REQ_IN)
}

/// Write one block to the virtio-blk device. Returns false on any failure.
unsafe fn virtio_write_block(block: usize, buf: &[u8; BLOCK_SIZE]) -> bool {
    if VIRTIO_BASE == 0 || block as u64 >= VIRTIO_SECTORS {
        return false;
    }
    virtio_transfer(block, buf.as_ptr(), REQ_OUT)
}

/// Perform a single-sector transfer (REQ_IN = read, REQ_OUT = write).
///
/// `data` points at the caller's 512-byte sector buffer (source for writes,
/// destination for reads). It is NEVER handed to the device directly — see
/// the DMA LESSON in QUEUE LAYOUT above. Returns `true` iff the device
/// reported status 0 (`VIRTIO_BLK_S_OK`).
///
/// LESSON (completion): after kicking the queue with QUEUE_NOTIFY the driver
/// must WAIT for the device to finish. The device signals completion by
/// advancing `used.idx` and writing the status byte — a slow, asynchronous
/// event. An earlier revision polled `if used.id == 0 { return true }` on a
/// freshly-zeroed page, which is true IMMEDIATELY (zero == zero!), so every
/// transfer "succeeded" without the device ever running. The correct pattern:
/// snapshot `used.idx` BEFORE the kick, then wait until it CHANGES.
unsafe fn virtio_transfer(block: usize, data: *const u8, req_type: u32) -> bool {
    let base = VIRTIO_BASE;
    let q = VIRTIO_QUEUE_PHYS as usize;

    // --- Queue layout offsets (see QUEUE LAYOUT above). ---
    let desc_off = 0usize;
    let avail_off = desc_off + QUEUE_LEN * core::mem::size_of::<Desc>();
    let ring_off = avail_off + core::mem::size_of::<AvailHeader>();
    let mut misc_off = ring_off + QUEUE_LEN * core::mem::size_of::<u16>();
    misc_off = (misc_off + 7) & !7; // 8-align the request header (u64 field)
    let req_off = misc_off;
    let status_off = req_off + core::mem::size_of::<BlkRequest>();
    let data_off = status_off + 1;
    let used_off = 4096usize; // second page: 4KB-aligned per spec
    // Defensive: our fixed QUEUE_LEN must keep page-0 contents in page 0.
    if data_off + BLOCK_SIZE > 4096 {
        return false;
    }

    // --- 1. Build the request header + poison the status byte. ---
    let req = BlkRequest {
        type_: req_type,
        reserved: 0,
        sector: block as u64,
    };
    core::ptr::write_unaligned(
        (q + req_off) as *mut BlkRequest,
        req,
    );
    // Poison with 0xFF: if the device never runs, we see 0xFF (failure),
    // never a stale 0 (fake success). Trust, but verify.
    core::ptr::write_unaligned((q + status_off) as *mut u8, 0xFF);

    // --- 2. Stage the data through the bounce buffer. ---
    if req_type == REQ_OUT {
        core::ptr::copy_nonoverlapping(data, (q + data_off) as *mut u8, BLOCK_SIZE);
    }

    // --- 3. Build the 3-descriptor chain: header -> data -> status. ---
    let d = |addr: u64, len: u32, flags: u16, next: u16| Desc { addr, len, flags, next };
    let descs = [
        d((q + req_off) as u64, core::mem::size_of::<BlkRequest>() as u32, DESC_F_NEXT, 1),
        d(
            (q + data_off) as u64,
            BLOCK_SIZE as u32,
            // Direction flag: REQ_IN means the DEVICE writes the buffer.
            DESC_F_NEXT | if req_type == REQ_IN { DESC_F_WRITE } else { 0 },
            2,
        ),
        d((q + status_off) as u64, 1, DESC_F_WRITE, 0), // status: always device-written
    ];
    for (i, desc) in descs.iter().enumerate() {
        core::ptr::write_unaligned((q + desc_off + i * core::mem::size_of::<Desc>()) as *mut Desc, *desc);
    }

    // --- 4. Offer chain head 0 in the next avail-ring slot. ---
    // Single-threaded driver: we always reuse descriptors 0-2, so the head is
    // always 0; only the ring SLOT advances with each submission.
    let slot = (AVAIL_IDX as usize) % QUEUE_LEN;
    core::ptr::write_unaligned((q + ring_off + slot * 2) as *mut u16, 0);
    // Compiler fence: descriptors + avail entry must be written BEFORE idx.
    // (x86 stores are already ordered, but the fence documents the rule and
    // keeps this correct if the code is ever ported to a weaker architecture.)
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    AVAIL_IDX = AVAIL_IDX.wrapping_add(1);
    core::ptr::write_unaligned((q + avail_off + 2) as *mut u16, AVAIL_IDX);

    // --- 5. Kick queue 0 and wait for used.idx to ADVANCE past our snapshot. ---
    let seen = SEEN_USED;
    outw(base + legacy_reg::QUEUE_NOTIFY, 0);
    let mut done = false;
    for _ in 0..1_000_000u32 {
        let idx = core::ptr::read_unaligned((q + used_off + 2) as *const u16);
        if idx != seen {
            done = true;
            SEEN_USED = idx;
            break;
        }
        core::hint::spin_loop();
    }
    if !done {
        return false; // timeout: the device never completed
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

    // --- 6. The device ran: check WHAT it reported. ---
    let status = core::ptr::read_unaligned((q + status_off) as *const u8);
    if status != 0 {
        return false; // VIRTIO_BLK_S_OK == 0; anything else is an I/O error
    }
    if req_type == REQ_IN {
        core::ptr::copy_nonoverlapping((q + data_off) as *const u8, data as *mut u8, BLOCK_SIZE);
    }
    true
}

/// Small I/O helpers: one per port width (`in`/`out` come in 8/16/32-bit
/// flavours and the width is part of the instruction — `out dx, al` writes ONE
/// byte, `out dx, eax` writes FOUR. Using the wrong width silently talks to
/// the wrong register, which is exactly the PCI bug fixed above).
#[inline]
unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    core::arch::asm!("in al, dx", out("al") v, in("dx") port, options(nostack, preserves_flags));
    v
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inw(port: u16) -> u16 {
    let v: u16;
    core::arch::asm!("in ax, dx", out("ax") v, in("dx") port, options(nostack, preserves_flags));
    v
}

#[inline]
unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value, options(nostack, preserves_flags));
}

/// 32-bit port write. (Named `outl` after the Linux convention: x86 has no
/// 64-bit port I/O, so 32 bits is the widest single `out`.)
#[inline]
unsafe fn outl(port: u16, value: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inl(port: u16) -> u32 {
    let v: u32;
    core::arch::asm!("in eax, dx", out("eax") v, in("dx") port, options(nostack, preserves_flags));
    v
}

/// Discover and initialise the virtio-blk device. Returns true on success.
///
/// LESSON (device bring-up order): legacy virtio devices expect a strict
/// handshake — RESET the status, ACK the device, negotiate features, set up
/// queues, THEN declare DRIVER_OK. Skipping steps (or programming queues
/// before the device knows a driver is attached) leaves the card inert.
/// On ANY failure we return false and the caller falls back to the RAM disk:
/// a storage driver must never prevent the kernel from booting.
pub fn init_virtio_blk() -> bool {
    unsafe {
        // --- 1. Find the card: scan PCI bus 0 for vendor 0x1AF4 / device 0x1001.
        //
        // LESSON (config dword 0x00): the first config register packs TWO
        // 16-bit IDs: the LOW half is the VENDOR id, the HIGH half is the
        // DEVICE id. An earlier revision compared the low half against the
        // DEVICE id (0x1001) — but the low half is the vendor (0x1AF4), so the
        // comparison could never match and no card was ever found. Always
        // unpack both halves explicitly.
        let mut found_slot: Option<u8> = None;
        for slot in 0u8..32 {
            let id = pci_config_read(0, slot, 0, 0x00);
            let vendor = (id & 0xFFFF) as u16; // low half = vendor
            let device = (id >> 16) as u16; // high half = device
            if vendor == VIRTIO_PCI_VENDOR_ID && device == VIRTIO_BLK_DEVICE_ID {
                found_slot = Some(slot);
                break;
            }
        }
        let slot = match found_slot {
            Some(s) => s,
            None => return false,
        };

        // --- 2. Read BAR0: must be an I/O region (bit 0 == 1). ---
        // An I/O BAR's address lives in bits 31:2 (mask 0xFFFC); bits 1:0 are
        // type flags, NOT address. Masking with 0xFFFF would keep the flags
        // and shift every port access by up to 3.
        let bar0 = pci_config_read(0, slot, 0, 0x10);
        if bar0 & 0x1 == 0 {
            return false; // memory BAR: not a legacy transport, can't drive it
        }
        let base = (bar0 & 0xFFFC) as u16;
        if base == 0 {
            return false;
        }

        // --- 3. Handshake: reset -> ACK|DRIVER -> features -> FEATURES_OK. ---
        outb(base + legacy_reg::STATUS, 0); // reset the device
        outb(
            base + legacy_reg::STATUS,
            dev_status::ACKNOWLEDGE | dev_status::DRIVER,
        );
        // We request NO optional features (plain 512-byte sectors): offer 0.
        outl(base + legacy_reg::GUEST_FEATURES, 0);
        outb(
            base + legacy_reg::STATUS,
            dev_status::ACKNOWLEDGE | dev_status::DRIVER | dev_status::FEATURES_OK,
        );
        // The device clears FEATURES_OK if it rejects our offer — verify it stuck.
        if inb(base + legacy_reg::STATUS) & dev_status::FEATURES_OK == 0 {
            return false;
        }

        // --- 4. Program queue 0: select it, check its size, hand it memory. ---
        outw(base + legacy_reg::QUEUE_SEL, 0);
        let max_queue = inw(base + legacy_reg::QUEUE_NUM);
        if (max_queue as usize) < QUEUE_LEN || max_queue == 0 {
            return false; // device queue too small for our layout
        }
        // Two CONTIGUOUS frames: the device derives the used ring's address
        // from the queue base, so the pages must be adjacent. First-fit
        // allocation at boot gives adjacent frames; VERIFY instead of assuming.
        let f0 = crate::paging::alloc_frame();
        let f1 = crate::paging::alloc_frame();
        // Queue memory is reached via the kernel identity map, which covers
        // low memory only (0-4 MiB): a queue above that would fault on access.
        if f0 == 0 || f1 != f0 + 4096 || f0 + (QUEUE_BYTES as u64) > 0x400000 {
            return false; // (leaked frames are reclaimed at next boot; see below)
        }
        // QUEUE_PFN takes a PAGE FRAME NUMBER (address >> 12), not an address.
        // Our frames are well below 4 GiB, so the PFN fits in 32 bits.
        outl(base + legacy_reg::QUEUE_PFN, (f0 >> 12) as u32);

        // --- 5. Read capacity: u64 sector count at config offset 0x14. ---
        // x86 has no 64-bit port I/O, so read it as two 32-bit halves.
        // (An earlier revision read a u32 at 0x10 — which is QUEUE_NOTIFY, a
        // write-only kick port — and mistook kick-acknowledges for capacity.)
        let cap_lo = inl(base + legacy_reg::DEV_CONFIG) as u64;
        let cap_hi = inl(base + legacy_reg::DEV_CONFIG + 4) as u64;
        let sectors = cap_lo | (cap_hi << 32);
        if sectors == 0 {
            return false;
        }

        // --- 6. Go live. ---
        outb(
            base + legacy_reg::STATUS,
            dev_status::ACKNOWLEDGE
                | dev_status::DRIVER
                | dev_status::FEATURES_OK
                | dev_status::DRIVER_OK,
        );

        VIRTIO_BASE = base;
        VIRTIO_SECTORS = sectors;
        VIRTIO_QUEUE_PHYS = f0;
        AVAIL_IDX = 0;
        SEEN_USED = 0;

        crate::vga::serial_print("[virtio-blk] base=");
        crate::vga::serial_print_hex("", base as u64);
        crate::vga::serial_print(" sectors=");
        crate::vga::serial_print_hex("", sectors);
        crate::vga::serial_print("\n");
        true
    }
}

// =============================================================================
// PUBLIC INTERFACE (dispatches to whichever backend initialised first)
// =============================================================================

/// Initialise storage. Tries virtio-blk first; falls back to the RAM disk.
pub fn init() {
    unsafe {
        if init_virtio_blk() {
            BACKEND = Some(Backend::Virtio);
            crate::vga::serial_print("[block_dev] using virtio-blk\n");
            return;
        }
        crate::vga::serial_print("[block_dev] no virtio-blk; falling back to RAM disk\n");
        init_ram_disk();
        BACKEND = Some(Backend::Ram);
    }
}

/// Read one block. Returns `true` on success, `false` on failure.
pub fn read_block(block: usize, buf: &mut [u8; BLOCK_SIZE]) -> bool {
    unsafe {
        match BACKEND {
            Some(Backend::Virtio) => virtio_read_block(block, buf),
            _ => ram_read_block(block, buf),
        }
    }
}

/// Write one block. Returns `true` on success.
pub fn write_block(block: usize, buf: &[u8; BLOCK_SIZE]) -> bool {
    unsafe {
        match BACKEND {
            Some(Backend::Virtio) => virtio_write_block(block, buf),
            _ => ram_write_block(block, buf),
        }
    }
}

/// Total number of blocks available (virtio capacity or RAM disk size).
pub fn num_blocks() -> usize {
    unsafe {
        match BACKEND {
            Some(Backend::Virtio) => VIRTIO_SECTORS as usize,
            _ => NUM_BLOCKS,
        }
    }
}

/// Read a range of bytes from the active backend at a byte offset.
///
/// LESSON (integer safety): `end - offset` below is a subtraction on `usize`.
/// If `offset` points PAST the end of the device, `end` clamps to `total`,
/// which is SMALLER than `offset` — and unsigned subtraction would wrap around
/// (panic in debug builds, a gigantic bogus length in release). Hence the
/// explicit `end <= offset` guard: reading past the end yields zero bytes,
/// never a wrap. `saturating_add` likewise protects against `offset + len`
/// overflowing `usize` itself for absurd inputs.
pub fn read_bytes(offset: usize, buf: &mut [u8]) -> usize {
    let total = num_blocks() * BLOCK_SIZE;
    let end = offset.saturating_add(buf.len()).min(total);
    if end <= offset {
        return 0;
    }
    let actual = end - offset;
    let mut blk = offset / BLOCK_SIZE;
    let mut rem = offset % BLOCK_SIZE;
    let mut out_pos = 0;
    while out_pos < actual {
        let mut b = [0u8; BLOCK_SIZE];
        if !read_block(blk, &mut b) {
            return out_pos;
        }
        let copy = core::cmp::min(BLOCK_SIZE - rem, actual - out_pos);
        buf[out_pos..out_pos + copy].copy_from_slice(&b[rem..rem + copy]);
        out_pos += copy;
        rem = 0;
        blk += 1;
    }
    actual
}
