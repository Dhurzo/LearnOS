//! Integration tests for the Interrupt Server (Phase 1).
//!
//! These tests run in a host environment (not inside the kernel).
//! They exercise the helper functions in kernel/src/interrupt_server.rs.

use kernel::interrupt_server::{register_handler, unregister_handler, has_handler};

#[test]
fn test_register_and_unregister_handler() {
    // Fresh IRQ should have no handlers
    assert!(!has_handler(1));

    // Register a handler PID for keyboard IRQ (IRQ 1)
    assert!(register_handler(1, 42));
    assert!(has_handler(1));

    // Unregister it again
    assert!(unregister_handler(1, 42));
    assert!(!has_handler(1));
}

#[test]
fn test_irq_out_of_range() {
    // IRQs >= BASE_IRQ_COUNT should be rejected
    assert!(!register_handler(99, 42));
    assert!(!unregister_handler(99, 42));
}

#[test]
fn test_double_register() {
    assert!(register_handler(1, 100));
    // Registering the same PID again should also succeed
    assert!(register_handler(1, 100));
    // Clean up
    assert!(unregister_handler(1, 100));
    assert!(unregister_handler(1, 100));
}
