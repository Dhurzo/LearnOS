//! Minimal Embedded Filesystem (Phase 3 — File System + Block Driver)
//!
//! Provides a flat file table with built-in file data embedded in the kernel
//! binary.  In a mature system this would be replaced by a FAT/ext2 reader
//! on a real block device; for now it gives us a working `open`/`read`/`close`
//! interface that the shell and ELF loader can use.
//!
//! Design:
//!   The kernel reserves a small RAM disk area at boot.  Files are stored
//!   as contiguous blocks in this RAM disk.  A directory table maps file
//!   names to (offset, size) pairs.
//!
//!   For the built-in files we embed static byte slices in the kernel image
//!   and point directory entries at them.  This is the "initramfs" pattern:
//!   the kernel ships with a few essential files baked in.

use core::str;

/// Maximum number of files in the file table.
const MAX_FILES: usize = 16;

/// Maximum filename length (including null terminator).
const MAX_NAME_LEN: usize = 32;

/// Maximum number of open file descriptors per process.
const MAX_FDS: usize = 8;

/// A single directory entry mapping a filename to data in the ramdisk.
#[derive(Clone, Copy)]
struct DirEntry {
    name: [u8; MAX_NAME_LEN],
    data: Option<&'static [u8]>,
}

/// Per-process open file descriptor.
#[derive(Clone, Copy)]
struct OpenFile {
    /// Index into DIR_TABLE, or None if slot is free.
    file_id: Option<usize>,
    /// Current read/write cursor.
    pos: usize,
}

/// The global directory table (populated at compile time).
static DIR_TABLE: [DirEntry; MAX_FILES] = build_dir_table();

/// Per-process open-file table.
static mut OPEN_FILES: [[OpenFile; MAX_FDS]; crate::process::MAX_PROCESSES] =
    [[OpenFile { file_id: None, pos: 0 }; MAX_FDS]; crate::process::MAX_PROCESSES];

// =============================================================================
// Built-in files — data embedded in the kernel binary
// =============================================================================

/// A simple "hello.txt" file that the shell can read.
const HELLO_TXT: &[u8] = b"Hello from the LearnOS filesystem!\nThis file is embedded in the kernel.\n";

/// A short "README" file.
const README_TXT: &[u8] = b"LearnOS - a minimal microkernel for learning.\n\nAvailable commands:\n  help  - show this message\n  read  - read a file by name\n  exec  - exec a built-in program\n  clear - clear the screen\n";

/// A file containing a simple ELF-like header description (for testing).
const BOOT_INFO: &[u8] = b"[boot]\nkernel=learnos.bin\ninitrd=init.tar\nroot=/dev/ram0\n";

/// Build the directory table at compile time.
const fn build_dir_table() -> [DirEntry; MAX_FILES] {
    let mut table = [DirEntry {
        name: [0u8; MAX_NAME_LEN],
        data: None,
    }; MAX_FILES];

    // Populate built-in files — these live at fixed table indices 0..3.

    table[0] = DirEntry {
        name: {
            let src = b"hello.txt";
            let mut dst = [0u8; MAX_NAME_LEN];
            let mut i = 0;
            while i < src.len() {
                dst[i] = src[i];
                i += 1;
            }
            dst
        },
        data: Some(HELLO_TXT),
    };

    table[1] = DirEntry {
        name: {
            let src = b"README";
            let mut dst = [0u8; MAX_NAME_LEN];
            let mut i = 0;
            while i < src.len() {
                dst[i] = src[i];
                i += 1;
            }
            dst
        },
        data: Some(README_TXT),
    };

    table[2] = DirEntry {
        name: {
            let src = b"boot.cfg";
            let mut dst = [0u8; MAX_NAME_LEN];
            let mut i = 0;
            while i < src.len() {
                dst[i] = src[i];
                i += 1;
            }
            dst
        },
        data: Some(BOOT_INFO),
    };

    table
}

/// Convert a filename string to the canonical table-name format.
fn normalize_name(name: &str) -> [u8; MAX_NAME_LEN] {
    let mut buf = [0u8; MAX_NAME_LEN];
    let bytes = name.as_bytes();
    let len = bytes.len().min(MAX_NAME_LEN - 1);
    let mut i = 0;
    while i < len {
        buf[i] = bytes[i];
        i += 1;
    }
    buf
}

