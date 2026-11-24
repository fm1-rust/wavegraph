//! One owned DSP slot. ALNK0 is the sole owner of the deadline and the edit host.
use fm1_audio::{OUTPUT_FRAMES, OutputBlock, Refill};
use wavegraph::control::{Host, Revision};

pub(super) struct Job<S> {
    first_frame: u64,
    revision: Revision,
    pub(super) slot: S,
}

impl<S> Job<S> {
    /// Low 32 bits of the block number, for the diagnostics display.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::arithmetic_side_effects,
        reason = "A nonzero block length and a wrapping display counter"
    )]
    pub(super) fn sequence(&self) -> u32 {
        (self.first_frame / OUTPUT_FRAMES as u64) as u32
    }

    pub(super) fn revision(&self) -> Revision {
        self.revision
    }
}

pub(super) struct Handoff<S> {
    free: Option<S>,
    frame: u64,
}

impl<S> Handoff<S> {
    pub(super) fn new(slot: S) -> Self {
        Self {
            free: Some(slot),
            frame: 0,
        }
    }

    /// Issue at most one job for this output half. The worker owns the slot
    /// until it returns the tagged completion, however many deadlines pass.
    pub(super) fn issue(&mut self, prepare: impl FnOnce(&mut S) -> Revision) -> Option<Job<S>> {
        let mut slot = self.free.take()?;
        let revision = prepare(&mut slot);
        Some(Job {
            first_frame: self.frame,
            revision,
            slot,
        })
    }

    /// Output::render supplies a cleared half. A missing or expired job leaves it silent.
    pub(super) fn fill(
        &self,
        out: &mut OutputBlock,
        done: Option<&Job<S>>,
        pcm: impl FnOnce(&S) -> &[i32; OUTPUT_FRAMES],
    ) -> bool {
        if let Some(job) = done.filter(|j| j.first_frame == self.frame) {
            for (frame, &sample) in out.iter_mut().zip(pcm(&job.slot)) {
                *frame = [sample; 2];
            }
            true
        } else {
            false
        }
    }

    /// Only a timely result written to the DAC can advance the edit receipt.
    pub(super) fn finish(
        &mut self,
        host: &mut Host<'_, bool>,
        result: Refill,
        done: &mut Option<Job<S>>,
    ) {
        if result == Refill::WouldBlock {
            return;
        }
        if let Some(job) = done.take() {
            if result == Refill::Written && job.first_frame == self.frame {
                host.submitted(job.revision, job.first_frame);
            }
            self.free = Some(job.slot);
        }
        self.frame = self.frame.wrapping_add(OUTPUT_FRAMES as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    use wavegraph::control::EditQueue;

    #[test]
    fn missed_deadline_keeps_slot_and_receipt_until_fresh_pcm_is_written() {
        let mut queue = EditQueue::new();
        let (mut editor, mut host) = queue.split();
        let mut receipt = pin!(editor.commit(true));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(receipt.as_mut().poll(&mut cx).is_pending());

        let mut h = Handoff::new([0; OUTPUT_FRAMES]);
        let mut job = h
            .issue(|_| host.begin_block(|_| Ok(())))
            .expect("first job");
        let revision = job.revision();
        assert_eq!(job.sequence(), 0);
        job.slot[..64].fill(111); // worker paused partway through its writes
        let mut out = [[0; 2]; OUTPUT_FRAMES];
        h.finish(&mut host, Refill::WouldBlock, &mut None);
        assert!(!h.fill(&mut out, None, |s| s));
        assert_eq!(out, [[0; 2]; OUTPUT_FRAMES]);
        h.finish(&mut host, Refill::Written, &mut None);
        assert!(h.issue(|_| unreachable!()).is_none());
        assert!(receipt.as_mut().poll(&mut cx).is_pending());

        job.slot[64..].fill(222);
        assert!(!h.fill(&mut out, Some(&job), |s| s));
        assert_eq!(out, [[0; 2]; OUTPUT_FRAMES]);
        h.finish(&mut host, Refill::Written, &mut Some(job));
        assert!(receipt.as_mut().poll(&mut cx).is_pending());

        for result in [Refill::Late, Refill::Written] {
            let job = h
                .issue(|pcm| {
                    pcm.fill(333);
                    host.begin_block(|_| unreachable!())
                })
                .expect("slot returned");
            assert!(h.fill(&mut out, Some(&job), |s| s));
            assert_eq!(out, [[333; 2]; OUTPUT_FRAMES]);
            let first_frame = job.first_frame;
            let mut completed = Some(job);
            h.finish(&mut host, Refill::WouldBlock, &mut completed);
            assert!(completed.is_some());
            assert!(receipt.as_mut().poll(&mut cx).is_pending());
            h.finish(&mut host, result, &mut completed);
            assert!(completed.is_none());
            if result == Refill::Late {
                assert!(receipt.as_mut().poll(&mut cx).is_pending());
            } else {
                assert_eq!(
                    receipt.as_mut().poll(&mut cx),
                    Poll::Ready(Ok(wavegraph::control::Applied {
                        revision,
                        first_frame
                    })),
                );
                assert_eq!(first_frame, 3 * OUTPUT_FRAMES as u64);
            }
        }
    }
}
