//! User-space Service Stubs
//!
//! These modules demonstrate how user-space services interact with the microkernel
//! via system calls. In a full implementation, these would be separate ELF binaries
//! loaded into user address space.
//!
//! For now, they serve as documentation of the syscall interface and can be used
//! as a template for future user-space development.

pub mod console {
    use crate::syscall::{syscall0, syscall1, syscall_nr};

    pub fn write_char(c: u8) {
        unsafe {
            let _ = syscall1(syscall_nr::VGA_WRITE, c as usize);
        }
    }

    pub fn write_string(s: &str) {
        for byte in s.bytes() {
            write_char(byte);
        }
    }

    pub fn clear() {
        unsafe {
            let _ = syscall0(syscall_nr::VGA_CLEAR);
        }
    }
}

pub mod vfs {
    use crate::syscall::{syscall3, syscall_nr};

    pub const STDOUT: usize = 1;
    pub const STDERR: usize = 2;

    pub fn write(fd: usize, buf: &[u8]) -> isize {
        unsafe { syscall3(syscall_nr::WRITE, fd, buf.as_ptr() as usize, buf.len()) }
    }
}

pub mod ipc_client {
    use crate::syscall::{syscall2, syscall3, syscall_nr};

    pub fn send_msg(to_pid: usize, msg_type: usize, data: usize) -> isize {
        unsafe { syscall3(syscall_nr::IPC_SEND, to_pid, msg_type, data) }
    }

    pub fn recv_msg(from_pid: usize) -> isize {
        unsafe { syscall2(syscall_nr::IPC_RECV, from_pid, 0) }
    }
}
