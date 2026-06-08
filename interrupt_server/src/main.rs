#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// Entry point for the user-space interrupt server process.
///
/// Phase 1 of the microkernel plan: this process loops on IPC_RECV,
/// receives IRQ messages from the kernel, and forwards them to
/// registered driver processes.
#[no_mangle]
pub extern "C" fn main() -> ! {
    loop {
        // TODO: Phase 1 — receive IRQ messages from kernel via IPC
        // and forward to registered handlers.
        //
        // 1. ipc_recv() → Message::Interrupt { irq, data }
        // 2. Look up handler PID for `irq` in local table
        // 3. ipc_send(handler_pid, translated_message)
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
