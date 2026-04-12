//! Memory Management and Paging
//!
//! This module handles virtual memory and user/kernel separation.
//!
//! =============================================================================
//! WHY PAGING?
//! =============================================================================
//!
//! Without paging, programs can access ALL physical memory.
//! This is dangerous - one buggy program can crash the whole system!
//!
//! Paging provides:
//! 1. Memory protection: Each process has its own address space
//! 2. Memory isolation: Can't access other processes' memory
//! 3. Efficient sharing: Can share read-only pages
//! 4. Memory overcommit: Can swap unused pages to disk
//!
//! =============================================================================
//! x86_64 PAGING (4-LEVEL)
//! =============================================================================
//!
//! x86_64 uses hierarchical page tables:
//!
//!   PML4 (Page Map Level 4) - 512 entries
//!     |
//!     +-> PDPT (Page Directory Pointer Table) - 512 entries
//!           |
//!           +-> PD (Page Directory) - 512 entries
//!                 |
//!                 +-> PT (Page Table) - 512 entries
//!                       |
//!                       +-> 4KB page of physical memory
//!
//! Each entry is 8 bytes (64 bits), each table 512 * 8 = 4096 bytes
//! (one memory page).
//!
//! Virtual address layout:
//!   Bits 63-48: Sign-extended (must match bit 47)
//!   Bits 47-39: PML4 index (9 bits)
//!   Bits 38-30: PDPT index (9 bits)
//!   Bits 29-21: PD index (9 bits)
//!   Bits 20-12: PT index (9 bits)
//!   Bits 11-0: Offset within 4KB page (12 bits)
//!
//! =============================================================================
//! PAGE TABLE ENTRIES (PTE)
//! =============================================================================
//!
//! Each 8-byte PTE contains:
//!   Bits 0: Present (P)
//!   Bit 1: Writable (W)
//!   Bit 2: User Accessible (U)
//!   Bit 3: Write-Through (PWT)
//!   Bit 4: Cache Disabled (PCD)
//!   Bit 5: Accessed (A) - set by CPU
//!   Bit 6: Dirty (D) - set by CPU (PTEs only)
//!   Bit 7: Large/Page Size (PS) - 2MB page
//!   Bits 8-11: Available for OS
//!   Bits 12-51: Physical frame number (address >> 12)
//!   Bits 52-62: Available for OS
//!   Bit 63: No Execute (NX)
//!
//! Common flags:
//!   0x03 = Present + Writable
//!   0x07 = Present + Writable + User
//!   0x83 = Present + Writable + Accessed + Large
//!
//! =============================================================================
//! USER/KERNEL SEPARATION
//! =============================================================================
//!
//! Virtual address space is split:
//!   Lower half (0x0000000000000000 - 0x00007FFFFFFFFFFF):
//!     User space. Can be accessed from user mode (ring 3).
//!
//!   Upper half (0xFFFF800000000000+):
//!     Kernel space. Only accessible from kernel mode (ring 0).
//!     User accesses to this region cause a page fault!
//!
//! This is enforced by the U bit in page table entries.
//! User pages have U=1, kernel pages have U=0.
//!
//! Physical:
//!   0x000000 - 0x200000: Kernel code (identity mapped)
//!   0x0B8000: VGA text memory
//!   User processes: loaded at 0x400000
//!
//! =============================================================================
//! ADDRESS CONSTANTS
//! =============================================================================

use core::sync::atomic::{AtomicU64, Ordering};

/// User process code load address (typical ELF load address)
pub const USER_VADDR_LOAD: u64 = 0x0000000000400000;
pub const USER_VADDR_START: u64 = USER_VADDR_LOAD;

/// User virtual address limit
pub const USER_VADDR_END: u64 = 0x00007FFFFFFFFFFF;

/// User process stack address (high, grows down)
/// Using a lower stack address within mapped memory for demo
pub const USER_STACK_VADDR: u64 = 0x0000000001001000;

