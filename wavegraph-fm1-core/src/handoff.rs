//! One owned DSP slot. ALNK0 is the sole owner of the deadline and the edit host.
use crate::AudioBlock;
use fm1_audio::{OUTPUT_FRAMES, OutputBlock, Refill, RefillInterval};
use wavegraph_dsp::control::{Host, Revision};

/// One render request tagged with its output frame and graph revision.
#[derive(Clone, Copy, Debug)]
pub struct Job<S> {
    first_frame: u64,
    revision: Revision,
    /// Application block owned by this job.
    pub slot: S,
}

impl<S> Job<S> {
    /// First sample frame on the DAC timeline.
    #[must_use]
    pub fn first_frame(&self) -> u64 {
        self.first_frame
    }

    /// Low 32 bits of the block number, for the diagnostics display.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::arithmetic_side_effects,
        reason = "A nonzero block length and a wrapping display counter"
    )]
    #[must_use]
    pub fn sequence(&self) -> u32 {
        (self.first_frame / OUTPUT_FRAMES as u64) as u32
    }

    /// Graph revision used to render this block.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }
}

/// Time since CPU0 last wrote current PCM, independent of worker activity.
#[derive(Debug, Default, Clone, Copy)]
pub enum OutputHealth {
    /// The last block was accepted.
    #[default]
    Current,
    /// Tick when the current uninterrupted gap began.
    Gap(u32),
    /// No current PCM for the configured timeout.
    Starved,
}

impl OutputHealth {
    /// Return (first missed block, timeout reached), accounting for tick wrap.
    pub fn update(&mut self, now: u32, accepted: bool, timeout: u32) -> (bool, bool) {
        if accepted {
            *self = Self::Current;
            return (false, false);
        }
        let first = matches!(self, Self::Current);
        if first {
            *self = Self::Gap(now);
        }
        if matches!(self, Self::Gap(start) if now.wrapping_sub(*start) >= timeout) {
            *self = Self::Starved;
        }
        (first, matches!(self, Self::Starved))
    }
}

#[derive(Debug)]
pub(super) struct Handoff<'a, B> {
    free: Option<&'a mut B>,
    frame: u64,
    uncertain: bool,
}

impl<'a, B: AudioBlock> Handoff<'a, B> {
    pub(super) fn new(slot: &'a mut B) -> Self {
        Self {
            free: Some(slot),
            frame: 0,
            uncertain: false,
        }
    }

