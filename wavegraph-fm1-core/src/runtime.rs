//! FM-1 transport and DAC service.
use crate::{
    AudioBlock, Job, Render,
    control::{Host, Revision},
    handoff::Handoff,
    peer,
};
use core::sync::atomic::{AtomicU32, Ordering, fence};
use fm1_audio::{MasterLevel, Output, Refill};
use fm1_rticx::{
    bsp::time::{Timers, cycles},
    time::{DeadlineClock, ExtU32, Timebase},
};
use heapless::mpmc::Queue;
use static_cell::ConstStaticCell;
use wavegraph_audio::{Playback, fm1::Dac};
use wavegraph_dsp::mailbox::{Mailbox, Receiver, Sender};

type BlockJob<B> = Job<&'static mut B>;

struct Worker<R: Render> {
    renderer: R,
    jobs: Receiver<'static, BlockJob<R::Block>>,
    completed: Sender<'static, BlockJob<R::Block>>,
}

/// Emit the single CPU1 entry point and its typed static storage in an FM-1 binary.
#[macro_export]
macro_rules! dsp_worker {
    ($vis:vis static $name:ident: $renderer:ty) => {
        $vis static $name: $crate::Runtime<$renderer> = $crate::Runtime::new();
        // SAFETY: the BSP stack switch enters this symbol once on CPU1.
        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".cpu1.text.wavegraph_worker")]
        extern "C" fn wavegraph_dsp_main() -> ! {
            $name.run()
        }
    };
}

/// Boot-time transfer of a renderer and its two single-slot mailboxes.
/// Declare once with [`dsp_worker!`](crate::dsp_worker).
pub struct Runtime<R: Render> {
    boot: Queue<Worker<R>, 2>,
    ready: AtomicU32,
    jobs: ConstStaticCell<Mailbox<BlockJob<R::Block>>>,
    done: ConstStaticCell<Mailbox<BlockJob<R::Block>>>,
}

impl<R: Render> core::fmt::Debug for Runtime<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

impl<R: Render> Runtime<R> {
    #[doc(hidden)]
    #[must_use]
    #[expect(deprecated, reason = "the single enqueue precedes the single dequeue")]
    pub const fn new() -> Self {
        Self {
            boot: Queue::new(),
            ready: AtomicU32::new(0),
            jobs: ConstStaticCell::new(Mailbox::new()),
            done: ConstStaticCell::new(Mailbox::new()),
        }
    }

    /// Transfer the graph to CPU1 after a successful [`peer::start`].
    /// CPU0 keeps the edit host. Use [`Driver::request`] to prime the first job.
    ///
    /// # Panics
    /// Called more than once per boot.
    pub fn connect<P: Copy>(
        &'static self,
        renderer: R,
        slot: &'static mut R::Block,
        host: Host<'static, P>,
    ) -> Driver<R::Block, P> {
        let (send, jobs) = self.jobs.take().split();
        let (completed, recv) = self.done.take().split();
        self.boot
            .enqueue(Worker {
                renderer,
                jobs,
                completed,
            })
            .ok()
            .expect("one renderer per boot");
        fence(Ordering::SeqCst);
        self.ready.store(0x57a6_c319, Ordering::Release);
        fence(Ordering::SeqCst);
        Driver {
            handoff: Handoff::new(slot),
            jobs: send,
            done: recv,
            first: None,
            host,
            missing_since: None,
        }
    }

    #[inline(never)]
    fn take_worker(&self) -> Worker<R> {
        self.boot.dequeue().expect("CPU0 published the renderer")
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn run(&'static self) -> ! {
        while self.ready.load(Ordering::Acquire) != 0x57a6_c319 {
            core::hint::spin_loop();
        }
        fence(Ordering::SeqCst);
        let mut worker = self.take_worker();
        loop {
            if let Some(job) = worker.jobs.try_recv() {
                worker.renderer.render(job.first_frame(), job.slot);
                worker
                    .completed
                    .try_send(job)
                    .ok()
                    .expect("one outstanding job");
            } else {
                core::hint::spin_loop();
            }
        }
    }
}

impl<R: Render> Default for Runtime<R> {
    fn default() -> Self {
        Self::new()
    }
}

/// CPU1 was held after a terminal failure. Drop the driver to close pending edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Fault {
    /// The BSP reported a CPU1 exception or invalid status.
    Worker = 4,
    /// CPU1 did not return the slot for one second.
    Stalled = 5,
}

