//! Storage resource assignment (the kernel's only job for the block device).
//!
//! LESSON (microkernel boundary): the kernel does not know what a disk is.
//! It does not parse FAT, does not speak ATA or virtio, and never touches the
//! block device on a server's behalf. Its entire contribution is:
//!
//!   1. Name the resource: the ISA-style IDE task-file registers
//!      (`0x1F0..0x1F7`) that the storage server is allowed to use.
//!   2. Grant those ports to that one process through the TSS I/O bitmap.
//!   3. Tell the server which ports it got, over one IPC message.
//!
//! Everything else — the ATA command protocol, LBA geometry, the FAT16
//! directory tree — lives in the user-space storage server
//! (`userspace/src/block_driver.rs` and `userspace/src/fs.rs`).
//!
//! Why IDE and not virtio-blk: the user-space driver implements the ATA
//! PIO transport, which every QEMU `pc` machine exposes through the fixed
//! ISA ports. A legacy virtio-blk function would need its own port-based
//! transport plus a DMA queue in the driver; until that driver exists,
//! probing for virtio would only hand the server ports its ATA code cannot
//! use.

/// ISA-style IDE hard-disk register ports (data port first, 8 registers).
pub const IDE_DATA_PORT: u16 = 0x1F0;
pub const IDE_IO_PORT_LEN: u16 = 8;

/// Port span used by the legacy virtio-blk common header and 64-bit capacity.
pub const VIRTIO_LEGACY_IO_PORT_LEN: u16 = 0x20;

/// The storage resource handed to the user-space storage server at boot.
pub struct StorageResource {
    /// First granted I/O port (the IDE data port).
    pub io_base: u16,
    /// Number of granted ports.
    pub io_len: u16,
}

/// The storage resource handed to the user-space storage server at boot.
/// This kernel only supports the legacy ISA IDE task-file registers at 0x1F0.
/// The PCI IDE controller is NOT enabled; the legacy ISA ports are used directly.
pub fn storage_resource() -> StorageResource {
    StorageResource {
        io_base: IDE_DATA_PORT,
        io_len: IDE_IO_PORT_LEN,
    }
}

/// ISA-style IDE hard-disk register ports (data port first, 8 registers).
pub const IDE_DATA_PORT: u16 = 0x1F0;
pub const IDE_IO_PORT_LEN: u16 = 8;

/// Port span used by the legacy virtio-blk common header and 64-bit capacity.
pub const VIRTIO_LEGACY_IO_PORT_LEN: u16 = 0x20;

/// The storage resource handed to the user-space storage server at boot.
pub struct StorageResource {
    /// First granted I/O port (the IDE data port).
    pub io_base: u16,
    /// Number of granted ports.
    pub io_len: u16,
}

/// The storage resource handed to the user-space storage server at boot.
/// This kernel only supports the legacy ISA IDE task-file registers at 0x1F0.
/// The PCI IDE controller is NOT enabled; the legacy ISA ports are used directly.
pub fn storage_resource() -> StorageResource {
    StorageResource {
        io_base: IDE_DATA_PORT,
        io_len: IDE_IO_PORT_LEN,
    }
}
