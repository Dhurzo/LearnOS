//! ELF Loader

use core::mem::size_of;

pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
pub const ELFCLASS64: u8 = 2;
pub const ELFDATA2LSB: u8 = 1;
pub const EV_CURRENT: u8 = 1;
pub const ET_EXEC: u16 = 2;
pub const EM_X86_64: u16 = 0x3E;
pub const PT_LOAD: u32 = 1;

#[repr(C)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

impl Elf64Ehdr {
    pub fn is_valid(&self) -> bool {
        self.e_ident[0..4] == ELF_MAGIC
            && self.e_ident[4] == ELFCLASS64
            && self.e_ident[5] == ELFDATA2LSB
            && self.e_ident[6] == EV_CURRENT
            && self.e_type == ET_EXEC
            && self.e_machine == EM_X86_64
    }
}

#[repr(C)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

pub unsafe fn load_elf(data: &[u8], load_addr: u64) -> Option<LoadedProgram> {
    if size_of::<Elf64Ehdr>() > data.len() {
        return None;
    }
    let hdr = &*(data.as_ptr() as *const Elf64Ehdr);
    if !hdr.is_valid() {
        return None;
    }
    let entry = hdr.e_entry;
    let phoff = hdr.e_phoff as usize;
    let phnum = hdr.e_phnum as usize;
    let phentsize = hdr.e_phentsize as usize;
    let mut max_vaddr: u64 = 0;
    let mut mem_size: u64 = 0;
    for i in 0..phnum {
        let offset = phoff + i * phentsize;
        if offset + phentsize > data.len() {
            continue;
        }
        let phdr = &*(data.as_ptr().add(offset) as *const Elf64Phdr);
        if phdr.p_type != PT_LOAD {
            continue;
        }
        let vaddr = phdr.p_vaddr;
        let memsz = phdr.p_memsz;
        let end = vaddr.saturating_add(memsz);
        if end > max_vaddr {
            max_vaddr = end;
        }
        mem_size = mem_size.max(max_vaddr);
    }
    for i in 0..phnum {
        let offset = phoff + i * phentsize;
        if offset + phentsize > data.len() {
            continue;
        }
        let phdr = &*(data.as_ptr().add(offset) as *const Elf64Phdr);
        if phdr.p_type != PT_LOAD {
            continue;
        }
        let file_offset = phdr.p_offset as usize;
        let file_size = phdr.p_filesz as usize;
        let vaddr = phdr.p_vaddr;
        let dest = load_addr.wrapping_add(vaddr);
        if file_offset + file_size <= data.len() {
            let src = data.as_ptr().add(file_offset);
            let dst = dest as *mut u8;
            core::ptr::copy_nonoverlapping(src, dst, file_size);
        }
        let bss_start = (dest as usize + file_size) as *mut u8;
        let bss_size = (phdr.p_memsz as usize).saturating_sub(file_size);
        if bss_size > 0 {
            core::ptr::write_bytes(bss_start, 0, bss_size);
        }
    }
    Some(LoadedProgram {
        entry: load_addr.wrapping_add(entry),
        load_addr,
        mem_size,
    })
}

pub struct LoadedProgram {
    pub entry: u64,
    pub load_addr: u64,
    pub mem_size: u64,
}
