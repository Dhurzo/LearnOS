//! User programs

use crate::syscall::{syscall1, syscall_nr};

pub mod init {
    use crate::syscall::{syscall1, syscall_nr};

    #[no_mangle]
    pub extern "C" fn init_main() {
        unsafe {
            let _ = syscall1(syscall_nr::VGA_WRITE, b'I' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'n' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'i' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b't' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'\n' as usize);
        }
        loop {}
    }
}

pub mod shell {
    use crate::syscall::{syscall1, syscall_nr};

    #[no_mangle]
    pub extern "C" fn shell_main() {
        unsafe {
            let _ = syscall1(syscall_nr::VGA_WRITE, b'S' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'h' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'e' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'l' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'l' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'>' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b' ' as usize);
            let _ = syscall1(syscall_nr::VGA_WRITE, b'\n' as usize);
        }
        loop {}
    }
}
