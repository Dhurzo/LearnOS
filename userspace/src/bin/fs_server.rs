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

use userspace::block_driver::{read_sector, SECTOR_SIZE};
use userspace::fs::Fat16;
use userspace::{vga_write, syscall0, syscall_nr};

/// Print a byte string one character at a time via the VGA/serial forwarder.
fn print(s: &[u8]) {
    unsafe {
        for &b in s.iter() {
            let _ = vga_write(b as usize);
        }
    }
}

/// Print an ASCII number (base 10) via vga_write.
fn print_num(mut n: u32) {
    if n == 0 {
        unsafe { let _ = vga_write(0); }
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
        print(b"=== fs_server: FAT16 smoke test ===\n");

        // Load sector 0 (the boot sector / BPB).
        let mut sec = [0u8; SECTOR_SIZE];
        if !read_sector(0, &mut sec) {
            print(b"No block device readable at boot.\n");
            idle();
        }
        match Fat16::new(&sec) {
            None => {
                print(b"Sector 0 is not a FAT16 volume (no valid BPB).\n");
                idle();
            }
            Some(fat) => {
                // Wire the parser to real block I/O.
                let mut read = |sector: usize, buf: &mut [u8]| -> usize {
                    let mut s = [0u8; SECTOR_SIZE];
                    if read_sector(sector, &mut s) {
                        let n = core::cmp::min(SECTOR_SIZE, buf.len());
                        buf[..n].copy_from_slice(&s[..n]);
                        n
                    } else {
                        0
                    }
                };

                // List the root directory.
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
