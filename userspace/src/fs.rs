//! fs — Phase 6 filesystem crate (user space).
//!
//! A minimal **FAT16** parser that runs entirely in user space on top of the
//! `block_driver` raw-block interface. It understands the FAT16 layout well
//! enough to:
//!   * parse the Boot Parameter Block (BPB) from sector 0,
//!   * list the root directory, and
//!   * read a file by following its FAT cluster chain.
//!
//! The parser is deliberately decoupled from storage: every operation takes an
//! injectable `read` closure (`FnMut(usize /*sector*/, &mut [u8])`). Production
//! code wires it to `block_driver::read_sector`; tests wire it to a stack buffer,
//! so the parsing logic can be verified without QEMU.

use crate::block_driver::SECTOR_SIZE;

/// A parsed 32-byte FAT16 root-directory entry.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DirEntry {
    /// 8.3 name with spaces padded (raw bytes as stored on disk).
    pub raw_name: [u8; 11],
    /// True if this is a subdirectory entry.
    pub is_dir: bool,
    /// First cluster of the file/directory (low word).
    pub first_cluster: u16,
    /// File size in bytes.
    pub size: u32,
}

impl DirEntry {
    /// The file name without extension padding, e.g. `"README"`.
    pub fn display_name(&self) -> &str {
        let mut end = 0;
        while end < self.raw_name.len() && self.raw_name[end] != b' ' {
            end += 1;
        }
        // SAFETY: `end` is bounded by raw_name.len(); the bytes are ASCII.
        unsafe { core::str::from_utf8_unchecked(&self.raw_name[..end]) }
    }

    /// True if this entry is a valid, usable file or directory (not erased/empty).
    pub fn is_valid(&self) -> bool {
        self.raw_name[0] != 0x00 && self.raw_name[0] != 0xE5
    }
}

/// A parsed FAT16 volume geometry, derived from the BPB in sector 0.
#[derive(Clone, Copy)]
pub struct Fat16 {
    bytes_per_sector: u32,
    sectors_per_cluster: u32,
    reserved_sectors: u32,
    root_dir_entry_count: u16,
    data_start_sector: u32,
    root_dir_sector: u32,
}

impl Fat16 {
    /// Parse the BPB from the raw bytes of sector 0.
    ///
    /// Returns `None` if the image is not a recognisable FAT16 volume (bad magic
    /// or zero-valued geometry fields).
    pub fn new(sector0: &[u8]) -> Option<Fat16> {
        if sector0.len() < 0x30 || sector0[510] != 0x55 || sector0[511] != 0xAA {
            return None;
        }
        let u16_le = |b: &[u8], off: usize| -> u32 {
            (b[off] as u32) | ((b[off + 1] as u32) << 8)
        };

        let bytes_per_sector = u16_le(sector0, 0x0B) as u32;
        let sectors_per_cluster = sector0[0x0D] as u32;
        let reserved_sectors = u16_le(sector0, 0x0E) as u32;
        let num_fats = sector0[0x10];
        let root_dir_entry_count = u16_le(sector0, 0x11) as u32;
        let fat_size_sectors = u16_le(sector0, 0x16) as u32;

        if bytes_per_sector == 0 || sectors_per_cluster == 0 || num_fats == 0 {
            return None;
        }

        // Root directory occupies whole sectors.
        //
        // LESSON (FAT16 on-disk layout): a FAT16 volume is a sequence of
        // REGIONS, back to back from sector 0:
        //
        //   [ reserved | FAT #1 .. FAT #N | root directory | data clusters ]
        //     ^           ^                  ^
        //     |           |                  +-- root_dir_sector: right AFTER
        //     |           |                      the last FAT. NOT after the
        //     |           +--------------------- root dir itself!
        //     +-- sector 0 (boot sector + BPB)
        //
        // An earlier revision computed
        //   root_dir_sector = reserved + fats*fat_size + root_dir_sectors
        // i.e. it skipped the root directory's OWN sectors and landed in the
        // data area — every directory read returned file contents instead of
        // entries. The root dir starts immediately after the FATs; the DATA
        // area starts after the root dir.
        let root_dir_sectors = (root_dir_entry_count * 32 + bytes_per_sector - 1) / bytes_per_sector;
        let root_dir_sector = reserved_sectors + num_fats as u32 * fat_size_sectors;
        // The data area begins immediately after the root directory.
        let data_start_sector = root_dir_sector + root_dir_sectors;

        Some(Fat16 {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            root_dir_entry_count: root_dir_entry_count as u16,
            data_start_sector,
            root_dir_sector,
        })
    }

