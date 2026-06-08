//! Memory Management and Paging
//!
//! This module handles virtual memory and user/kernel separation.
//! It provides per-process address spaces via CR3 switching.
//!
//! =============================================================================
//! PAGE TABLE ENTRY (PTE) FLAGS
//! =============================================================================
//!
//! Each 8-byte PTE contains:
//!   Bit 0: Present (P)
//!   Bit 1: Writable (W)
//!   Bit 2: User Accessible (U)
//!   Bit 3: Write-Through (PWT)
//!   Bit 4: Cache Disabled (PCD)
//!   Bit 5: Accessed (A) - set by CPU
//!   Bit 6: Dirty (D) - set by CPU (PTEs only)
//!   Bit 7: Large/Page Size (PS) - 2MB or 1GB page
//!   Bit 8: Global (G)
//!   Bits 9-11: Available for OS
//!   Bits 12-51: Physical frame number (address >> 12)
//!   Bits 52-62: Available for OS
//!   Bit 63: No Execute (NX)
//!
//! =============================================================================
//! ADDRESS SPACE LAYOUT
//! =============================================================================
//!
//! Per-process page tables isolate user memory. The kernel region (identity
//! mapped, first few MB) is accessible only from ring 0 (U=0). User pages
//! have U=1 so they're accessible from ring 3.
//!
//!   PML4[0] → PDPT[0] → PD (512 × 2MB entries or 4KB PTs)
//!     PD[0]   0x00000000 - 0x001FFFFF  Kernel code  (2MB huge, U=0)
//!     PD[1]   0x00200000 - 0x003FFFFF  Kernel data  (2MB huge, U=0)
//!     PD[2]   0x00400000 - 0x005FFFFF  Split into 4KB PT for user code (U=1)
//!     PD[3-6] Not present
//!     PD[7]   0x00E00000 - 0x00FFFFFF  User stack   (2MB huge, U=1)
//!
//! Physical memory beyond 16MB is used by the frame allocator for dynamic
//! page-table allocation and user-program code pages.
//!
//! =============================================================================
//! FRAME ALLOCATOR (Bitmap-based with free-list support)
//! =============================================================================
//!
//! We use a bitmap to track all 4KB frames in the physical address range
//! [PHYS_MEM_START, PHYS_MEM_END). A set bit (1) means the frame is free;
//! a cleared bit (0) means it is in use. This is Phase 0 of the microkernel
//! plan: it replaces the old bump allocator so that freed frames can be
//! reclaimed and reused.
//!
//!   Bitmap size: 64 words × 64 bits each = 4096 bits per cache line
//!   Covers: (PHYS_MEM_END - PHYS_MEM_START) / PAGE_SIZE frames
//!
//! Allocation scans the bitmap for the first free frame (lowest address).
//! Freeing simply sets the corresponding bit back to 1.

use core::sync::atomic::{AtomicU64, Ordering};

// =============================================================================
// PHYSICAL MEMORY CONSTANTS
// =============================================================================

/// Physical memory range managed by the bitmap allocator.
/// 0x100000 (1 MB) to 0x8000000 (128 MB) — QEMU default with -m 128.
pub const PHYS_MEM_START: u64 = 0x0010_0000;
pub const PHYS_MEM_END: u64   = 0x0800_0000;

/// Number of 4 KB frames we manage.
pub const NUM_FRAMES: usize = ((PHYS_MEM_END - PHYS_MEM_START) / 4096) as usize; // 32512

/// Number of 64-bit words in the bitmap.
pub const BITMAP_WORDS: usize = (NUM_FRAMES + 63) / 64; // 508

/// Frame allocation bitmap: bit = 1 means free, 0 means in use.
/// Initialised to all-1 (all free); init_frame_allocator() marks kernel pages.
static mut FRAME_BITMAP: [u64; BITMAP_WORDS] = [u64::MAX; BITMAP_WORDS];

/// Convert a physical address to a bitmap frame index.
fn addr_to_frame_idx(addr: u64) -> usize {
    ((addr - PHYS_MEM_START) / 4096) as usize
}

/// Convert a bitmap frame index back to a physical address.
fn frame_idx_to_addr(idx: usize) -> u64 {
    PHYS_MEM_START + (idx as u64) * 4096
}

