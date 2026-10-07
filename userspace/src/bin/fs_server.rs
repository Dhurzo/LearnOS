//! fs_server — Phase 6 storage smoke-test server (user space).
//!
//! Loads sector 0 from the block device, parses the FAT16 BPB, lists the root
//! directory, and reads a file by name. All output goes through `vga_write`
//! (which forwards to serial), so it is observable in QEMU's `-serial mon:stdio`.
//!
//! On startup it prints one line per root entry plus the contents of any file
//! named "README". It then idles like init rather than exiting, so loading it at
//! boot does not halt the system.

#![no_std]
#![no_main]

use userspace::block_driver::{read_sector, last_reason, SECTOR_SIZE};
use userspace::fs::Fat16;
use userspace::{fs_proto, ipc_recv, ipc_send, vga_write, syscall0, syscall_nr, IpcMessage, MSG_STORAGE_CONFIG};

/// Staging area for file reads (16 KiB in .bss — mapped as part of our image).
static mut FILE_BUF: [u8; fs_proto::MAX_STAGED] = [0; fs_proto::MAX_STAGED];

/// Print a byte string one character at a time via the VGA/serial forwarder.
fn print(s: &[u8]) {
    unsafe {
        for &b in s.iter() {
            // The VGA server drains one char per IPC pop; retry until the
            // character is actually accepted (queue may be momentarily full).
            while vga_write(b as usize) < 0 {}
        }
    }
}

/// Print an ASCII number (base 10) via vga_write.
fn print_num(mut n: u32) {
    if n == 0 {
        print(b"0");
        return;
    }
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    print(&digits[i..]);
}