    /// Convert a cluster number to its starting sector.
    #[inline]
    fn cluster_to_sector(&self, cluster: u16) -> u32 {
        self.data_start_sector + (cluster as u32 - 2) * self.sectors_per_cluster
    }

    /// Read one FAT entry (the next cluster in a chain), or `None` if unreadable.
    fn read_fat_entry<F>(&self, mut read: F, cluster: u16) -> Option<u16>
    where
        F: FnMut(usize, &mut [u8]) -> usize,
    {
        let bps = self.bytes_per_sector;
        // FAT region starts at `reserved_sectors`; each FAT16 entry is 2 bytes.
        let byte_off = self.reserved_sectors * bps + cluster as u32 * 2;
        let mut buf0 = [0u8; SECTOR_SIZE];
        if read(byte_off as usize / bps as usize, &mut buf0) == 0 {
            return None;
        }
        let off_in = (byte_off % bps) as usize;
        // A two-byte entry may straddle a sector boundary.
        let mut hi = [0u8; SECTOR_SIZE];
        let crossed = off_in + 2 > bps as usize;
        if crossed {
            if read((byte_off as usize + 1) / bps as usize, &mut hi) == 0 {
                return None;
            }
        }
        let lo = buf0[off_in] as u16;
        let h = if crossed {
            hi[(off_in + 1) % (bps as usize)] as u16
        } else {
            buf0[off_in + 1] as u16
        };
        Some(lo | (h << 8))
    }

    /// Read a single root-directory entry by index.
    pub fn root_entry<F>(&self, mut read: F, index: usize) -> Option<DirEntry>
    where
        F: FnMut(usize, &mut [u8]) -> usize,
    {
        if index >= self.root_dir_entry_count as usize {
            return None;
        }
        let bps = self.bytes_per_sector as usize;
        let entry_off = index * 32;
        let sector = self.root_dir_sector as usize + entry_off / bps;
        let in_sector = entry_off % bps;
        let mut buf0 = [0u8; SECTOR_SIZE];
        if read(sector, &mut buf0) == 0 {
            return None;
        }
        // A 32-byte entry may span two sectors.
        let mut buf1 = [0u8; SECTOR_SIZE];
        if in_sector + 32 > bps {
            if read(sector + 1, &mut buf1) == 0 {
                return None;
            }
        }
        let get = |pos: usize| -> u8 {
            let p = in_sector + pos;
            if p < bps {
                buf0[p]
            } else {
                buf1[p - bps]
            }
        };

        let mut raw = [0u8; 11];
        for pos in 0..11 {
            raw[pos] = get(pos);
        }
        // LESSON (directory-entry layout, 32 bytes — offsets are NOT
        // interchangeable!): the fields we need live at FIXED positions:
        //
        //   0x00..0x0B  name (8.3, space-padded)
        //   0x0B        attributes
        //   0x1A..0x1C  first cluster, LOW word (little-endian)
        //   0x1C..0x20  file SIZE in bytes (u32, little-endian)
        //
        // An earlier revision read the cluster from 0x1C (that's the first
        // SIZE byte!) mixed with 0x1A, and the size from 0x1E..0x22 (two bytes
        // past the end of the entry!) — so every file pointed at a garbage
        // cluster with a garbage length. The unit-test image builder shared
        // the same wrong offsets, which is why the tests still passed: a test
        // that bakes the bug into its fixture proves nothing. Both are fixed
        // here; see `make_image` below.
        let attrs = get(0x0A);
        let first_cluster = get(0x1A) as u16 | (get(0x1B) as u16) << 8;
        let size = (get(0x1C) as u32)
            | (get(0x1D) as u32) << 8
            | (get(0x1E) as u32) << 16
            | (get(0x1F) as u32) << 24;

        Some(DirEntry {
            raw_name: raw,
            is_dir: attrs & 0x10 != 0,
            first_cluster,
            size,
        })
    }

    /// List the root directory into `out`, returning the number of valid entries.
    ///
    /// Erased (0xE5) and empty (all-zero) entries are skipped. Volume labels are
    /// reported but flagged via `is_dir` being false and attribute 0x08/0x20.
    pub fn list_root<F>(&self, mut read: F, out: &mut [Option<DirEntry>]) -> usize
    where
        F: FnMut(usize, &mut [u8]) -> usize,
    {
        let mut count = 0usize;
        for index in 0..self.root_dir_entry_count as usize {
            if count >= out.len() {
                break;
            }
            match self.root_entry(&mut read, index) {
                Some(e) if e.is_valid() => {
                    out[count] = Some(e);
                    count += 1;
                }
                _ => {}
            }
        }
        count
    }

