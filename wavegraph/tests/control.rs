//! Exercise the async editor and the synchronous render host on the same timeline.
#![cfg(feature = "async")]

extern crate alloc;

use alloc::{sync::Arc, task::Wake};
use core::{
    future::Future,
    pin::pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, Waker},
};
use wavegraph::{
    Event, Process,
    bytecode::{Error, Program},
    control::{Applied, CommitError, EditQueue},
    dsp::Software,
};
use wavegraph_monosynth::{Message, Monosynth, Tuning, Waveform};
use wavegraph_sample::I24;

#[derive(Default)]
struct Notify(AtomicBool);

impl Wake for Notify {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[test]
fn a_held_note_survives_rejection_and_a_late_first_block() -> Result<(), Error> {
    let dry = Program::<2, 1>::decode(&[1, 1, 0, 0, 1])?;
    let half = Program::<2, 1>::decode(&[1, 1, 0, 0, 1, 2, 0, 64])?;
    let tuning = Tuning::new(1760, 1).expect("four samples per A4 cycle");
    let mut synth = Monosynth::new(&tuning);
    let mut nodes: [&mut dyn Process<Message, I24>; 1] = [&mut synth];
    let mut patch = dry.bind(&mut nodes)?;
    let mut buses = [[I24::default(); 3]; 2];
    let mut queue = EditQueue::new();
    let (mut control, mut host) = queue.split();
    let notify = Arc::new(Notify::default());
    let waker = Waker::from(Arc::clone(&notify));
    let mut cx = Context::from_waker(&waker);

    let initial = host.begin_block(|plan| patch.replace(plan, buses.len()));
    let notes = [
        Event {
            frame: 0,
            port: 0,
            message: Message::Waveform(Waveform::Sawtooth),
        },
        Event {
            frame: 0,
            port: 0,
            message: Message::NoteOn {
                note: 69,
                velocity: 8,
            },
        },
    ];
    patch.render(&mut Software, &mut buses, &[&notes], |_, e| Err(e))?;
    assert_eq!(buses[0].map(I24::inner), [-32768, -16384, 0]);

    {
        let mut rejected = pin!(control.commit(&half));
        assert!(rejected.as_mut().poll(&mut cx).is_pending());
        assert_eq!(host.begin_block(|plan| patch.replace(plan, 1)), initial);
        assert!(notify.0.swap(false, Ordering::Relaxed));
        assert_eq!(
            rejected.as_mut().poll(&mut cx),
            Poll::Ready(Err(CommitError::Rejected(Error::Buffers)))
        );
    }

    let mut edit = pin!(control.commit(&half));
    assert!(edit.as_mut().poll(&mut cx).is_pending());
    let revision = host.begin_block(|plan| patch.replace(plan, buses.len()));
    assert_ne!(revision, initial);
    // An older output completes after the edit. The first new block misses its deadline.
    host.submitted(initial, 0);
    patch.render(&mut Software, &mut buses, &[], |_, e| Err(e))?;
    assert_eq!(buses[1].map(I24::inner), [8192, -16384, -8192]);
    assert!(!notify.0.load(Ordering::Relaxed));
    assert!(edit.as_mut().poll(&mut cx).is_pending());

    assert_eq!(
        host.begin_block(|plan| patch.replace(plan, buses.len())),
        revision
    );
    patch.render(&mut Software, &mut buses, &[], |_, e| Err(e))?;
    assert_eq!(buses[1].map(I24::inner), [0, 8192, -16384]);
    host.submitted(revision, 6);
    host.submitted(revision, 9);
    drop(host);
    assert!(notify.0.load(Ordering::Relaxed));
    assert_eq!(
        edit.as_mut().poll(&mut cx),
        Poll::Ready(Ok(Applied {
            revision,
            first_frame: 6
        }))
    );
    Ok(())
}

#[test]
fn dropping_waiters_keeps_accepted_edits_paired_with_their_receipts() {
    let mut queue = EditQueue::new();
    let (mut control, mut host) = queue.split();
    let mut cx = Context::from_waker(Waker::noop());
    {
        let mut abandoned = pin!(control.commit(1));
        assert!(abandoned.as_mut().poll(&mut cx).is_pending());
    }
    {
        let mut waiting_for_space = pin!(control.commit(2));
        assert!(waiting_for_space.as_mut().poll(&mut cx).is_pending());
    }
    let mut next = pin!(control.commit(3));
    assert!(next.as_mut().poll(&mut cx).is_pending());
    let mut received = Vec::new();
    let mut apply = |id| {
        received.push(id);
        Ok(())
    };
    let first = host.begin_block(&mut apply);
    host.submitted(first, 128);
    assert!(next.as_mut().poll(&mut cx).is_pending());
    let second = host.begin_block(&mut apply);
    assert_ne!(first, second);
    host.submitted(first, 256);
    assert!(next.as_mut().poll(&mut cx).is_pending());
    host.submitted(second, 384);
    assert_eq!(
        next.as_mut().poll(&mut cx),
        Poll::Ready(Ok(Applied {
            revision: second,
            first_frame: 384
        }))
    );
    assert_eq!(received, [1, 3]);
}

#[test]
fn dropping_the_host_wakes_the_waiter_and_closes_the_queue() {
    let mut queue = EditQueue::new();
    let (mut control, host) = queue.split();
    let notify = Arc::new(Notify::default());
    let waker = Waker::from(Arc::clone(&notify));
    let mut cx = Context::from_waker(&waker);
    {
        let mut waiting = pin!(control.commit(1));
        assert!(waiting.as_mut().poll(&mut cx).is_pending());
        drop(host);
        assert!(notify.0.load(Ordering::Relaxed));
        assert_eq!(
            waiting.as_mut().poll(&mut cx),
            Poll::Ready(Err(CommitError::Closed))
        );
    }
    assert_eq!(
        pin!(control.commit(2)).poll(&mut cx),
        Poll::Ready(Err(CommitError::Closed))
    );
}
