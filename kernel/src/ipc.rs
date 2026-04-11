//! Simple IPC mechanism for the microkernel.
//!
//! This module provides a primitive message queue used for communication
//! between the kernel and system services (like VGA or Keyboard drivers).

use core::sync::atomic::{AtomicUsize, Ordering};

/// Maximum number of messages in the global IPC queue.
const MAX_MESSAGES: usize = 32;

/// Types of messages that can be sent via IPC.
#[derive(Clone, Copy)]
pub enum Message {
    /// A character to be printed on the VGA screen.
    VgaPrint(u8),
    /// A command to clear the VGA screen.
    VgaClear,
    /// A command to move the cursor back (backspace).
    VgaBackspace,
    /// A newline command for the VGA screen.
    VgaNewline,
    /// An input event from the keyboard.
    KeyboardEvent(KeyEvent),
}

/// Types of events produced by the keyboard driver.
#[derive(Clone, Copy)]
pub enum KeyEvent {
    Char(u8),
    Enter,
    Backspace,
}

/// A simple circular buffer for storing messages.
pub struct MessageQueue {
    buffer: [Option<Message>; MAX_MESSAGES],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl MessageQueue {
    pub const fn new() -> Self {
        Self {
            buffer: [None; MAX_MESSAGES],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Sends a message to the queue. Returns `false` if the queue is full.
    pub fn send(&mut self, msg: Message) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let next_tail = (tail + 1) % MAX_MESSAGES;

        if next_tail == self.head.load(Ordering::Acquire) {
            return false; // Queue full
        }

        self.buffer[tail] = Some(msg);
        self.tail.store(next_tail, Ordering::Release);
        true
    }

    /// Receives a message from the queue. Returns `None` if the queue is empty.
    pub fn receive(&mut self) -> Option<Message> {
        let head = self.head.load(Ordering::Relaxed);

        if head == self.tail.load(Ordering::Acquire) {
            return None; // Queue empty
        }

        let msg = self.buffer[head].take();
        self.head
            .store((head + 1) % MAX_MESSAGES, Ordering::Release);
        msg
    }
}

/// Global IPC queue.
pub static mut IPC_QUEUE: MessageQueue = MessageQueue::new();