    /// Find a file or directory by its 8.3 name (case-insensitive).
    pub fn find<F>(&self, mut read: F, name: &str) -> Option<DirEntry>
    where
        F: FnMut(usize, &mut [u8]) -> usize,
    {
        let want = to_83(name);
        for index in 0..self.root_dir_entry_count as usize {
            if let Some(e) = self.root_entry(&mut read, index) {
                // Case-insensitive: normalize both sides the same way. `to_83`
                // upper-cases ASCII; lower-case the expected name too so the
                // comparison is consistent regardless of on-disk casing.
                if e.is_valid() && normalize_name(&e.raw_name) == normalize_name(&want) {
                    return Some(e);
                }
            }
        }
        None
    }

    /// Read the full contents of a file into `out`, following its FAT chain.
    ///
    /// Returns the number of bytes written (which will be `min(out.len(), size)`).
    pub fn read_file<F>(&self, mut read: F, entry: &DirEntry, out: &mut [u8]) -> usize
    where
        F: FnMut(usize, &mut [u8]) -> usize,
    {
        if entry.first_cluster < 2 || entry.size == 0 {
            return 0;
        }
        let mut cluster = entry.first_cluster;
        let mut written = 0usize;
        // Guard against corrupt chains (infinite loops).
        for _ in 0..(entry.size as usize / SECTOR_SIZE + 2) {
            if cluster < 2 || cluster >= 0xFFF8 {
                break;
            }
            // LESSON (caller buffer): the contract says we return
            // `min(out.len(), size)`. An earlier revision sized `copy` from
            // `size` alone, so a caller passing a buffer SMALLER than the file
            // triggered a slice-index panic (debug) or memory corruption
            // (release). Never trust the on-disk length more than the buffer
            // you were given: stop when `out` is full.
            if written >= out.len() {
                break;
            }
            let sector = self.cluster_to_sector(cluster);
            let mut buf = [0u8; SECTOR_SIZE];
            if read(sector as usize, &mut buf) == 0 {
                break;
            }
            let remaining = (entry.size as usize - written).min(out.len() - written);
            if remaining == 0 {
                break;
            }
            let copy = core::cmp::min(SECTOR_SIZE, remaining);
            out[written..written + copy].copy_from_slice(&buf[..copy]);
            written += copy;

            match self.read_fat_entry(&mut read, cluster) {
                Some(next) if next >= 0xFFF8 || next == 0xFFFF => break, // end / bad chain
                Some(next) => cluster = next,
                None => break,
            }
        }
        written
    }
}

/// Lowercase a name and strip trailing spaces, for case-insensitive 8.3 compare.
fn normalize_name(raw: &[u8; 11]) -> [u8; 11] {
    let mut out = [0u8; 11];
    let mut i = 0;
    while i < 11 {
        let c = raw[i];
        out[i] = if c >= b'A' && c <= b'Z' { c + (b'a' - b'A') } else { c };
        i += 1;
    }
    out
}

/// Compare two raw 8.3 names case-insensitively.
///
/// FAT16 stores names in upper case on most tools, but the on-disk bytes are
/// untrusted input: a volume written by another tool may contain lower-case
/// names. The storage server therefore compares names the same way the shell
/// upper-cases them, so `cat readme` finds `README` on either kind of image.
pub fn names_equal(a: &[u8; 11], b: &[u8; 11]) -> bool {
    normalize_name(a) == normalize_name(b)
}

