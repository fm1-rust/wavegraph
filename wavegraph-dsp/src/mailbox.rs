//! Move one owned value at a time between a synchronous renderer and its host.
//!
//! Split the mailbox once and give each endpoint its own task or core. Neither
//! operation allocates, waits for its peer, or calls a task waker. A failed
//! send returns the original value. Use two mailboxes to lend a render buffer
//! to a worker and get it back after the worker has finished writing.
//!
//! Both cores must access the same coherent memory. A CPU-only mailbox does
//! not perform cache maintenance for a DMA device.

use core::{
    fmt,
    sync::atomic::{AtomicUsize, Ordering, fence},
};
use heapless::spsc::{Consumer, Producer, Queue};

/// Storage for a one-item, single-producer, single-consumer channel.
#[derive(Debug)]
pub struct Mailbox<T> {
    queue: Queue<T, 2>, // heapless reserves one of the two entries.
    state: Publication,
}

impl<T: Send> Mailbox<T> {
    /// Create an empty mailbox.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            queue: Queue::new(),
            state: Publication(AtomicUsize::new(0)),
        }
    }

    /// Borrow the unique sender and receiver. A value still in the mailbox
    /// when the endpoints are dropped is available after splitting it again.
    pub fn split(&mut self) -> (Sender<'_, T>, Receiver<'_, T>) {
        let (producer, consumer) = self.queue.split();
        (
            Sender {
                producer,
                state: &self.state,
            },
            Receiver {
                consumer,
                state: &self.state,
            },
        )
    }
}

impl<T: Send> Default for Mailbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

// The queue moves T safely. This word publishes the completed move across
// a full hardware barrier in either direction. Some weakly ordered embedded
// targets lower acquire/release word operations to plain loads and stores.
// A fence outside enqueue alone cannot order its internal payload/index stores.
#[derive(Debug)]
struct Publication(AtomicUsize);

impl Publication {
    #[inline]
    fn ready(&self, full: bool) -> bool {
        if self.0.load(Ordering::Acquire) != usize::from(full) {
            return false;
        }
        fence(Ordering::SeqCst);
        true
    }

    #[inline]
    fn set(&self, full: bool) {
        fence(Ordering::SeqCst);
        self.0.store(usize::from(full), Ordering::Release);
        fence(Ordering::SeqCst);
    }
}

/// The sole sender of a mailbox.
pub struct Sender<'a, T> {
    producer: Producer<'a, T>,
    state: &'a Publication,
}

impl<T> fmt::Debug for Sender<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender").finish_non_exhaustive()
    }
}

impl<T> Sender<'_, T> {
    /// Transfer ownership immediately if the slot is free.
    ///
    /// # Errors
    /// Returns the original value if the receiver has not taken its predecessor.
    #[inline]
    pub fn try_send(&mut self, value: T) -> Result<(), T> {
        if !self.state.ready(false) {
            return Err(value);
        }
        self.producer.enqueue(value)?;
        self.state.set(true);
        Ok(())
    }
}

/// The sole receiver of a mailbox.
pub struct Receiver<'a, T> {
    consumer: Consumer<'a, T>,
    state: &'a Publication,
}

impl<T> fmt::Debug for Receiver<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish_non_exhaustive()
    }
}

impl<T> Receiver<'_, T> {
    /// Take the published value, or return immediately if the slot is empty.
    #[inline]
    pub fn try_recv(&mut self) -> Option<T> {
        if !self.state.ready(true) {
            return None;
        }
        let value = self.consumer.dequeue()?;
        self.state.set(false);
        Some(value)
    }
}
