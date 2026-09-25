//! Narrow PCI resource discovery for boot-time driver assignment.
//!
//! LESSON (microkernel boundary): this module does not issue block requests or
//! understand virtqueues. It identifies a legacy virtio-blk PCI function, enables
//! its I/O-space BAR and bus mastering, then lets the storage process own the
//! protocol using a per-process TSS I/O-port grant and DMA pages.

/// Port span used by the legacy virtio-blk common header and 64-bit capacity.
pub const VIRTIO_LEGACY_IO_PORT_LEN: u16 = 0x20;

const VIRTIO_PCI_VENDOR_ID: u16 = 0x1AF4;
const VIRTIO_BLK_DEVICE_ID: u16 = 0x1001;
const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// Find the first legacy virtio-blk function on PCI bus 0 and return its BAR0
/// I/O base. Device protocol and queue setup are deliberately left to userland.
pub fn probe_virtio_io_base() -> Option<u16> {
    unsafe {
        for device in 0u8..32 {
            let id = pci_config_read(0, device, 0, 0x00);
            let vendor = (id & 0xFFFF) as u16;
            let device_id = (id >> 16) as u16;
            if vendor != VIRTIO_PCI_VENDOR_ID || device_id != VIRTIO_BLK_DEVICE_ID {
                continue;
            }

            let bar0 = pci_config_read(0, device, 0, 0x10);
            if bar0 & 1 == 0 {
                return None; // Modern MMIO transport needs a different user driver.
            }
            let base = (bar0 & 0xFFFC) as u16;
            if base == 0 || base.checked_add(VIRTIO_LEGACY_IO_PORT_LEN - 1).is_none() {
                return None;
            }

            // PCI Command bits: I/O Space Enable and Bus Master Enable. This is
            // resource setup only; the device's request protocol stays in userland.
            let command = pci_config_read(0, device, 0, 0x04) as u16 | 0x0005;
            pci_config_write_u16(0, device, 0, 0x04, command);
            return Some(base);
        }
    }
    None
}

/// Read one aligned 32-bit PCI configuration register (mechanism #1).
unsafe fn pci_config_read(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let address = (1u32 << 31)
        | ((bus as u32) << 16)
        | (((device & 0x1F) as u32) << 11)
        | (((function & 0x07) as u32) << 8)
        | ((offset & 0xFC) as u32);
    core::arch::asm!(
        "out dx, eax",
        in("dx") PCI_CONFIG_ADDRESS,
        in("eax") address,
        options(nostack, preserves_flags),
    );
    let value: u32;
    core::arch::asm!(
        "in eax, dx",
        in("dx") PCI_CONFIG_DATA,
        out("eax") value,
        options(nostack, preserves_flags),
    );
    value
}

/// Write only the PCI Command half-word; the adjacent Status register contains
/// write-one-to-clear bits and must not be overwritten with the read value.
unsafe fn pci_config_write_u16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
    let address = (1u32 << 31)
        | ((bus as u32) << 16)
        | (((device & 0x1F) as u32) << 11)
        | (((function & 0x07) as u32) << 8)
        | ((offset & 0xFC) as u32);
    core::arch::asm!(
        "out dx, eax",
        in("dx") PCI_CONFIG_ADDRESS,
        in("eax") address,
        options(nostack, preserves_flags),
    );
    core::arch::asm!(
        "out dx, ax",
        in("dx") PCI_CONFIG_DATA,
        in("ax") value,
        options(nostack, preserves_flags),
    );
}
