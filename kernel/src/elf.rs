//! Minimal ELF Loader (Phase 3 — User-Space ELF Loader + Block Driver)
//!
//! Parses ELF-64 program headers and maps LOAD segments into a process's
//! address space.  This is a *kernel-side* loader for now; a fully
//! microkernel design would move it to user space.
//!
//! ELF64 header structures (simplified, no external crate dependency).

/// ELF magic: \x7f E L F
pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// ELF64 file header (52 bytes, but we only read the first 64).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ElfHeader {
    pub magic: [u8; 4],
    pub class: u8,          // 1 = 32-bit, 2 = 64-bit
    pub encoding: u8,       // 1 = little, 2 = big
    pub version: u8,
    pub osabi: u8,
    pub padding: [u8; 8],
    pub e_type: u16,        // 2 = executable
    pub e_machine: u16,     // 0x3E = x86-64
    pub e_version: u32,
    pub e_entry: u64,       // Entry point virtual address
    pub e_phoff: u64,       // Program header offset
    pub e_shoff: u64,       // Section header offset (unused)
    pub e_flags: u32,
    pub e_ehsize: u16,      // ELF header size
    pub e_phentsize: u16,   // Program header entry size
    pub e_phnum: u16,       // Number of program headers
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// ELF64 program header (56 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProgramHeader {
    pub p_type: u32,        // 1 = LOAD
    pub p_flags: u32,       // PF_X=1, PF_W=2, PF_R=4
    pub p_offset: u64,      // Offset in file
    pub p_vaddr: u64,       // Virtual address to load at
    pub p_paddr: u64,       // Physical address (unused)
    pub p_filesz: u64,      // Size in file
    pub p_memsz: u64,       // Size in memory (may be > filesz for .bss)
    pub p_align: u64,       // Alignment
}

/// LOAD segment type.
pub const PT_LOAD: u32 = 1;

/// Errors that can occur during ELF loading.
#[derive(Debug)]
pub enum ElfError {
    BadMagic,
    Not64Bit,
    BadEncoding,
    BadMachine,
    NotExecutable,
    NoLoadSegments,
    OutOfMemory,
    SegmentOverlap,
    BadSegment,
    BadAddress,
}

/// Result of loading an ELF into a page table.
pub struct LoadResult {
    pub entry: u64,
    pub stack_top: u64,
}