/// Stack size (8KB)
pub const USER_STACK_SIZE: u64 = 0x2000;

/// Page size (4KB)
pub const PAGE_SIZE: u64 = 4096;

/// Number of entries per page table
pub const PT_ENTRIES: usize = 512;

/// ============================================================================
/// PAGE TABLE FLAG CONSTANTS
/// ============================================================================
pub mod page_flags {
    /// Page is present in memory
    pub const PRESENT: u64 = 1 << 0;

    /// Page is writable
    pub const WRITABLE: u64 = 1 << 1;

    /// Accessible from user mode
    pub const USER_ACCESS: u64 = 1 << 2;

    /// Write-through caching
    pub const WRITE_THROUGH: u64 = 1 << 3;

    /// Cache disabled
    pub const CACHE_DISABLE: u64 = 1 << 4;

    /// No execute (NX bit)
    pub const EXECUTABLE_DISABLE: u64 = 1 << 63;
}

/// User-accessible page flags (U=1)
pub const PTE_USER: u64 = page_flags::PRESENT | page_flags::WRITABLE | page_flags::USER_ACCESS;

/// Kernel-only page flags (U=0)
pub const PTE_KERNEL: u64 = page_flags::PRESENT | page_flags::WRITABLE;

/// ============================================================================
/// TRACKING CURRENT PROCESS
/// ============================================================================

/// Current running process ID (atomic for thread safety)
static CURRENT_PROCESS_ID: AtomicU64 = AtomicU64::new(0);

/// Set current process ID
pub fn set_current_process(pid: u64) {
    CURRENT_PROCESS_ID.store(pid, Ordering::Release);
}

/// Get current process ID
pub fn get_current_process() -> u64 {
    CURRENT_PROCESS_ID.load(Ordering::Acquire)
}

/// ============================================================================
/// MEMORY REGION
/// ============================================================================

/// Represents a process's memory region
#[derive(Clone, Copy)]
pub struct MemoryRegion {
    /// Start address
    pub start: u64,
    /// Size in bytes
    pub size: u64,
    /// Page flags
    pub flags: u64,
}

impl MemoryRegion {
    /// Create new region
    pub const fn new(start: u64, size: u64, flags: u64) -> Self {
        Self { start, size, flags }
    }
}

/// ============================================================================
/// PAGE TABLE ENTRY HELPER
/// ============================================================================

/// Helper for creating/manipulating page table entries
#[derive(Clone, Copy)]
pub struct PageTableEntry {
    pub raw: u64,
}

impl PageTableEntry {
    /// Create new PTE
    pub fn new(addr: u64, flags: u64) -> Self {
        Self {
            // Mask off lower 12 bits (offset within page)
            raw: (addr & 0x000FFFFFFFFFF000) | flags,
        }
    }

    /// Check if page is present
    pub fn present(&self) -> bool {
        self.raw & page_flags::PRESENT != 0
    }

    /// Check if writable
    pub fn writable(&self) -> bool {
        self.raw & page_flags::WRITABLE != 0
    }

    /// Check if user-accessible
    pub fn user_accessible(&self) -> bool {
        self.raw & page_flags::USER_ACCESS != 0
    }
}

/// ============================================================================
/// USER POINTER VALIDATION
/// ============================================================================

/// Check if address is in user space
///
/// Used by syscalls to validate user pointers before dereferencing.
pub fn is_valid_user_vaddr(addr: u64) -> bool {
    addr >= USER_VADDR_START && addr < USER_VADDR_END
}

/// Check if buffer is entirely in user space
///
/// Checks start and (start + size) are both in user space.
pub fn is_valid_user_buffer(addr: u64, size: usize) -> bool {
    let end = addr.saturating_add(size as u64);
    is_valid_user_vaddr(addr) && is_valid_user_vaddr(end.saturating_sub(1))
}