/// Send with bounded retries: the receiver's queue may be momentarily full.
/// Gives up (returns false) instead of wedging the server on a dead peer.
fn send_retry(dst_pid: usize, msg: &IpcMessage) -> bool {
    unsafe {
        for _ in 0..10_000u32 {
            if ipc_send(dst_pid, msg as *const IpcMessage as usize) == 0 {
                return true;
            }
            let _ = syscall0(syscall_nr::SCHEDULE);
        }
        false
    }
}
/// Print a hex byte via vga_write.
fn print_hex(b: u8) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    unsafe {
        let _ = vga_write(HEX[(b >> 4) as usize] as usize);
        let _ = vga_write(HEX[(b & 0x0f) as usize] as usize);
    }
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    unsafe {
        // The kernel boot code is a tiny resource manager: it grants this
        // process the IDE register ports via the TSS bitmap and pushes a
        // storage-config message. The ATA protocol itself is implemented
        // here in user space.
        let mut config: IpcMessage = core::mem::zeroed();
        let received = ipc_recv(&mut config as *mut IpcMessage as usize);
        if received <= 0 || config.msg_type != MSG_STORAGE_CONFIG {
            print(b"No storage-device resources were granted by the kernel.\n");
            idle();
        }
        let io_base = u16::from_le_bytes([config.data[0], config.data[1]]);
        let queue_phys = u64::from_le_bytes([
            config.data[2], config.data[3], config.data[4], config.data[5],
            config.data[6], config.data[7], config.data[8], config.data[9],
        ]);
        let ok = userspace::block_driver::init(io_base, queue_phys);
        if !ok {
            print(b"VirtIO-blk initialization failed.\n");
            idle();
        }

        print(b"=== fs_server: FAT16 smoke test ===\n");

        // Load sector 0 (the boot sector / BPB).
        let mut sec = [0u8; SECTOR_SIZE];
        if !read_sector(0, &mut sec) {
            print(b"No block device readable at boot. R=");
            print_hex(last_reason());
            print(b"\n");
            idle();
        }
        let fat = match Fat16::new(&sec) {
            None => {
                print(b"Sector 0 is not a FAT16 volume (no valid BPB).\n");
                print(b"first4=");
                for b in sec[..4].iter() {
                    print_hex(*b);
                }
                print(b"\n");
                idle();
            }
            Some(fat) => fat,
        };

        // Sector reader shared by the smoke test and the file server below.
        // One-sector cache: directory scans re-read the same sector once per
        // entry (16 ATA round-trips for a single-sector root dir), and FAT
        // walks revisit sectors too. The image is read-only, so caching is
        // safe and cuts block I/O by an order of magnitude.
        let mut cache_sec = usize::MAX;
        let mut cache = [0u8; SECTOR_SIZE];
        let mut read = |sector: usize, buf: &mut [u8]| -> usize {
            let n = core::cmp::min(SECTOR_SIZE, buf.len());
            if sector == cache_sec {
                buf[..n].copy_from_slice(&cache[..n]);
                return n;
            }
            let mut s = [0u8; SECTOR_SIZE];
            if read_sector(sector, &mut s) {
                buf[..n].copy_from_slice(&s[..n]);
                cache.copy_from_slice(&s);
                cache_sec = sector;
                n
            } else {
                0
            }
        };

        // Boot smoke test (HITO-4 gate watches for these lines).
        print(b"Root directory:\n");
        let mut entries = [None; 64];
        let count = fat.list_root(&mut read, &mut entries);
        for e in entries.iter().take(count) {
            if let Some(e) = e {
                print(b"  ");
                print(e.display_name().as_bytes());
                if e.is_dir {
                    print(b"/");
                }
                print(b"  size=");
                print_num(e.size);
                print(b"  cluster=");
                print_hex(e.first_cluster as u8);
                print(b"\n");
            }
        }

        // Read a file named "README" if present.
        match fat.find(&mut read, "README") {
            Some(entry) => {
                print(b"\n--- README contents ---\n");
                let mut out = [0u8; 256];
                let n = fat.read_file(&mut read, &entry, &mut out);
                print(&out[..n]);
                if n < out.len() {
                    print(b"\n");
                }
            }
            None => {
                print(b"No 'README' file found in root.\n");
            }
        }

        // File-service loop: answer the shell's LIST/READ requests forever.
        // (The smoke test above already proved block I/O works; from here on
        // this process is a server, not a test.)
        loop {
            let mut req: IpcMessage = unsafe { core::mem::zeroed() };
            if unsafe { ipc_recv(&mut req as *mut IpcMessage as usize) } <= 0 {
                unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
                continue;
            }
            let from = req.src_pid as usize;
            match req.msg_type {
                fs_proto::LIST => {
                    let mut entries = [None; 64];
                    let count = fat.list_root(&mut read, &mut entries);
                    for e in entries.iter().take(count).flatten() {
                        let mut d = [0u8; 60];
                        d[..11].copy_from_slice(&e.raw_name);
                        d[11..15].copy_from_slice(&e.size.to_le_bytes());
                        d[15..17].copy_from_slice(&e.first_cluster.to_le_bytes());
                        d[17] = if e.is_dir { 1 } else { 0 };
                        let m = IpcMessage::new(0, fs_proto::ENTRY, d);
                        if !send_retry(from, &m) {
                            break;
                        }
                    }
                    let end = IpcMessage::new(0, fs_proto::END, [0u8; 60]);
                    let _ = send_retry(from, &end);
                }
                fs_proto::READ => {
                    let mut name = [0u8; 11];
                    name.copy_from_slice(&req.data[..11]);
                    let off = u32::from_le_bytes([req.data[12], req.data[13], req.data[14], req.data[15]]) as usize;
                    let mut len = u32::from_le_bytes([req.data[16], req.data[17], req.data[18], req.data[19]]) as usize;
                    if len > fs_proto::DATA_MAX {
                        len = fs_proto::DATA_MAX;
                    }
                    let mut d = [0u8; 60];
                    // Locate the entry by raw 8.3 bytes (shell sends upper-case).
                    let mut entries = [None; 64];
                    let count = fat.list_root(&mut read, &mut entries);
                    let mut found = None;
                    for e in entries.iter().take(count).flatten() {
                        if e.raw_name == name {
                            found = Some(*e);
                            break;
                        }
                    }
                    match found {
                        None => {
                            d[..4].copy_from_slice(&fs_proto::READ_ERROR.to_le_bytes());
                        }
                        Some(entry) => {
                            let size = entry.size as usize;
                            if size > fs_proto::MAX_STAGED {
                                d[..4].copy_from_slice(&fs_proto::READ_ERROR.to_le_bytes());
                            } else {
                                let staged: &mut [u8] = unsafe {
                                    core::slice::from_raw_parts_mut(
                                        core::ptr::addr_of_mut!(FILE_BUF) as *mut u8,
                                        size.max(1),
                                    )
                                };
                                let n = fat.read_file(&mut read, &entry, &mut staged[..size]);
                                let avail = n.saturating_sub(off);
                                let take = core::cmp::min(len, avail);
                                d[..4].copy_from_slice(&(take as u32).to_le_bytes());
                                d[4..4 + take].copy_from_slice(&staged[off..off + take]);
                            }
                        }
                    }
                    let m = IpcMessage::new(0, fs_proto::DATA, d);
                    let _ = send_retry(from, &m);
                }
                _ => {}
            }
        }
    }

    idle();
}

/// Idle forever, yielding the CPU on each empty time slice (mirrors init).
fn idle() -> ! {
    loop {
        unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
    }
}