fn check(healthy: bool, elapsed: u32) -> Result<(), Fault> {
    let fault = if !healthy {
        Fault::Worker
    } else if elapsed >= Timers::CRYSTAL_HZ {
        Fault::Stalled
    } else {
        return Ok(());
    };
    peer::stop();
    Err(fault)
}

/// Outcome of a serviced DMA half, with read-only access to returned application data.
#[derive(Debug)]
pub struct Refilled<'a, B> {
    /// Hardware submission status (Written or Late).
    pub result: Refill,
    /// The returned block matched this half's frame and the interval was unambiguous.
    pub current: bool,
    /// The DMA release interval included missed halves or was ambiguous.
    pub dma_gap: bool,
    /// Returned job, including stale results for diagnostics.
    pub completed: Option<Job<&'a B>>,
}

/// The CPU0 side of a render session. Move it from the startup task into ALNK0.
#[derive(Debug)]
pub struct Driver<B: AudioBlock, P: 'static> {
    handoff: Handoff<'static, B>,
    jobs: Sender<'static, BlockJob<B>>,
    done: Receiver<'static, BlockJob<B>>,
    first: Option<BlockJob<B>>,
    host: Host<'static, P>,
    missing_since: Option<u32>,
}

impl<B: AudioBlock, P: Copy> Driver<B, P> {
    /// Snapshot application inputs and apply edits only when the unique slot is free.
    /// Call once at startup and once after each serviced half.
    pub fn request(&mut self, prepare: impl FnOnce(&mut B, &mut Host<'_, P>) -> Revision) {
        if let Some(job) = self.handoff.issue(|block| prepare(block, &mut self.host)) {
            self.jobs
                .try_send(job)
                .ok()
                .expect("free slot implies empty job queue");
        }
    }

    /// Wait for the first result before starting the audio clock. TIMER4 and the
    /// RTICX deadline clock must already be running.
    ///
    /// # Errors
    /// Returns the terminal worker fault after holding CPU1.
    pub async fn wait_first(&mut self) -> Result<(), Fault> {
        let start = cycles();
        while !self.poll_first(start)? {
            DeadlineClock::delay(250.micros()).await;
        }
        Ok(())
    }

    /// Prime synchronously in RTICX init with TIMER4 running and DMA still idle.
    ///
    /// # Errors
    /// Returns the terminal worker fault after holding CPU1.
    pub fn wait_first_blocking(&mut self) -> Result<(), Fault> {
        let start = cycles();
        while !self.poll_first(start)? {
            core::hint::spin_loop();
        }
        Ok(())
    }

    fn poll_first(&mut self, start: u32) -> Result<bool, Fault> {
        check(peer::healthy(), cycles().wrapping_sub(start))?;
        if self.first.is_none() {
            self.first = self.done.try_recv();
        }
        Ok(self.first.is_some())
    }

    /// Service ALNK0 once. A miss leaves the cleared half silent. Only a current
    /// block written to the DAC acknowledges an edit. None means no half was due.
    ///
    /// # Errors
    /// A terminal fault holds CPU1. Drop the driver and service silence thereafter.
    pub fn refill(
        &mut self,
        output: &mut Output,
        master: MasterLevel,
    ) -> Result<Option<Refilled<'_, B>>, Fault> {
        let started = cycles();
        let healthy = peer::healthy();
        let mut done = None;
        let mut current = false;
        let mut dma_gap = false;
        let Ok(result) = Dac(output, master).render(|out, interval| {
            dma_gap = self.handoff.observe(interval);
            if healthy {
                done = self.first.take().or_else(|| self.done.try_recv());
                current = self.handoff.fill(out, done.as_ref());
            }
        });
        let result = result.into();
        if result == Refill::WouldBlock {
            return Ok(None);
        }
        if done.is_some() {
            self.missing_since = None;
        } else {
            check(
                healthy,
                started.wrapping_sub(*self.missing_since.get_or_insert(started)),
            )?;
        }
        let completed = self.handoff.finish(&mut self.host, result, &mut done);
        Ok(Some(Refilled {
            result,
            current,
            dma_gap,
            completed,
        }))
    }
}
