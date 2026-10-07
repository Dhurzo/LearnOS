#![no_std]
#![no_main]
// The interrupt server is single-threaded: IRQ handling never preempts itself,
// so direct access to the `static mut` handler table is safe by construction.
#![allow(static_mut_refs)]

//! User-space interrupt server (Interrupt Server Router — ISR).
//!
//! This process implements the microkernel's user-space interrupt handling:
//! it receives hardware-IRQ messages from the kernel over IPC, keeps a local
//! table of which driver handles which IRQ, and forwards each IRQ to the
//! registered handler in the format that handler expects.
//!
//! Handlers do NOT get their routing table pushed by the kernel. Instead they
//! **self-register** with this server at startup via `MSG_IRQ_REGISTER` (see
//! `userspace/src/bin/keyboard.rs`). The kernel only decides *whether* to route
//! IRQs through us (`INTERRUPT_SERVER_PID != 0`) — it never tells us who handles
//! each line.

use userspace::{ipc_recv, ipc_send, vga_write, IpcMessage};

/// Print a boot/probe line via the VGA server (serial mirror included).
fn trace(s: &[u8]) {
    unsafe {
        for &b in s.iter() {
            let _ = vga_write(b as usize);
        }
    }
}

/// IPC message types exchanged with the kernel and with handlers.
///
/// Kept in sync with the kernel's direct-delivery constants so that forwarding
/// through this server produces byte-identical messages to the legacy
/// `INTERRUPT_SERVER_PID == 0` path (see `kernel/src/interrupt_server.rs`).
mod msg {
    /// Kernel → us: hardware IRQ. `data[0]` = IRQ number, `data[1..]` = payload.
    pub const HW_IRQ: u16 = 4;
    /// Us → keyboard handler: a scancode arrived (`data[0]`).
    pub const KEY_SCANCODE: u16 = 3;
    /// Us → generic handler: IRQ data (`data[0]` = first payload byte).
    pub const IRQ_DATA: u16 = 5;
    /// Handler → us: self-registration. `data[0]` = IRQ, `data[1..3]` = PID (u16 LE).
    pub const REGISTER: u16 = 6;
}

/// Maximum number of registered (irq -> handler pid) entries.
const MAX_HANDLERS: usize = 16;

/// Local IRQ → handler-PID table owned by the interrupt server.
///
/// Mirrors the kernel's `IRQ_HANDLERS` table, but lives in user space and is
/// populated by driver self-registration rather than by the kernel.
static mut HANDLERS: [(u8, u16); MAX_HANDLERS] = [(0, 0); MAX_HANDLERS];

/// Look up the handler PID registered for `irq`. Returns 0 if none.
unsafe fn lookup_handler(irq: u8) -> u16 {
    for (line, pid) in HANDLERS.iter() {
        if *line == irq && *pid != 0 {
            return *pid;
        }
    }
    0
}

/// Register `handler_pid` as the handler for `irq`. Replaces an existing entry
/// for the same IRQ, otherwise fills the first free slot. No-op if full.
unsafe fn register(irq: u8, handler_pid: u16) {
    for slot in HANDLERS.iter_mut() {
        if slot.0 == irq {
            slot.1 = handler_pid;
            return;
        }
    }
    for slot in HANDLERS.iter_mut() {
        if slot.1 == 0 {
            slot.0 = irq;
            slot.1 = handler_pid;
            return;
        }
    }
}

/// Forward a hardware IRQ to `handler_pid` as the message that handler expects.
///
/// Mirrors the kernel's direct-delivery translation in
/// `kernel/src/interrupt_server.rs`: IRQ 1 (keyboard) is delivered as
/// `MSG_KEY_SCANCODE` with the scancode in `data[0]`; every other IRQ is
/// delivered as `MSG_IRQ_DATA`.
unsafe fn forward(irq: u8, payload_first: u8, handler_pid: u16) {
    let msg_type = if irq == 1 { msg::KEY_SCANCODE } else { msg::IRQ_DATA };
    let mut data = [0u8; userspace::IPC_MSG_SIZE - 4];
    data[0] = payload_first;
    let msg = IpcMessage::new(0, msg_type, data);
    let _ = ipc_send(handler_pid as usize, &msg as *const IpcMessage as usize);
}

/// Entry point for the user-space interrupt server process.
///
/// Loops on `IPC_RECV`: parses registration requests and hardware-IRQ messages,
/// maintains the IRQ → handler table, and forwards IRQs to handlers.
#[no_mangle]
pub extern "C" fn main() -> ! {
    let buf: [u8; 64] = [0u8; 64];
    trace(b"ISR up\n");
    loop {
        let len = unsafe { ipc_recv(buf.as_ptr() as usize) };
        if len <= 0 {
            continue;
        }

        // The kernel wrote a 64-byte IpcMessage into `buf`. Copy it out via an
        // unaligned read (IpcMessage is Copy + repr(C), no padding).
        let msg: IpcMessage = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const IpcMessage) };

        match msg.msg_type {
            // A handler is telling us it owns a given IRQ.
            msg::REGISTER => {
                let irq = msg.data[0];
                let handler_pid = (msg.data[1] as u16) | ((msg.data[2] as u16) << 8);
                unsafe { register(irq, handler_pid) };
            }
            // The kernel is delivering a hardware IRQ.
            msg::HW_IRQ => {
                let irq = msg.data[0];
                let payload_first = msg.data.get(1).copied().unwrap_or(0);
                let handler_pid = unsafe { lookup_handler(irq) };
                if handler_pid != 0 {
                    unsafe { forward(irq, payload_first, handler_pid) };
                }
            }
            // Unknown / unrelated message: ignore.
            _ => {}
        }
    }
}