/// Convert a user-supplied name to the padded, upper-cased 8.3 on-disk form.
fn to_83(name: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    let bytes = name.as_bytes();
    if let Some(dot) = bytes.iter().position(|&b| b == b'.') {
        let (base, ext) = bytes.split_at(dot);
        let ext = &ext[1..]; // drop the dot
        let n = core::cmp::min(base.len(), 8);
        out[..n].copy_from_slice(&base[..n]);
        let m = core::cmp::min(ext.len(), 3);
        out[8..8 + m].copy_from_slice(&ext[..m]);
    } else {
        let n = core::cmp::min(bytes.len(), 8);
        out[..n].copy_from_slice(&bytes[..n]);
    }
    // Store upper-case (as most FAT tools do for ASCII).
    for b in out.iter_mut() {
        if *b >= b'a' && *b <= b'z' {
        *b -= b'a' - b'A';
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal FAT16 image in memory for parser unit tests, writing into
    /// a caller-provided stack buffer (no heap / no `Vec`, so this compiles under
    /// `#![no_std]`). Returns the number of bytes written.
    ///
    /// Layout: sector 0 = boot sector (BPB); FAT(s) of `fat_size` sectors; root
    /// dir of `root_entries` entries; then the data area holding the file's
    /// clusters (chained 2 -> 3 -> ... -> end-of-chain, so multi-sector files
    /// exercise FAT-chain following, not just single-cluster reads).
    ///
    /// LESSON (test honesty): this builder once used the SAME wrong offsets as
    /// the parser (root dir after its own sectors, cluster at 0x1C, size at
    /// 0x1E). Parser and fixture agreed with each other — and disagreed with
    /// every real FAT16 image ever formatted. A fixture must encode the SPEC
    /// (see the layout comments in `Fat16::new` / `root_entry`), never mirror
    /// the implementation under test.
    fn make_image(
        img: &mut [u8],
        root_entries: u16,
        file_bytes: &[u8],
        fat_size: u16,
    ) -> usize {
        let bps = 512u32;
        let spc = 1u32; // one sector per cluster (simplest)
        let reserved = 1u32;
        let num_fats = 2u8;
        let root_dir_sectors = ((root_entries as u32) * 32 + bps - 1) / bps;
        // Root directory starts after the FAT(s); data area starts after the root dir.
        let rd_sector = reserved + num_fats as u32 * fat_size as u32;
        let data_start = rd_sector + root_dir_sectors;

        // One cluster per sector; clusters are numbered from 2.
        let clusters = ((file_bytes.len() as u32) + bps - 1) / bps;
        let total = (data_start + clusters * spc) as usize * SECTOR_SIZE;
        assert!(total <= img.len(), "stack buffer too small for image");
        img[..total].fill(0);

        // BPB.
        let w16 = |b: &mut [u8], off: u16, v: u16| {
            b[off as usize] = v as u8;
            b[(off + 1) as usize] = (v >> 8) as u8;
        };
        let w32 = |b: &mut [u8], off: u16, v: u32| {
            b[off as usize..(off as usize) + 4].copy_from_slice(&v.to_le_bytes());
        };
        w16(&mut img[0..], 0x0B, bps as u16); // bytes per sector
        img[0x0D] = spc as u8; // sectors per cluster
        w16(&mut img[0..], 0x0E, reserved as u16); // reserved sectors
        img[0x10] = num_fats; // number of FATs
        w16(&mut img[0..], 0x11, root_entries); // root dir entry count
        img[0x15] = 0xF8; // media descriptor
        w16(&mut img[0..], 0x16, fat_size); // sectors per FAT
        // Total sectors (32-bit) lives at BPB offset 0x20 — NOT 0x1E (that is
        // the middle of the hidden-sectors field; writing there corrupts it).
        w32(&mut img[0..], 0x20, total as u32 / bps as u32);
        img[510] = 0x55;
        img[511] = 0xAA;

        // Root directory: one file entry at index 0. FAT names are padded to 8
        // chars and the extension field (bytes 8..11) is space-filled for files
        // without an extension, so build the full 11-byte name that way.
        let rd_off = (rd_sector as usize) * SECTOR_SIZE;
        let mut fname = [b' '; 11];
        fname[..5].copy_from_slice(b"HELLO");
        img[rd_off..rd_off + 11].copy_from_slice(&fname);
        img[rd_off + 0x0A] = 0x20; // archive attribute
        // First cluster (u16, little-endian) at 0x1A; size (u32) at 0x1C.
        img[rd_off + 0x1A] = 2; // first cluster = 2 (first data cluster)
        img[rd_off + 0x1B] = 0;
        img[rd_off + 0x1C..rd_off + 0x20].copy_from_slice(&(file_bytes.len() as u32).to_le_bytes());

        // FAT chain: clusters 2..2+clusters-1 linked in order, last one marked
        // end-of-chain (0xFFF8); everything else free (0x0000).
        let fat_sector = reserved as usize;
        for c in 0..clusters {
            let entry_off = fat_sector * SECTOR_SIZE + (2 + c) as usize * 2;
            let next = if c + 1 == clusters { 0xFFF8u16 } else { 2 + c as u16 + 1 };
            img[entry_off] = next as u8;
            img[entry_off + 1] = (next >> 8) as u8;
        }

        // File data, one cluster (sector) per link of the chain.
        for c in 0..clusters {
            let from = c as usize * bps as usize;
            let to = core::cmp::min(from + bps as usize, file_bytes.len());
            let file_sector = (data_start + c) as usize * SECTOR_SIZE;
            img[file_sector..file_sector + (to - from)].copy_from_slice(&file_bytes[from..to]);
        }

        total
    }

    fn read_mem<'a>(img: &'a [u8]) -> impl FnMut(usize, &mut [u8]) -> usize + 'a {
        move |sector: usize, buf: &mut [u8]| {
            let start = sector * SECTOR_SIZE;
            // Past-the-end reads yield 0 bytes (like a real device would):
            // without this guard `img.len() - start` underflows on `usize`.
            if start >= img.len() {
                return 0;
            }
            let n = core::cmp::min(SECTOR_SIZE, img.len() - start);
            buf[..n].copy_from_slice(&img[start..start + n]);
            n
        }
    }

    #[test]
    fn parses_bpb_and_reads_file() {
        let content = b"Hello from FAT16!\n";
        let mut img = [0u8; 4096];
        make_image(&mut img, 8, content, 1);
        let fat = Fat16::new(&img[0..512]).expect("valid BPB");
        assert_eq!(fat.bytes_per_sector, 512);

        let mut entry = None;
        for i in 0..fat.root_dir_entry_count as usize {
            if let Some(e) = fat.root_entry(&mut read_mem(&img), i) {
                assert!(e.is_valid(), "entry {} not valid", i);
                assert_eq!(normalize_name(&e.raw_name), normalize_name(&to_83("hello")),
                    "name mismatch at idx {}", i);
                entry = Some(e);
                break;
            }
        }
        let entry = entry.expect("file found");
        assert_eq!(entry.size, content.len() as u32);

        let mut out = [0u8; 64];
        let n = fat.read_file(&mut read_mem(&img), &entry, &mut out);
        assert_eq!(&out[..n], &content[..]);
    }

    #[test]
    fn rejects_non_fat_image() {
        let img = [0u8; 512];
        assert!(Fat16::new(&img).is_none());
    }

    #[test]
    fn root_dir_starts_immediately_after_fats() {
        // Pins the ABSOLUTE on-disk layout: with reserved=1 sector and 2 FATs
        // of 1 sector each, the root directory MUST live at sector 3 — not at
        // sector 4 (the old double-counted formula). This is the test that
        // would have caught the geometry bug: it checks raw sector contents,
        // independent of the parser's own arithmetic.
        let content = b"Hello from FAT16!\n";
        let mut img = [0u8; 4096];
        make_image(&mut img, 8, content, 1);
        // Sector 3, first 5 bytes: the "HELLO" name of our root entry.
        assert_eq!(&img[3 * SECTOR_SIZE..3 * SECTOR_SIZE + 5], b"HELLO");
        // And the parser must agree: entry 0 is valid and points at cluster 2.
        let fat = Fat16::new(&img[0..512]).expect("valid BPB");
        let e = fat.root_entry(&mut read_mem(&img), 0).expect("entry 0");
        assert!(e.is_valid());
        assert_eq!(e.first_cluster, 2);
        assert_eq!(e.size, content.len() as u32);
    }

    #[test]
    fn reads_multi_cluster_file() {
        // A 700-byte file spans 2 clusters (512 + 188): exercises FAT-chain
        // following (cluster 2 -> 3 -> end) rather than single-cluster reads.
        let mut content = [0u8; 700];
        for (i, b) in content.iter_mut().enumerate() {
            *b = (i % 251) as u8; // non-trivial pattern, catches misordering
        }
        let mut img = [0u8; 8192];
        make_image(&mut img, 8, &content, 1);
        let fat = Fat16::new(&img[0..512]).expect("valid BPB");
        let entry = fat.find(&mut read_mem(&img), "hello").expect("file found");
        let mut out = [0u8; 1024];
        let n = fat.read_file(&mut read_mem(&img), &entry, &mut out);
        assert_eq!(n, content.len());
        assert_eq!(&out[..n], &content[..]);
    }

    

    #[test]
    fn raw_name_comparison_is_case_insensitive() {
        // The shell upper-cases the name it sends; the volume may store it in
        // either case. Both must match.
        let upper = *b"README     ";
        let lower = *b"readme     ";
        assert!(names_equal(&upper, &lower));
        assert!(names_equal(&upper, &upper));
        assert!(!names_equal(&upper, b"HELLO      "));
    }

    #[test]
    fn read_file_clamps_to_output_buffer() {

        // A caller buffer SMALLER than the file must yield `out.len()` bytes,
        // not a slice-index panic: the on-disk size is untrusted input.
        let mut content = [0u8; 700];
        for (i, b) in content.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let mut img = [0u8; 8192];
        make_image(&mut img, 8, &content, 1);
        let fat = Fat16::new(&img[0..512]).expect("valid BPB");
        let entry = fat.find(&mut read_mem(&img), "hello").expect("file found");
        let mut out = [0u8; 100];
        let n = fat.read_file(&mut read_mem(&img), &entry, &mut out);
        assert_eq!(n, out.len());
        assert_eq!(&out[..n], &content[..n]);
    }
}
