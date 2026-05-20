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
//! FRAME ALLOCATOR
//! =============================================================================
//!
//! Simple bump allocator that hands out 4KB-aligned physical frames from
//! just past the kernel's BSS section (__kernel_end) upward. This is safe
//! because:
//!   - The kernel identity-maps physical 0-16MB (enough for kernel + early allocs)
//!   - We know QEMU provides at least 128MB (-m 128)
//!   - No virtual-to-physical offset: physical address = virtual address

use core::sync::atomic::{AtomicU64, Ordering};

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

static ALLOC_CURSOR: AtomicU64 = AtomicU64::new(0);

pub fn init_frame_allocator() {
    unsafe {
        let kernel_end = &__kernel_end as *const u8 as u64;
        let start = (kernel_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        ALLOC_CURSOR.store(start, Ordering::Relaxed);
    }
}

/// Allocate a single 4KB physical frame.
///
/// Returns the physical address of the frame (identity-mapped, so it's both
/// the physical and virtual address). The frame is zero-initialized.
///
/// # Panics
///
/// Panics if the cursor wraps around (exhausted the identity-mapped region).
pub fn alloc_frame() -> u64 {
    let addr = ALLOC_CURSOR.fetch_add(PAGE_SIZE, Ordering::Relaxed);
    // Zero the frame
    unsafe {
        core::ptr::write_bytes(addr as *mut u8, 0, PAGE_SIZE as usize);
    }
    addr
}

/// Allocate and zero a 4KB frame, returning the physical address.
pub fn alloc_zeroed_frame() -> u64 {
    alloc_frame()
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
            | page_flags::WRITABLE;
        write_pt_entry(pml4, 0, pdpt_ent);

        // === Level 2: PD ===
        let pd = alloc_zeroed_frame();
        let pd_ent = (pd & 0x000FFFFFFFFFF000)
            | page_flags::PRESENT
            | page_flags::WRITABLE;
        write_pt_entry(pdpt, 0, pd_ent);

        // --- PD[0]: identity map 0x000000-0x1FFFFF (kernel code, U=0) ---
        // 2MB huge page: Present | Writable | Large (no User)
        write_pt_entry(pd, 0, 0x000000 | (page_flags::PRESENT | page_flags::WRITABLE | page_flags::LARGE));

        // --- PD[1]: identity map 0x200000-0x3FFFFF (kernel data+IDT, U=0) ---
        write_pt_entry(pd, 1, 0x200000 | (page_flags::PRESENT | page_flags::WRITABLE | page_flags::LARGE));

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
