//! Await a program change while a host continues to render synchronously.
//!
//! Split an [`EditQueue`] between a control task and the host that owns the
//! patch. An edit can be a reference to a decoded program or a small preset ID.
//! The host applies it between blocks using [`Patch::replace`], then reports
//! the first accepted block. The application supplies its `critical-section`
//! implementation and an interrupt-safe task waker if the host runs in an ISR.
//!
//! [`Patch::replace`]: crate::bytecode::Patch::replace

use crate::bytecode;
use core::{
    cell::RefCell,
    future::poll_fn,
    mem,
    task::{Poll, Waker},
};
use critical_section::Mutex;

/// Identifies the program used to render a block within one host session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Revision(u64);

/// The first block accepted by the output using the requested program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Applied {
    /// Revision returned by [`Host::begin_block`] for this edit.
    pub revision: Revision,
    /// First sample frame of that block on the host's render timeline.
    pub first_frame: u64,
}

/// Why an edit could not take effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitError {
    /// The host rejected the replacement and kept the previous program.
    Rejected(bytecode::Error),
    /// The host was dropped before it confirmed the edit.
    Closed,
}

#[derive(Debug)]
enum Slot<P> {
    Idle,
    Queued(P),
    Rendering,
    Finished(Result<Applied, CommitError>),
}

#[derive(Debug)]
struct State<P> {
    slot: Slot<P>,
    waiter: Option<Waker>,
    closed: bool,
}

/// Caller-owned storage for one outstanding edit and its receipt.
#[derive(Debug)]
pub struct EditQueue<P> {
    state: Mutex<RefCell<State<P>>>,
}

impl<P> EditQueue<P> {
    /// Create an empty queue. Use a small, copyable program handle for `P`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(RefCell::new(State {
                slot: Slot::Idle,
                waiter: None,
                closed: false,
            })),
        }
    }

    /// Start a render session with revision zero representing the initial program.
    pub fn split(&mut self) -> (Controller<'_, P>, Host<'_, P>) {
        *self = Self::new();
        (
            Controller { queue: self },
            Host {
                queue: self,
                revision: Revision(0),
                awaiting_output: false,
            },
        )
    }

    fn with<R>(&self, f: impl FnOnce(&mut State<P>) -> R) -> R {
        critical_section::with(|cs| f(&mut self.state.borrow_ref_mut(cs)))
    }

    fn update_and_wake(&self, update: impl FnOnce(&mut State<P>)) {
        let waiter = self.with(|state| {
            update(state);
            state.waiter.take()
        });
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }
}

impl<P> Default for EditQueue<P> {
    fn default() -> Self {
        Self::new()
    }
}

/// Control-task endpoint of an [`EditQueue`].
#[derive(Debug)]
pub struct Controller<'a, P> {
    queue: &'a EditQueue<P>,
}

impl<P: Copy> Controller<'_, P> {
    /// Queue a replacement and await its first accepted output block. Dropping
    /// the future after enqueue leaves the edit in flight. A subsequent commit
    /// waits for that edit to finish before reusing the slot.
    ///
    /// # Errors
    /// Returns the host's replacement error or [`CommitError::Closed`].
    pub async fn commit(&mut self, plan: P) -> Result<Applied, CommitError> {
        let mut sent = false;
        poll_fn(|cx| {
            // Waker callbacks, including cloning and dropping, run outside the critical section.
            let mut waiter = Some(cx.waker().clone());
            self.queue.with(|state| {
                match state.slot {
                    Slot::Finished(result) if sent => return Poll::Ready(result),
                    _ if state.closed => return Poll::Ready(Err(CommitError::Closed)),
                    Slot::Idle | Slot::Finished(_) => {
                        state.slot = Slot::Queued(plan);
                        sent = true;
                    }
                    Slot::Queued(_) | Slot::Rendering => {}
                }
                mem::swap(&mut state.waiter, &mut waiter);
                Poll::Pending
            })
        })
        .await
    }
}

/// Render-side endpoint. Its methods complete synchronously.
#[derive(Debug)]
pub struct Host<'a, P> {
    queue: &'a EditQueue<P>,
    revision: Revision,
    awaiting_output: bool,
}

impl<P: Copy> Host<'_, P> {
    /// Apply at most one edit at a block boundary and return the active revision.
    /// Tag the rendered block with this revision. The callback runs outside
    /// the critical section and should preserve the active program on error,
    /// as [`Patch::replace`](crate::bytecode::Patch::replace) does.
    /// Successful edits increment the session's wrapping revision counter.
    pub fn begin_block(
        &mut self,
        apply: impl FnOnce(P) -> Result<(), bytecode::Error>,
    ) -> Revision {
        let plan = self.queue.with(|state| match state.slot {
            Slot::Queued(plan) => {
                state.slot = Slot::Rendering;
                Some(plan)
            }
            _ => None,
        });
        if let Some(plan) = plan {
            match apply(plan) {
                Ok(()) => {
                    self.revision.0 = self.revision.0.wrapping_add(1);
                    self.awaiting_output = true;
                }
                Err(error) => self.finish(Err(CommitError::Rejected(error))),
            }
        }
        self.revision
    }

    /// Report a block accepted by the output, with the revision used to render
    /// it in this session and its first frame. Older revisions and repeat submissions are ignored.
    /// Omit this call for a missed or unsubmitted block.
    pub fn submitted(&mut self, revision: Revision, first_frame: u64) {
        if self.awaiting_output && revision == self.revision {
            self.awaiting_output = false;
            self.finish(Ok(Applied {
                revision,
                first_frame,
            }));
        }
    }

    fn finish(&self, result: Result<Applied, CommitError>) {
        self.queue
            .update_and_wake(|state| state.slot = Slot::Finished(result));
    }
}

impl<P> Drop for Host<'_, P> {
    fn drop(&mut self) {
        self.queue.update_and_wake(|state| state.closed = true);
    }
}