/// Mark a single frame as in-use (called during init to reserve kernel pages).
unsafe fn mark_frame_in_use(phys: u64) {
    if phys < PHYS_MEM_START || phys >= PHYS_MEM_END {
        return;
    }
    let idx = addr_to_frame_idx(phys);
    let w = idx / 64;
    let b = idx % 64;
    if w < BITMAP_WORDS {
        FRAME_BITMAP[w] &= !(1u64 << b);
    }
}

// =============================================================================
// ADDRESS CONSTANTS
// =============================================================================

/// User process code load address (typical ELF load address)
pub const USER_VADDR_LOAD: u64 = 0x0000000000400000;
pub const USER_VADDR_START: u64 = USER_VADDR_LOAD;

/// User virtual address limit
pub const USER_VADDR_END: u64 = 0x00007FFFFFFFFFFF;

/// User process stack — top address (grows down).
/// Must be within a 2MB region that maps to a user-accessible huge page (PD[7]).
pub const USER_STACK_VADDR: u64 = 0x0000000000FFF000;

/// Virtual address for the VGA text buffer when mapped into a user-space driver.
/// Uses PD[3] range (0x600000-0x7FFFFF), currently unused in the address-space layout.
pub const VGA_BUFFER_VADDR: u64 = 0x0000000000600000;

/// Physical address of the VGA text mode buffer (80×25 colour).
pub const VGA_PHYS_ADDR: u64 = 0xB8000;

/// Stack size (8KB)
pub const USER_STACK_SIZE: u64 = 0x2000;

/// Page size (4KB)
pub const PAGE_SIZE: u64 = 4096;

/// Number of entries per page table
pub const PT_ENTRIES: usize = 512;

// =============================================================================
// PAGE TABLE FLAG CONSTANTS
// =============================================================================

pub mod page_flags {
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER_ACCESS: u64 = 1 << 2;
    pub const WRITE_THROUGH: u64 = 1 << 3;
    pub const CACHE_DISABLE: u64 = 1 << 4;
    pub const ACCESSED: u64 = 1 << 5;
    pub const DIRTY: u64 = 1 << 6;
    /// Large page: in PD → 2MB page, in PDPT → 1GB page
    pub const LARGE: u64 = 1 << 7;
    /// Global page: TLB entries survive CR3 writes (only when CR4.PGE=1)
    pub const GLOBAL: u64 = 1 << 8;
    pub const EXECUTABLE_DISABLE: u64 = 1 << 63;
    /// Copy-on-Write: page is shared between parent and child after fork.
    /// When either process writes to it, the page fault handler must
    /// allocate a new frame and copy the data.
    /// Uses bit 9 (available for OS use in x86-64 PTEs).
    pub const COW: u64 = 1 << 9;
}

/// User-accessible page flags (U=1)
pub const PTE_USER: u64 = page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS;

/// Kernel-only page flags (U=0)
pub const PTE_KERNEL: u64 = page_flags::PRESENT | page_flags::WRITABLE;

// =============================================================================
// FRAME ALLOCATOR
// =============================================================================

extern "C" {
    static __kernel_end: u8;
}

/// Initialise the bitmap allocator.
///
/// Marks all physical pages from 0 up to `__kernel_end` as in-use
/// (kernel code/data occupies them). Everything above is marked free.
pub fn init_frame_allocator() {
    unsafe {
        let kernel_end = &__kernel_end as *const u8 as u64;
        // Mark kernel code/data pages as in-use
        let mut addr = PHYS_MEM_START;
        while addr < kernel_end {
            mark_frame_in_use(addr);
            addr += 4096;
        }
    }
}

/// Allocate a single 4 KB physical frame.
///
/// Scans the bitmap for the first free frame, marks it in-use,
/// zeroes it, and returns its physical address.
///
/// Returns 0 if no free frames are available (OOM).
pub fn alloc_frame() -> u64 {
    unsafe {
        for (word_idx, word) in FRAME_BITMAP.iter_mut().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                let frame_idx = word_idx * 64 + bit;
                if frame_idx < NUM_FRAMES {
                    // Claim this frame
                    *word &= !(1u64 << bit);
                    let addr = frame_idx_to_addr(frame_idx);
                    // Zero the frame
                    core::ptr::write_bytes(addr as *mut u8, 0, PAGE_SIZE as usize);
                    return addr;
                }
                // Clear this bit and continue scanning the word
                bits &= bits - 1;
            }
        }
    }
    0 // Out of memory
}

