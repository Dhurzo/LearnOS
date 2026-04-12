//! Inter-Process Communication (IPC) Primitives for the Microkernel
//!
//! This module defines the core message-passing infrastructure used by the microkernel
//! to facilitate communication between the kernel core and user-space services.
//!
//! ## Design Philosophy
//!
//! In a microkernel, memory management and process scheduling are minimal.
//! Most "operating system" functionality lives in **servers** (processes) running in user space.
//! These servers communicate via IPC.
//!
//! This implementation provides a lightweight, lock-free message queue suitable for
//! a single-core educational kernel. It uses atomic operations for head/tail indices
//! to ensure thread-safety (even if we only have one CPU for now).
//!
//! ## Message Types
//!
//! The `Message` enum defines the contract between the kernel and its services.
//! - **VgaPrint / VgaClear / VgaBackspace**: Commands for the Display Server.
//! - **KeyboardEvent**: Data from the Input Server (Keyboard Driver).
//!
//! ## Usage
//!
//! Services push messages to the queue using `send()`.
//! The kernel core (dispatcher) reads messages using `receive()`.
//!
//! ```rust
//! // Service sending a character to be printed
//! queue.send(Message::VgaPrint(b'X'));
//!
//! // Kernel processing the message
//! if let Some(msg) = queue.receive() {
//!     handle(msg);
//! }
//! ```

use core::sync::atomic::{AtomicUsize, Ordering};

/// Maximum number of messages that can be buffered in the IPC queue.
///
/// This size is chosen to be small enough for a static buffer but large enough
/// to handle burst traffic from drivers.
const MAX_MESSAGES: usize = 32;

/// The set of messages that can be sent between the kernel and services.
#[derive(Clone, Copy)]
pub enum Message {
    /// A single character to be printed to the screen.
    VgaPrint(u8),
    /// A command to clear the entire screen.
    VgaClear,
    /// A command to move the cursor back one position (backspace).
    VgaBackspace,
}

/// A lock-free, single-producer single-consumer (SPSC) circular buffer implementation.
///
/// This queue allows services to enqueue messages that the kernel will dequeue and process.
/// It relies on `AtomicUsize` for the head and tail indices to ensure consistency.
pub struct MessageQueue {
    /// The internal ring buffer storage.
    buffer: [Option<Message>; MAX_MESSAGES],
    /// Index of the next slot to read from.
    head: AtomicUsize,
    /// Index of the next slot to write to.
    tail: AtomicUsize,
}

impl MessageQueue {
    /// Creates a new, empty message queue.
    pub const fn new() -> Self {
        // Initialize the buffer with empty options and zero indices.
        Self {
            buffer: [None; MAX_MESSAGES],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Enqueues a message.
    ///
    /// Returns `true` if the message was successfully enqueued.
    /// Returns `false` if the queue is full (overflow handling).
    ///
    /// # Ordering
    ///
    /// Uses `Relaxed` for the load of `tail` (we only care about our own view)
    /// and `Acquire`/`Release` for the synchronization of the data itself.
    pub fn send(&mut self, msg: Message) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let next_tail = (tail + 1) % MAX_MESSAGES;

        // Check if the queue is full (head == next_tail implies collision)
        if next_tail == self.head.load(Ordering::Acquire) {
            return false; // Queue full
        }

        // Write the message to the buffer
        self.buffer[tail] = Some(msg);

        // Update the tail index, signaling to consumers that a new message is available
        self.tail.store(next_tail, Ordering::Release);
        true
    }

    /// Dequeues a message.
    ///
    /// Returns `Some(Message)` if a message was available, or `None` if the queue is empty.
    ///
    /// # Ordering
    ///
    /// Similar to `send`, uses atomic ordering to ensure we see the writes
    /// made by the producer.
    pub fn receive(&mut self) -> Option<Message> {
        let head = self.head.load(Ordering::Relaxed);

        // Check if the queue is empty
        if head == self.tail.load(Ordering::Acquire) {
            return None; // Queue empty
        }

        // Read the message from the buffer
        let msg = self.buffer[head].take();

        // Update the head index, signaling to producers that a slot is free
        self.head
            .store((head + 1) % MAX_MESSAGES, Ordering::Release);
        msg
    }
}

/// The global IPC queue instance.
///
/// This static holds the queue used for all kernel-to-service and service-to-kernel
/// communication.
///
/// # Safety
///
/// It is declared as `mut` because the queue requires mutable access for `send`/`receive`.
/// In a multi-core system, this would require internal mutability (RefCell) orAtomics.
/// Here, we rely on the single-threaded nature of the kernel init phase.
pub static mut IPC_QUEUE: MessageQueue = MessageQueue::new();