/// Load an ELF binary into the given page table.
///
/// `data` must be the complete ELF file image in memory.
/// `pml4_phys` is the physical address of the process's PML4.
///
/// Returns the entry point and a suggested stack top on success.
///
/// # Safety
///
/// `data` must point to valid, complete ELF data.
/// `pml4_phys` must be a valid PML4 frame.
pub unsafe fn load_elf(data: &[u8], pml4_phys: u64) -> Result<LoadResult, ElfError> {
    if data.len() < 64 {
        return Err(ElfError::BadMagic);
    }

    // ELF bytes may start at an address with alignment 1 (e.g. include_bytes!).
    // Copy the fixed-size header rather than constructing an aligned reference
    // into an arbitrary byte slice.
    let hdr = unsafe { core::ptr::read_unaligned(data.as_ptr() as *const ElfHeader) };

    // Validate
    if hdr.magic != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if hdr.class != 2 {
        // 2 = 64-bit
        return Err(ElfError::Not64Bit);
    }
    if hdr.encoding != 1 || hdr.version != 1 {
        return Err(ElfError::BadEncoding);
    }
    if hdr.e_machine != 0x3E || hdr.e_version != 1 {
        return Err(ElfError::BadMachine);
    }
    if hdr.e_type != 2 {
        // 2 = ET_EXEC
        return Err(ElfError::NotExecutable);
    }

    let phoff = usize::try_from(hdr.e_phoff).map_err(|_| ElfError::NoLoadSegments)?;
    let phentsize = hdr.e_phentsize as usize;
    let phnum = hdr.e_phnum as usize;

    if hdr.e_ehsize < 64 || phoff == 0 || phentsize < 56 || phnum == 0 {
        return Err(ElfError::NoLoadSegments);
    }
    let ph_table_size = phnum.checked_mul(phentsize).ok_or(ElfError::NoLoadSegments)?;
    let ph_table_end = phoff.checked_add(ph_table_size).ok_or(ElfError::NoLoadSegments)?;
    if ph_table_end > data.len() {
        return Err(ElfError::NoLoadSegments);
    }

    let entry = hdr.e_entry;
    let mut found_load = false;
    let mut entry_is_executable = false;

    // Process each program header
    for i in 0..phnum {
        let phdr_offset = phoff + i * phentsize;
        let phdr_ptr = data.as_ptr().add(phdr_offset) as *const ProgramHeader;
        let phdr = unsafe { core::ptr::read_unaligned(phdr_ptr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }
        found_load = true;

        if phdr.p_filesz > phdr.p_memsz {
            return Err(ElfError::BadSegment);
        }

        let file_offset = usize::try_from(phdr.p_offset).map_err(|_| ElfError::BadSegment)?;
        let file_size = usize::try_from(phdr.p_filesz).map_err(|_| ElfError::BadSegment)?;
        let file_end = file_offset.checked_add(file_size).ok_or(ElfError::BadSegment)?;
        if file_end > data.len() {
            return Err(ElfError::BadSegment);
        }

        if phdr.p_memsz == 0 {
            continue;
        }
        let segment_end = phdr.p_vaddr.checked_add(phdr.p_memsz).ok_or(ElfError::BadAddress)?;
        let vaddr = phdr.p_vaddr & !0xFFF;
        let end = segment_end.checked_add(0xFFF).ok_or(ElfError::BadAddress)? & !0xFFF;
        if vaddr < crate::paging::USER_VADDR_LOAD
            || end > crate::paging::USER_ELF_VADDR_END
            || end <= vaddr
        {
            return Err(ElfError::BadAddress);
        }
        if phdr.p_flags & 1 != 0 && entry >= phdr.p_vaddr && entry < segment_end {
            entry_is_executable = true;
        }

        // Map each page and copy only the intersection of this page with the
        // segment's file-backed bytes. The virtual/file offsets need not be
        // page-aligned; zeroed frames provide the remaining BSS bytes.
        let mut current_vaddr = vaddr;
        while current_vaddr < end {
            let frame = crate::paging::alloc_frame();
            if frame == 0 {
                return Err(ElfError::OutOfMemory);
            }

            let file_vaddr_end = phdr.p_vaddr + phdr.p_filesz;
            let copy_vaddr_start = core::cmp::max(current_vaddr, phdr.p_vaddr);
            let copy_vaddr_end = core::cmp::min(current_vaddr + 4096, file_vaddr_end);
            if copy_vaddr_end > copy_vaddr_start {
                let copy_start = file_offset + (copy_vaddr_start - phdr.p_vaddr) as usize;
                let copy_len = (copy_vaddr_end - copy_vaddr_start) as usize;
                let dest_offset = (copy_vaddr_start - current_vaddr) as usize;
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(copy_start),
                    (frame as *mut u8).add(dest_offset),
                    copy_len,
                );
            }

            let mut flags = crate::paging::page_flags::PRESENT
                | crate::paging::page_flags::USER_ACCESS;
            if phdr.p_flags & 2 != 0 {
                flags |= crate::paging::page_flags::WRITABLE;
            }
            if phdr.p_flags & 1 == 0 {
                flags |= crate::paging::page_flags::EXECUTABLE_DISABLE;
            }

            if !crate::paging::map_phys_page(pml4_phys, current_vaddr, frame, flags) {
                crate::paging::free_frame(frame);
                return Err(ElfError::OutOfMemory);
            }

            current_vaddr += 4096;
        }
    }

    if !found_load {
        return Err(ElfError::NoLoadSegments);
    }
    if !entry_is_executable {
        return Err(ElfError::BadAddress);
    }

    Ok(LoadResult {
        entry,
        stack_top: crate::paging::USER_STACK_VADDR,
    })
}
