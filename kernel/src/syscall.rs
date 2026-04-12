//! System Call Interface

use crate::process;
use crate::vga;
use core::sync::atomic::AtomicU64;

pub mod syscall_nr {
    pub const EXIT: usize = 0;
    pub const WRITE: usize = 1;
    pub const READ: usize = 2;
    pub const IPC_SEND: usize = 3;
    pub const IPC_RECV: usize = 4;
    pub const BRK: usize = 5;
    pub const GETPID: usize = 6;
    pub const VGA_WRITE: usize = 7;
    pub const VGA_CLEAR: usize = 8;
    pub const SCHEDULE: usize = 9;
}

pub const SYSCALL_VECTOR: u8 = 0x80;

static CURRENT_PID: AtomicU64 = AtomicU64::new(0);

pub type SyscallHandler = fn(usize, usize, usize, usize, usize, usize) -> isize;

pub struct SyscallTable {
    handlers: [Option<SyscallHandler>; 16],
}

impl SyscallTable {
    pub const fn new() -> Self {
        Self {
            handlers: [None; 16],
        }
    }

    pub fn register(&mut self, nr: usize, handler: SyscallHandler) {
        if nr < 16 {
            self.handlers[nr] = Some(handler);
        }
    }

    pub fn handle(
        &self,
        nr: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> isize {
        self.handlers[nr]
            .map(|h| h(a1, a2, a3, a4, a5, nr))
            .unwrap_or(-1)
    }
}

pub static SYSCALL_TABLE: SyscallTable = {
    let mut table = SyscallTable::new();
    table.handlers[syscall_nr::WRITE] = Some(sys_write);
    table.handlers[syscall_nr::EXIT] = Some(sys_exit);
    table.handlers[syscall_nr::VGA_WRITE] = Some(sys_vga_write);
    table.handlers[syscall_nr::VGA_CLEAR] = Some(sys_vga_clear);
    table.handlers[syscall_nr::SCHEDULE] = Some(sys_schedule);
    table.handlers[syscall_nr::GETPID] = Some(sys_getpid);
    table
};

fn sys_write(fd: usize, buf: usize, count: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    if (fd == 1 || fd == 2) && buf >= 0x400000 && buf < 0x800000000 {
        let slice = unsafe { core::slice::from_raw_parts(buf as *const u8, count) };
        for &byte in slice {
            vga::write_byte(byte);
        }
        count as isize
    } else {
        -1
    }
}

fn sys_exit(_code: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    loop {}
}

fn sys_vga_write(byte: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    vga::write_byte(byte as u8);
    0
}

fn sys_vga_clear(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    vga::clear_screen();
    0
}

fn sys_schedule(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    process::schedule_next();
    0
}

fn sys_getpid(_a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _nr: usize) -> isize {
    process::get_current_pid() as isize
}

#[no_mangle]
pub unsafe extern "C" fn syscall_dispatch(
    nr: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
) -> isize {
    if nr < 16 {
        let handler = SYSCALL_TABLE.handlers[nr];
        if let Some(h) = handler {
            return h(a1, a2, a3, a4, a5, nr);
        }
    }
    -1
}

pub fn get_syscall_handler_addr() -> u64 {
    syscall_dispatch as u64
}

#[no_mangle]
pub extern "C" fn timer_handler() {
    process::schedule_next();
    unsafe {
        core::arch::asm!("mov al, 0x20; out 0x20, al", options(nostack));
    }
}

pub fn get_timer_handler_addr() -> u64 {
    timer_handler as u64
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct IdtEntry {
    pub offset_low: u16,
    pub selector: u16,
    pub ist: u8,
    pub type_attr: u8,
    pub offset_mid: u16,
    pub offset_high: u32,
    pub reserved: u32,
}

impl IdtEntry {
    pub fn new(handler_addr: u64, selector: u16) -> Self {
        Self {
            offset_low: handler_addr as u16,
            selector,
            ist: 0,
            type_attr: 0x8E,
            offset_mid: (handler_addr >> 16) as u16,
            offset_high: (handler_addr >> 32) as u32,
            reserved: 0,
        }
    }

    pub fn null() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }
}

const IDT_ADDR: u64 = 0x100000;

pub fn get_idt_base() -> *mut IdtEntry {
    IDT_ADDR as *mut IdtEntry
}

pub unsafe fn init_idt() {
    let idt = get_idt_base();
    for i in 0..256 {
        idt.add(i).write(IdtEntry::null());
    }
    let syscall_addr = syscall_dispatch as *const () as u64;
    let idt_entry = IdtEntry::new(syscall_addr, 0x08);
    idt.add(0x80).write(idt_entry);
    let timer_addr = timer_handler as *const () as u64;
    let timer_entry = IdtEntry::new(timer_addr, 0x08);
    idt.add(0x20).write(timer_entry);
    #[repr(C, packed)]
    struct IdtPtr {
        limit: u16,
        base: u64,
    }
    let idt_ptr = IdtPtr {
        limit: 16 * 256 - 1,
        base: idt as u64,
    };
    core::arch::asm!("lidt [{}]", in(reg) &idt_ptr, options(nostack));
}

#[inline(always)]
pub unsafe fn syscall0(nr: usize) -> isize {
    let ret: isize;
    core::arch::asm!("mov rax, rdi; syscall", in("rdi") nr, out("rax") ret, options(nostack));
    ret
}

#[inline(always)]
pub unsafe fn syscall1(nr: usize, a1: usize) -> isize {
    let ret: isize;
    core::arch::asm!("mov rax, rsi; syscall", in("rdi") nr, in("rsi") a1, out("rax") ret, options(nostack));
    ret
}

#[inline(always)]
pub unsafe fn syscall2(nr: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    core::arch::asm!(
        "mov rax, rdx; syscall",
        in("rdi") nr,
        in("rsi") a1,
        in("rdx") a2,
        out("rax") ret,
        options(nostack)
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall3(nr: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    core::arch::asm!(
        "mov rax, r10; syscall",
        in("rdi") nr,
        in("rsi") a1,
        in("rdx") a2,
        in("r10") a3,
        out("rax") ret,
        options(nostack)
    );
    ret
}
