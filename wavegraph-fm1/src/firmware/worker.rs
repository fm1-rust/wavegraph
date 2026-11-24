//! The checked CPU1 entry runs a fixed synchronous renderer for the entire boot.

use super::render::{RenderBlock, Renderer};
use crate::{handoff::Job, peer};
use core::sync::atomic::{AtomicU32, Ordering, fence};
use fm1_rticx::bsp::time::{Timers, cycles};
use heapless::mpmc::Queue;
use wavegraph_dsp::mailbox::{Receiver, Sender};

pub(super) type BlockJob = Job<&'static mut RenderBlock>;

pub(super) struct Worker {
    pub(super) renderer: Renderer,
    pub(super) jobs: Receiver<'static, BlockJob>,
    pub(super) completed: Sender<'static, BlockJob>,
    pub(super) release: Receiver<'static, ()>,
}

// One boot-time move into CPU1's private stack. The known ready word is
// published only after CPU0 has filled the queue. Only the bootstrap uses
// atomic read-modify-write; steady-state job transfer is strictly SPSC.
#[expect(
    deprecated,
    reason = "the one enqueue completes before the one dequeue at startup"
)]
static BOOT: Queue<Worker, 2> = Queue::new();
static READY: AtomicU32 = AtomicU32::new(0);
const INSTALLED: u32 = 0x57a6_c319;

pub(super) fn install(worker: Worker) {
    BOOT.enqueue(worker).ok().expect("one renderer per boot");
    fence(Ordering::SeqCst);
    READY.store(INSTALLED, Ordering::Release);
    fence(Ordering::SeqCst);
}

// Leave the one-shot dequeue in XIP to save worker SRAM.
#[inline(never)]
fn take_worker() -> Worker {
    BOOT.dequeue().expect("CPU0 published the renderer")
}

// SAFETY: the BSP bootstrap enters this symbol exactly once, in CPU1 user
// context, after checking its core ID and switching to the reserved stack.
#[unsafe(no_mangle)]
// Keep the idle poll in SRAM; rendering can call the DSP graph in flash.
#[unsafe(link_section = ".cpu1.text.wavegraph_worker")]
extern "C" fn wavegraph_dsp_main() -> ! {
    while READY.load(Ordering::Acquire) != INSTALLED {
        core::hint::spin_loop();
    }
    fence(Ordering::SeqCst);
    let mut worker = take_worker();
    let mut measured = None;
    loop {
        let Some(job) = worker.jobs.try_recv() else {
            core::hint::spin_loop();
            continue;
        };
        let started = cycles();
        worker.renderer.render(job.first_frame(), job.slot);
        job.slot.work = cycles().wrapping_sub(started);
        job.slot.stack_free =
            if measured.is_none_or(|at| started.wrapping_sub(at) >= Timers::CRYSTAL_HZ) {
                measured = Some(started);
                Some(peer::stack_free())
            } else {
                None
            };
        if cfg!(feature = "late-block-probe") && job.slot.hold {
            while worker.release.try_recv().is_none() {
                core::hint::spin_loop();
            }
        }
        job.slot.ready_at = cycles();
        worker
            .completed
            .try_send(job)
            .ok()
            .expect("one outstanding job");
    }
}