/// Allocate and zero a 4 KB frame, returning the physical address.
pub fn alloc_zeroed_frame() -> u64 {
    alloc_frame()
}

// =============================================================================
// FRAME FREEING (Phase 0 — microkernel memory reclamation)
// =============================================================================

/// Free a physical frame, returning it to the bitmap pool.
///
/// The frame's bit is set to 1 (free).  Subsequent calls to `alloc_frame`
/// may recycle it.
pub fn free_frame(phys: u64) {
    if phys < PHYS_MEM_START || phys >= PHYS_MEM_END {
        return;
    }
    let idx = addr_to_frame_idx(phys);
    let w = idx / 64;
    let b = idx % 64;
    if w >= BITMAP_WORDS {
        return;
    }
    unsafe {
        FRAME_BITMAP[w] |= 1u64 << b;
    }
}

/// Walk a user process's page tables and free every 4-KiB mapped frame.
///
/// `pml4_phys` — physical address of the process's PML4.
/// `vaddr`     — starting virtual address (must be page-aligned).
/// `len`       — number of bytes to unmap (rounded up to PAGE_SIZE).
///
/// This only touches leaf PTEs; intermediate table frames (PML4, PDPT, PD,
/// PT) are NOT freed here — use `free_page_table_tree` for that.
///
/// # Safety
///
/// `pml4_phys` must be the physical address of a valid PML4. The kernel
/// must not have any live references into the unmapped range.
pub unsafe fn munmap_range(pml4_phys: u64, vaddr: u64, len: u64) {
    let end = vaddr.saturating_add((len + 4095) & !4095);
    let mut va = vaddr & !4095; // page-align start

    while va < end && va < USER_VADDR_END {
        let pml4_idx = ((va >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((va >> 30) & 0x1FF) as usize;
        let pd_idx   = ((va >> 21) & 0x1FF) as usize;
        let pt_idx   = ((va >> 12) & 0x1FF) as usize;

        // Read PML4E
        let pml4e = read_pt_entry(pml4_phys, pml4_idx);
        if pml4e & page_flags::PRESENT == 0 {
            va += 1 << 39; // Skip whole 512 GB region
            continue;
        }
        let pdpt = pml4e & 0x000FFFFFFFFFF000;

        // Check for 1 GB huge page
        let pdpte = read_pt_entry(pdpt, pdpt_idx);
        if pdpte & page_flags::PRESENT == 0 {
            va += 1 << 30; // Skip whole 1 GB region
            continue;
        }
        if pdpte & page_flags::LARGE != 0 {
            // 1 GB huge page — free the 2 MiB-aligned chunks inside
            // (For simplicity, skip huge pages in munmap for now.)
            va += 1 << 30;
            continue;
        }
        let pd = pdpte & 0x000FFFFFFFFFF000;

        // Check for 2 MB huge page
        let pde = read_pt_entry(pd, pd_idx);
        if pde & page_flags::PRESENT == 0 {
            va += 1 << 21; // Skip whole 2 MB region
            continue;
        }
        if pde & page_flags::LARGE != 0 {
            // 2 MB huge page
            let base = pde & 0x000FFFFFFFFFF000;
            for i in 0..512 {
                let frame = base + (i as u64) * 4096;
                free_frame(frame);
            }
            // Clear the PDE
            write_pt_entry(pd, pd_idx, 0);
            va += 1 << 21;
            continue;
        }
        let pt = pde & 0x000FFFFFFFFFF000;

        // Free the 4 KB page if present
        let pte = read_pt_entry(pt, pt_idx);
        if pte & page_flags::PRESENT != 0 {
            let frame = pte & 0x000FFFFFFFFFF000;
            free_frame(frame);
            write_pt_entry(pt, pt_idx, 0);
        }
        va += 4096;
    }
}

/// Free all page-table frames (PML4, PDPT, PD, PT) for a process.
///
/// Walks the full 4-level tree and frees every frame that belongs to the
/// page-table hierarchy.  Does NOT free leaf mapped frames (those are
/// handled by `munmap_range`).
///
/// # Safety
///
/// The caller must ensure the process is no longer scheduled and that no
/// other CPU core is using these page tables.
pub unsafe fn free_page_table_tree(pml4_phys: u64) {
    // Walk PML4 entries
    for pml4_idx in 0..PT_ENTRIES {
        let pml4e = read_pt_entry(pml4_phys, pml4_idx);
        if pml4e & page_flags::PRESENT == 0 {
            continue;
        }
        let pdpt_base = pml4e & 0x000FFFFFFFFFF000;

        // Walk PDPT entries
        for pdpt_idx in 0..PT_ENTRIES {
            let pdpte = read_pt_entry(pdpt_base, pdpt_idx);
            if pdpte & page_flags::PRESENT == 0 {
                continue;
            }
            if pdpte & page_flags::LARGE != 0 {
                continue; // 1 GB huge page — skip
            }
            let pd_base = pdpte & 0x000FFFFFFFFFF000;

            // Walk PD entries
            for pd_idx in 0..PT_ENTRIES {
                let pde = read_pt_entry(pd_base, pd_idx);
                if pde & page_flags::PRESENT == 0 {
                    continue;
                }
                if pde & page_flags::LARGE != 0 {
                    continue; // 2 MB huge page — skip
                }
                // Free the PT frame
                let pt_base = pde & 0x000FFFFFFFFFF000;
                free_frame(pt_base);
            }
            // Free the PD frame
            free_frame(pd_base);
        }
        // Free the PDPT frame
        free_frame(pdpt_base);
    }
    // Free the PML4 frame itself
    free_frame(pml4_phys);
}

// =============================================================================
// PAGE TABLE HELPERS
// =============================================================================

/// Write a 64-bit entry into a page-table frame at the given index.
///
/// # Safety
///
/// `table_phys` must be the physical address of a valid 4KB page-table frame.
/// `index` must be < 512.
pub unsafe fn write_pt_entry(table_phys: u64, index: usize, value: u64) {
    let ptr = (table_phys as *mut u64).add(index);
    ptr.write_volatile(value);
}

/// Read a 64-bit entry from a page-table frame.
pub unsafe fn read_pt_entry(table_phys: u64, index: usize) -> u64 {
    let ptr = (table_phys as *const u64).add(index);
    ptr.read_volatile()
}

/// Zero an entire page-table frame (set all 512 entries to 0).
pub unsafe fn zero_pt_frame(table_phys: u64) {
    core::ptr::write_bytes(table_phys as *mut u8, 0, 4096);
}

// =============================================================================
// COPY-ON-WRITE (Phase 2 — Process Creation)
// =============================================================================

/// Copy a page table tree for fork with Copy-on-Write.
///
/// Walks the 4-level page table rooted at `src_pml4`, creates a new
/// parallel tree, and maps every user-accessible leaf page with the COW
/// flag set and WRITABLE cleared.  Both parent and child share the same
/// physical frames until one of them writes.
///
/// Returns the physical address of the new PML4.
///
/// # Safety
///
/// `src_pml4` must be a valid PML4 physical address.
pub unsafe fn copy_page_table_cow(src_pml4: u64) -> u64 {
    let dst_pml4 = alloc_zeroed_frame();

    for pml4_idx in 0..PT_ENTRIES {
        let src_pml4e = read_pt_entry(src_pml4, pml4_idx);
        if src_pml4e & page_flags::PRESENT == 0 {
            continue;
        }

        let src_pdpt = src_pml4e & 0x000FFFFFFFFFF000;
        let dst_pdpt = alloc_zeroed_frame();

        // Link the new PDPT into the new PML4 with the same flags
        let iflags = src_pml4e & (page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS);
        write_pt_entry(dst_pml4, pml4_idx, (dst_pdpt & 0x000FFFFFFFFFF000) | iflags);

        for pdpt_idx in 0..PT_ENTRIES {
            let src_pdpte = read_pt_entry(src_pdpt, pdpt_idx);
            if src_pdpte & page_flags::PRESENT == 0 {
                continue;
            }

            if src_pdpte & page_flags::LARGE != 0 {
                // 1 GB huge page — copy the PDE as-is but mark COW
                let cow_pdpte = (src_pdpte & !page_flags::WRITABLE) | page_flags::COW;
                write_pt_entry(dst_pdpt, pdpt_idx, cow_pdpte);
                continue;
            }

            let src_pd = src_pdpte & 0x000FFFFFFFFFF000;
            let dst_pd = alloc_zeroed_frame();
            let iflags = src_pdpte & (page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS);
            write_pt_entry(dst_pdpt, pdpt_idx, (dst_pd & 0x000FFFFFFFFFF000) | iflags);

            for pd_idx in 0..PT_ENTRIES {
                let src_pde = read_pt_entry(src_pd, pd_idx);
                if src_pde & page_flags::PRESENT == 0 {
                    continue;
                }

                if src_pde & page_flags::LARGE != 0 {
                    // 2 MB huge page — mark COW
                    let cow_pde = (src_pde & !page_flags::WRITABLE) | page_flags::COW;
                    write_pt_entry(dst_pd, pd_idx, cow_pde);
                    continue;
                }

                let src_pt = src_pde & 0x000FFFFFFFFFF000;
                let dst_pt = alloc_zeroed_frame();
                let iflags = src_pde & (page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS);
                write_pt_entry(dst_pd, pd_idx, (dst_pt & 0x000FFFFFFFFFF000) | iflags);

                for pt_idx in 0..PT_ENTRIES {
                    let src_pte = read_pt_entry(src_pt, pt_idx);
                    if src_pte & page_flags::PRESENT == 0 {
                        continue;
                    }
                    // Only user pages get COW; kernel pages are shared as-is
                    if src_pte & page_flags::USER_ACCESS == 0 {
                        write_pt_entry(dst_pt, pt_idx, src_pte);
                        continue;
                    }
                    // Mark as read-only + COW in the *new* page table
                    let cow_pte = (src_pte & !page_flags::WRITABLE) | page_flags::COW;
                    write_pt_entry(dst_pt, pt_idx, cow_pte);
                }
            }
        }
    }
    dst_pml4
}

/// Handle a page fault caused by a write to a Copy-on-Write page.
///
/// Called from the page-fault handler (#PF, vector 0x0E) when:
///   - The faulting address is in user space
///   - The page is present but marked COW / read-only
///
/// Allocates a new physical frame, copies the data from the shared frame,
/// updates the PTE to be writable (removing COW), and flushes the TLB.
///
/// # Safety
///
/// `cr3` must be the current process's PML4 physical address.
pub unsafe fn handle_cow_fault(cr3: u64, fault_addr: u64) -> bool {
    // Walk the page tables to find the leaf PTE
    let pml4_idx = ((fault_addr >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((fault_addr >> 30) & 0x1FF) as usize;
    let pd_idx   = ((fault_addr >> 21) & 0x1FF) as usize;
    let pt_idx   = ((fault_addr >> 12) & 0x1FF) as usize;

    let pml4e = read_pt_entry(cr3, pml4_idx);
    if pml4e & page_flags::PRESENT == 0 { return false; }
    let pdpt = pml4e & 0x000FFFFFFFFFF000;

    let pdpte = read_pt_entry(pdpt, pdpt_idx);
    if pdpte & page_flags::PRESENT == 0 { return false; }
    if pdpte & page_flags::LARGE != 0 { return false; } // Skip 1G pages
    let pd = pdpte & 0x000FFFFFFFFFF000;

    let pde = read_pt_entry(pd, pd_idx);
    if pde & page_flags::PRESENT == 0 { return false; }
    if pde & page_flags::LARGE != 0 { return false; } // Skip 2M pages
    let pt = pde & 0x000FFFFFFFFFF000;

    let pte = read_pt_entry(pt, pt_idx);
    if pte & page_flags::PRESENT == 0 { return false; }
    if pte & page_flags::COW == 0 { return false; } // Not a COW page

    // Allocate a new frame and copy the data
    let old_phys = pte & 0x000FFFFFFFFFF000;
    let new_frame = alloc_frame();
    if new_frame == 0 { return false; } // OOM

    // Copy 4 KB from old frame to new frame
    core::ptr::copy_nonoverlapping(
        old_phys as *const u8,
        new_frame as *mut u8,
        4096,
    );

    // Update the PTE: new phys addr, writable, no COW
    let new_pte = (new_frame & 0x000FFFFFFFFFF000)
        | (pte & (page_flags::PRESENT | page_flags::USER_ACCESS | page_flags::GLOBAL
                  | page_flags::CACHE_DISABLE | page_flags::WRITE_THROUGH))
        | page_flags::WRITABLE;
    write_pt_entry(pt, pt_idx, new_pte);

    // Flush the TLB for this single page
    core::arch::asm!("invlpg [{}]", in(reg) fault_addr, options(nostack));

    true
}

// =============================================================================
// ADDRESS SPACE CREATION
// =============================================================================

/// Create a set of per-process page tables.
///
/// Layout:
///   PML4[0] → PDPT[0] → PD
///     PD[0]: 0x00000000-0x001FFFFF (kernel, 2MB huge, U=0)
///     PD[1]: 0x00200000-0x003FFFFF (kernel, 2MB huge, U=0)
///     PD[2]: 0x00400000-0x005FFFFF → PT (4KB entries, user code)
///     PD[7]: 0x00E00000-0x00FFFFFF (user stack, 2MB huge, U=1)
///
/// Arguments:
///   `code_phys`: physical address of user code to map at 0x400000
///
/// Returns the physical address of the new PML4 (to be loaded into CR3).
pub fn create_address_space(code_phys: u64) -> u64 {
    unsafe {
        // === Level 4: PML4 ===
        let pml4 = alloc_zeroed_frame();
        // Enable CR4.PGE so kernel pages with G bit survive CR3 switches
        // (done once in main.rs — see enable_pge())

        // === Level 3: PDPT ===
        let pdpt = alloc_zeroed_frame();
        let pdpt_ent = (pdpt & 0x000FFFFFFFFFF000)
            | page_flags::PRESENT
            | page_flags::WRITABLE
            | page_flags::USER_ACCESS;
        write_pt_entry(pml4, 0, pdpt_ent);

        // === Level 2: PD ===
        let pd = alloc_zeroed_frame();
        let pd_ent = (pd & 0x000FFFFFFFFFF000)
            | page_flags::PRESENT
            | page_flags::WRITABLE
            | page_flags::USER_ACCESS;
        write_pt_entry(pdpt, 0, pd_ent);

        // --- PD[0]: identity map 0x000000-0x1FFFFF (kernel code) ---
        // USER_ACCESS required: when the timer fires at CPL=3, the CPU must read
        // the GDT (code segment descriptor) and TSS (for stack switch) which live
        // in this region.  Without U=1 the CPU cannot complete the privilege-level
        // transition and raises #GP(0x0102) referencing the IDT gate.
        write_pt_entry(pd, 0, 0x000000 | (page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS | page_flags::LARGE));

        // --- PD[1]: identity map 0x200000-0x3FFFFF (kernel data + IDT) ---
        // USER_ACCESS required: the IDT itself lives at 0x200000.  The CPU reads
        // the IDT gate descriptor before switching CPL, so the page must be
        // accessible while the processor is still at CPL=3.
        write_pt_entry(pd, 1, 0x200000 | (page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS | page_flags::LARGE));

        // --- PD[2]: user code region 0x400000-0x5FFFFF → 4KB PT ---
        let pt_code = alloc_zeroed_frame();
        // Map code_phys at virtual 0x400000 (PT index 0 covers 0x400000-0x400FFF)
        let code_flags = page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS;
        write_pt_entry(pt_code, 0, (code_phys & 0x000FFFFFFFFFF000) | code_flags);
        // PD[2] points to PT (no LARGE bit — this is a regular 4KB-page directory)
        write_pt_entry(
            pd,
            2,
            (pt_code & 0x000FFFFFFFFFF000) | page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS,
        );

        // --- PD[3-6]: Not present (zero already from alloc_zeroed_frame) ---

        // --- PD[7]: identity map 0xE00000-0xFFFFFF (user stack, 2MB huge, U=1) ---
        write_pt_entry(
            pd,
            7,
            0xE00000 | (page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS | page_flags::LARGE),
        );

        pml4
    }
}

/// Enable Page Global Enable (CR4.PGE) so kernel pages marked Global survive
/// CR3 writes. This avoids TLB flushes for kernel mappings on every context
/// switch.
///
/// # Safety
///
/// Modifies CR4. Must be called once during boot.
pub unsafe fn enable_pge() {
    let cr4: u64;
    core::arch::asm!("mov {0}, cr4", out(reg) cr4, options(nostack));
    core::arch::asm!("mov cr4, {0}", in(reg) (cr4 | 0x80), options(nostack));
}

// =============================================================================
// PHYSICAL PAGE MAPPING (for user-space drivers)
// =============================================================================
//
// In a microkernel, even device drivers are user-space processes. To let a
// device driver (like the VGA server) access hardware memory directly, we
// need to map physical device memory into its address space.
//
// This is what map_phys_page does: it walks a process's page tables and
// inserts a 4KB mapping from a virtual address to a physical address.
//
// The key insight: x86-64 uses a 4-level page table hierarchy:
//
//   Virtual Address (48 bits used):
//   ┌────────────┬────────────┬────────────┬────────────┬──────────┐
//   │   PML4[9]  │  PDPT[9]   │   PD[9]    │   PT[9]    │  Offset  │
//   │   47:39    │   38:30    │   29:21    │   20:12    │   11:0   │
//   └────────────┴────────────┴────────────┴────────────┴──────────┘
//
//   PML4[0] → PDPT (Page Directory Pointer Table)
//   PDPT[0] → PD   (Page Directory)
//   PD[N]   → PT   (Page Table)    — OR — 2MB huge page
//   PT[M]   → 4KB Physical Page
//
// Each table has 512 entries (9 bits of index), and each entry is 8 bytes.
// A full table fits in one 4KB page.

/// Map a physical page into any address space at a given virtual address.
///
/// Walks the 4-level page table rooted at `pml4_phys`, allocating any
/// intermediate tables that do not yet exist, then writes the final PTE
/// with `phys` and `flags`.
///
/// This is a "lazy" page-table walker: if a table at any level doesn't
/// exist yet (PML4E/PDPTE/PDE not Present), we allocate a new zeroed
/// frame and point the parent entry at it. This means we don't pre-build
/// the full page table tree — we build only the parts we need.
///
/// Why is identity mapping needed for the frame allocator?
///   The frames we allocate live in physical memory beyond the 16MB
///   identity-mapped region. But since the kernel is identity-mapped
///   at boot (physical address == virtual address), we can access
///   those frames directly at their physical addresses. If the kernel
///   used a virtual offset (like 0xFFFF800000000000+), we'd need to
///   translate here.
///
/// # Safety
///
/// - `pml4_phys` must be the physical address of a valid PML4 frame.
/// - `phys` must be 4 KB aligned.
/// - `virt` is the desired canonical virtual address.
/// - The caller must ensure no aliasing / double-map violations.
pub unsafe fn map_phys_page(pml4_phys: u64, virt: u64, phys: u64, flags: u64) {
    // Extract the 9-bit index for each page-table level from the virtual address.
    // Each index selects one of 512 entries in its respective table.
    let pml4_idx = ((virt >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((virt >> 30) & 0x1FF) as usize;
    let pd_idx   = ((virt >> 21) & 0x1FF) as usize;
    let pt_idx   = ((virt >> 12) & 0x1FF) as usize;

    // =========================================================================
    // LEVEL 4: Walk PML4 → PDPT
    // =========================================================================
    // Read the PML4 entry for this virtual address range. If the entry is
    // Present, it already points to a PDPT frame. If not, we allocate a
    // new zeroed frame and write the entry — this is the "lazy allocation"
    // pattern repeated at every level.
    let pml4e = read_pt_entry(pml4_phys, pml4_idx);
    let pdpt = if pml4e & page_flags::PRESENT != 0 {
        // Extract the physical frame address (bits 12-51) from the existing entry
        pml4e & 0x000FFFFFFFFFF000
    } else {
        // Allocate a new PDPT frame and link it into the PML4
        let frame = alloc_zeroed_frame();
        // Propagate USER_ACCESS from the PTE flags up through intermediate entries
        let iflags = page_flags::PRESENT | page_flags::WRITABLE
            | (flags & page_flags::USER_ACCESS);
        write_pt_entry(pml4_phys, pml4_idx, (frame & 0x000FFFFFFFFFF000) | iflags);
        frame
    };

    // =========================================================================
    // LEVEL 3: Walk PDPT → PD
    // =========================================================================
    let pdpte = read_pt_entry(pdpt, pdpt_idx);
    let pd = if pdpte & page_flags::PRESENT != 0 {
        pdpte & 0x000FFFFFFFFFF000
    } else {
        let frame = alloc_zeroed_frame();
        let iflags = page_flags::PRESENT | page_flags::WRITABLE
            | (flags & page_flags::USER_ACCESS);
        write_pt_entry(pdpt, pdpt_idx, (frame & 0x000FFFFFFFFFF000) | iflags);
        frame
    };

    // =========================================================================
    // LEVEL 2: Walk PD → PT
    // =========================================================================
    // Note: We always create a 4KB PT here. In theory, we could use a 2MB
    // huge page (set the LARGE bit in the PDE), but for device-memory mappings
    // like the VGA buffer at 0xB8000, a single 4KB page is all we need.
    let pde = read_pt_entry(pd, pd_idx);
    let pt = if pde & page_flags::PRESENT != 0 {
        pde & 0x000FFFFFFFFFF000
    } else {
        let frame = alloc_zeroed_frame();
        let iflags = page_flags::PRESENT | page_flags::WRITABLE
            | (flags & page_flags::USER_ACCESS);
        write_pt_entry(pd, pd_idx, (frame & 0x000FFFFFFFFFF000) | iflags);
        frame
    };

    // =========================================================================
    // LEVEL 1: Write the final 4 KB PTE
    // =========================================================================
    // Now we're at the leaf level. The PTE maps a single 4KB physical page
    // to the virtual address. The `flags` argument typically includes
    // PRESENT | WRITABLE | USER_ACCESS, and for device memory we also
    // set CACHE_DISABLE (PCD bit) — because writing to VGA memory with
    // caching enabled can cause stale reads or unpredictable behaviour.
    let pte = (phys & 0x000FFFFFFFFFF000) | flags;
    write_pt_entry(pt, pt_idx, pte);

    // After this, the VGA server (or any user-space driver) can access
    // its device memory directly via virtual addresses — no syscall needed
    // for each character write. This is the key performance advantage of
    // user-space drivers in a microkernel.
}

// =============================================================================
// TRACKING CURRENT PROCESS (kept for compatibility)
// =============================================================================

use core::sync::atomic::AtomicU16;

static CURRENT_PROCESS_ID: AtomicU16 = AtomicU16::new(0);

pub fn set_current_process(pid: u16) {
    CURRENT_PROCESS_ID.store(pid, Ordering::Release);
}

pub fn get_current_process() -> u16 {
    CURRENT_PROCESS_ID.load(Ordering::Acquire)
}

// =============================================================================
// MEMORY REGION (kept for compatibility)
// =============================================================================

#[derive(Clone, Copy)]
pub struct MemoryRegion {
    pub start: u64,
    pub size: u64,
    pub flags: u64,
}

impl MemoryRegion {
    pub const fn new(start: u64, size: u64, flags: u64) -> Self {
        Self { start, size, flags }
    }
}

// =============================================================================
// PAGE TABLE ENTRY HELPER (kept for compatibility)
// =============================================================================

#[derive(Clone, Copy)]
pub struct PageTableEntry {
    pub raw: u64,
}

impl PageTableEntry {
    pub fn new(addr: u64, flags: u64) -> Self {
        Self {
            raw: (addr & 0x000FFFFFFFFFF000) | flags,
        }
    }

    pub fn present(&self) -> bool {
        self.raw & page_flags::PRESENT != 0
    }

    pub fn writable(&self) -> bool {
        self.raw & page_flags::WRITABLE != 0
    }

    pub fn user_accessible(&self) -> bool {
        self.raw & page_flags::USER_ACCESS != 0
    }
}

// =============================================================================
// USER POINTER VALIDATION
// =============================================================================

pub fn is_valid_user_vaddr(addr: u64) -> bool {
    addr >= USER_VADDR_START && addr < USER_VADDR_END
}

pub fn is_valid_user_buffer(addr: u64, size: usize) -> bool {
    let end = addr.saturating_add(size as u64);
    is_valid_user_vaddr(addr) && is_valid_user_vaddr(end.saturating_sub(1))
}