    /// Issue at most one job for this output half. The worker owns the slot
    /// until it returns the tagged completion, however many deadlines pass.
    pub(super) fn issue(
        &mut self,
        prepare: impl FnOnce(&mut B) -> Revision,
    ) -> Option<Job<&'a mut B>> {
        let slot = self.free.take()?;
        let revision = prepare(slot);
        Some(Job {
            first_frame: self.frame,
            revision,
            slot,
        })
    }

    /// Account for releases coalesced before this callback. finish() counts the
    /// serviced half for both Written and Late. An ambiguous interval reanchors
    /// the logical clock without claiming a count for unobserved releases.
    pub(super) fn observe(&mut self, interval: RefillInterval) -> bool {
        self.uncertain = interval == RefillInterval::Uncertain;
        match interval {
            RefillInterval::Elapsed(n) => {
                #[allow(
                    clippy::arithmetic_side_effects,
                    reason = "Nonzero u32 minus one, times 128 frames, fits u64."
                )]
                let skipped = u64::from(n.get() - 1) * OUTPUT_FRAMES as u64;
                self.frame = self.frame.wrapping_add(skipped);
                skipped != 0
            }
            RefillInterval::First => false,
            RefillInterval::Uncertain => true,
        }
    }

    fn current(&self, job: &Job<&mut B>) -> bool {
        !self.uncertain && job.first_frame == self.frame
    }

    /// Output::render supplies a cleared half. A missing or expired job leaves it silent.
    pub(super) fn fill(&self, out: &mut OutputBlock, done: Option<&Job<&mut B>>) -> bool {
        if let Some(job) = done.filter(|j| self.current(j)) {
            job.slot.write(out);
            true
        } else {
            false
        }
    }

    /// Only a timely result written to the DAC can advance the edit receipt.
    pub(super) fn finish<P: Copy>(
        &mut self,
        host: &mut Host<'_, P>,
        result: Refill,
        done: &mut Option<Job<&'a mut B>>,
    ) -> Option<Job<&B>> {
        if result == Refill::WouldBlock {
            return None;
        }
        if result == Refill::Written
            && let Some(job) = done.as_ref().filter(|job| self.current(job))
        {
            host.submitted(job.revision, job.first_frame);
        }
        self.frame = self.frame.wrapping_add(OUTPUT_FRAMES as u64);
        let Job {
            slot,
            revision,
            first_frame,
        } = done.take()?;
        Some(Job {
            slot: self.free.insert(slot),
            revision,
            first_frame,
        })
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
    use wavegraph_dsp::control::EditQueue;

    #[test]
    fn coalesced_or_ambiguous_release_rejects_ready_pcm_and_defers_its_edit() {
        for (interval, next_block) in [
            (
                RefillInterval::Elapsed(2.try_into().expect("two halves")),
                2,
            ),
            (
                RefillInterval::Elapsed(3.try_into().expect("three halves")),
                3,
            ),
            (RefillInterval::Uncertain, 1),
        ] {
            let mut queue = EditQueue::new();
            let (mut editor, mut host) = queue.split();
            let mut receipt = pin!(editor.commit(true));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(receipt.as_mut().poll(&mut cx).is_pending());

            let mut pcm = [7; OUTPUT_FRAMES];
            let mut h = Handoff::new(&mut pcm);
            assert!(!h.observe(RefillInterval::First));
            let mut done = h.issue(|_| host.begin_block(|_| Ok(())));
            let mut out = [[0; 2]; OUTPUT_FRAMES];
            assert!(h.observe(interval));
            assert!(!h.fill(&mut out, done.as_ref()));
            assert_eq!(out, [[0; 2]; OUTPUT_FRAMES]);
            h.finish(&mut host, Refill::Written, &mut done);
            assert!(receipt.as_mut().poll(&mut cx).is_pending());

            let mut fresh = h.issue(|_| host.begin_block(|_| unreachable!()));
            assert!(!h.observe(RefillInterval::Elapsed(core::num::NonZeroU32::MIN)));
            assert!(h.fill(&mut out, fresh.as_ref()));
            assert_eq!(out, [[7; 2]; OUTPUT_FRAMES]);
            h.finish(&mut host, Refill::Written, &mut fresh);
            assert!(
                matches!(receipt.as_mut().poll(&mut cx), Poll::Ready(Ok(applied))
                if applied.first_frame == next_block * OUTPUT_FRAMES as u64)
            );
        }
    }

    #[test]
    fn continuously_late_worker_alarms_then_acknowledges_the_first_usable_block() {
        let mut queue = EditQueue::new();
        let (mut editor, mut host) = queue.split();
        let mut receipt = pin!(editor.commit(true));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(receipt.as_mut().poll(&mut cx).is_pending());
        let mut pcm = [1; OUTPUT_FRAMES];
        let mut handoff = Handoff::new(&mut pcm);
        let mut health = OutputHealth::default();
        let mut job = None;
        let mut out = [[0; 2]; OUTPUT_FRAMES];
        let mut expired = 0;
        // TIMER4 wraps during a run with one stale completion every two deadlines.
        let start = u32::MAX - 500;
        for n in 0u32..2000 {
            if job.is_none() {
                job = handoff.issue(|_| host.begin_block(|_| Ok(())));
            }
            let mut done = if n % 2 == 1 { job.take() } else { None };
            expired += u32::from(done.is_some());
            let accepted = handoff.fill(&mut out, done.as_ref());
            assert!(!accepted);
            assert_eq!(
                health.update(start.wrapping_add(n), accepted, 345),
                (n == 0, n >= 345)
            );
            handoff.finish(&mut host, Refill::Written, &mut done);
            assert!(receipt.as_mut().poll(&mut cx).is_pending());
        }
        assert_eq!(expired, 1000);
        assert_eq!(health.update(start, false, 345), (false, true)); // full counter wrap
        assert_eq!(out, [[0; 2]; OUTPUT_FRAMES]);
        let mut done = handoff.issue(|_| host.begin_block(|_| unreachable!()));
        let accepted = handoff.fill(&mut out, done.as_ref());
        assert!(accepted);
        assert_eq!(
            health.update(start.wrapping_add(2000), accepted, 345),
            (false, false)
        );
        assert_eq!(out, [[1; 2]; OUTPUT_FRAMES]);
        handoff.finish(&mut host, Refill::Written, &mut done);
        assert!(
            matches!(receipt.as_mut().poll(&mut cx), Poll::Ready(Ok(applied))
            if applied.first_frame == 2000 * OUTPUT_FRAMES as u64)
        );
    }

    #[test]
    fn missed_deadline_keeps_slot_and_receipt_until_fresh_pcm_is_written() {
        let mut queue = EditQueue::new();
        let (mut editor, mut host) = queue.split();
        let mut receipt = pin!(editor.commit(true));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(receipt.as_mut().poll(&mut cx).is_pending());

        let mut pcm = [0; OUTPUT_FRAMES];
        let mut h = Handoff::new(&mut pcm);
        let job = h
            .issue(|_| host.begin_block(|_| Ok(())))
            .expect("first job");
        let revision = job.revision();
        assert_eq!(job.sequence(), 0);
        job.slot[..64].fill(111); // worker paused partway through its writes
        let mut out = [[0; 2]; OUTPUT_FRAMES];
        h.finish(&mut host, Refill::WouldBlock, &mut None);
        assert!(!h.fill(&mut out, None));
        assert_eq!(out, [[0; 2]; OUTPUT_FRAMES]);
        h.finish(&mut host, Refill::Written, &mut None);
        assert!(h.issue(|_| unreachable!()).is_none());
        assert!(receipt.as_mut().poll(&mut cx).is_pending());

        job.slot[64..].fill(222);
        assert!(!h.fill(&mut out, Some(&job)));
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
            assert!(h.fill(&mut out, Some(&job)));
            assert_eq!(out, [[333; 2]; OUTPUT_FRAMES]);
            let first_frame = job.first_frame();
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
                    Poll::Ready(Ok(wavegraph_dsp::control::Applied {
                        revision,
                        first_frame
                    })),
                );
                assert_eq!(first_frame, 3 * OUTPUT_FRAMES as u64);
            }
        }
    }
}