/// Find the directory index for a given filename, or None.
fn find_file(name: &str) -> Option<usize> {
    let normalized = normalize_name(name);
    for (i, entry) in DIR_TABLE.iter().enumerate() {
        if entry.data.is_some() && entry.name == normalized {
            return Some(i);
        }
    }
    None
}

// =============================================================================
// Public API — called from kernel (syscall handlers or directly)
// =============================================================================

/// Open a file by name.
///
/// Returns a file descriptor (>= 0) on success, or None if the file
/// was not found or the per-process FD table is full.
pub fn open(pid: u16, name: &str) -> Option<usize> {
    let file_id = find_file(name)?;
    let pid_idx = pid as usize;
    if pid_idx >= crate::process::MAX_PROCESSES {
        return None;
    }
    unsafe {
        let slots = &mut OPEN_FILES[pid_idx];
        for (fd, slot) in slots.iter_mut().enumerate() {
            if slot.file_id.is_none() {
                slot.file_id = Some(file_id);
                slot.pos = 0;
                return Some(fd);
            }
        }
    }
    None // All FD slots occupied
}

/// Read from an open file descriptor into a buffer.
///
/// Returns the number of bytes read, or None if the FD is invalid.
pub fn read(pid: u16, fd: usize, buf: &mut [u8]) -> Option<usize> {
    let pid_idx = pid as usize;
    if pid_idx >= crate::process::MAX_PROCESSES {
        return None;
    }
    unsafe {
        let slot = &mut OPEN_FILES[pid_idx][fd];
        let file_id = slot.file_id?;
        let entry = &DIR_TABLE[file_id];
        let data = entry.data?;
        let remaining = data.len().saturating_sub(slot.pos);
        let to_copy = buf.len().min(remaining);
        if to_copy > 0 {
            buf[..to_copy].copy_from_slice(&data[slot.pos..slot.pos + to_copy]);
            slot.pos += to_copy;
        }
        Some(to_copy)
    }
}

/// Close an open file descriptor.
pub fn close(pid: u16, fd: usize) -> bool {
    let pid_idx = pid as usize;
    if pid_idx >= crate::process::MAX_PROCESSES {
        return false;
    }
    unsafe {
        let slot = &mut OPEN_FILES[pid_idx][fd];
        if slot.file_id.is_some() {
            slot.file_id = None;
            slot.pos = 0;
            true
        } else {
            false
        }
    }
}

/// List all files in the directory table (for `ls`-like commands).
/// Returns up to `max` entries, writing names into the provided buffer
/// as null-terminated strings.
pub fn list(buf: &mut [u8]) -> usize {
    let mut written = 0;
    for entry in DIR_TABLE.iter() {
        if entry.data.is_none() {
            continue;
        }
        // Find the null terminator in the name
        let name_len = entry.name.iter().position(|&c| c == 0).unwrap_or(MAX_NAME_LEN);
        let name_slice = &entry.name[..name_len];
        // Copy name + newline into buffer
        let copy_len = (name_len + 1).min(buf.len().saturating_sub(written));
        if copy_len == 0 {
            break;
        }
        let end = written + name_len.min(copy_len);
        buf[written..end].copy_from_slice(&name_slice[..copy_len.min(name_len)]);
        if end < buf.len() {
            buf[end] = b'\n';
        }
        written = end + 1;
    }
    written
}

/// Get the size of an open file (for pread / mmap).
pub fn file_size(pid: u16, fd: usize) -> Option<usize> {
    let pid_idx = pid as usize;
    if pid_idx >= crate::process::MAX_PROCESSES {
        return None;
    }
    unsafe {
        let slot = &OPEN_FILES[pid_idx][fd];
        let file_id = slot.file_id?;
        let entry = &DIR_TABLE[file_id];
        entry.data.map(|d| d.len())
    }
}

/// Read the entire data of a file into a caller-provided buffer.
/// Returns the number of bytes copied.
pub fn read_all(pid: u16, fd: usize, buf: &mut [u8]) -> Option<usize> {
    let pid_idx = pid as usize;
    if pid_idx >= crate::process::MAX_PROCESSES {
        return None;
    }
    unsafe {
        let slot = &OPEN_FILES[pid_idx][fd];
        let file_id = slot.file_id?;
        let entry = &DIR_TABLE[file_id];
        let data = entry.data?;
        let to_copy = buf.len().min(data.len());
        buf[..to_copy].copy_from_slice(&data[..to_copy]);
        Some(to_copy)
    }
}
