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
    NotExecutable,
    NoLoadSegments,
    OutOfMemory,
    SegmentOverlap,
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

    // Parse ELF header
    let hdr: &ElfHeader = unsafe { &*(data.as_ptr() as *const ElfHeader) };

    // Validate
    if hdr.magic != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if hdr.class != 2 {
        // 2 = 64-bit
        return Err(ElfError::Not64Bit);
    }
    if hdr.e_type != 2 {
        // 2 = ET_EXEC
        return Err(ElfError::NotExecutable);
    }

    let phoff = hdr.e_phoff as usize;
    let phentsize = hdr.e_phentsize as usize;
    let phnum = hdr.e_phnum as usize;

    if phoff == 0 || phentsize < 56 || phnum == 0 {
        return Err(ElfError::NoLoadSegments);
    }
    if phoff + phnum * phentsize > data.len() {
        return Err(ElfError::NoLoadSegments);
    }

    let entry = hdr.e_entry;

    // Process each program header
    for i in 0..phnum {
        let phdr_ptr = data.as_ptr().add(phoff + i * phentsize) as *const ProgramHeader;
        let phdr: &ProgramHeader = unsafe { &*phdr_ptr };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        let vaddr = phdr.p_vaddr & !0xFFF; // Page-align start
        let end = (phdr.p_vaddr + phdr.p_memsz + 0xFFF) & !0xFFF;
        let file_offset = phdr.p_offset;
        let file_size = phdr.p_filesz as usize;
        let mem_size = (end - vaddr) as usize;

        // Map each page in the segment
        let mut current_vaddr = vaddr;
        while current_vaddr < end {
            let frame = crate::paging::alloc_frame();
            if frame == 0 {
                return Err(ElfError::OutOfMemory);
            }

            // Zero the whole frame first
            core::ptr::write_bytes(frame as *mut u8, 0, 4096);

            // Copy data from the ELF image
            let page_offset = (current_vaddr - vaddr) as usize;
            let copy_start = (file_offset as usize).saturating_sub(page_offset);
            let copy_len = (file_size as usize).saturating_sub(page_offset).min(4096);
            if copy_len > 0 && copy_start + copy_len <= data.len() {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(copy_start),
                    frame as *mut u8,
                    copy_len,
                );
            }

            // Determine page flags
            let mut flags = crate::paging::page_flags::PRESENT
                | crate::paging::page_flags::USER_ACCESS;
            if phdr.p_flags & 2 != 0 {
                // PF_W
                flags |= crate::paging::page_flags::WRITABLE;
            }
            // PF_R (bit 2) is implied by PRESENT

            // Map the page
            crate::paging::map_phys_page(pml4_phys, current_vaddr, frame, flags);

            current_vaddr += 4096;
        }
    }

    Ok(LoadResult {
        entry,
        stack_top: crate::paging::USER_STACK_VADDR,
    })
}
