// tests/test_capability.rs
use crate::process::{ProcessTable, IpcMessage};
use crate::capability::{check_ipc_send, grant_capability, revoke_capability, CAP_VGA_SEND, CAP_KEYBOARD_SEND};

#[test]
fn test_unauthorized_ipc() {
    // Create two processes: sender without capability, receiver with VGA capability.
    let mut table = ProcessTable::new();
    let pid_sender = 1;
    let pid_receiver = 2;
    table.add_process(pid_sender);
    table.add_process(pid_receiver);

    // Grant receiver capability to receive VGA messages.
    grant_capability(pid_receiver, CAP_VGA_SEND);

    // Sender attempts to send IPC to receiver without capability.
    let msg = IpcMessage::new(0, 1, [0u8; 60]);
    let result = table.get_mut(pid_sender).unwrap().ipc_push(msg.clone());
    assert!(result.is_err(), "Sender should not be able to send IPC without capability");
}
