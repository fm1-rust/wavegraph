//! FM-1 CPU1 lifecycle. The BSP owns the ROM handoff, status line, exception
//! vector and hardware hold. CPU1 has one fixed entry and never enables IRQs.

use fm1_rticx::bsp;

#[link(name = "fm1_cpu1_probe", kind = "static")]
unsafe extern "C" {
    fn fm1_cpu1_probe_run(persistent: u32) -> u32;
    fn fm1_cpu1_probe_healthy() -> u32;
    fn fm1_cpu1_probe_stop();
    fn fm1_cpu1_audio_ready(reply: u32);
}

unsafe extern "C" {
    safe fn fm1_core_id() -> u32;
    fn fm1_cli();
    fn wavegraph_dsp_enter() -> !;
    fn wavegraph_cpu1_usp_free() -> u32;
    fn wavegraph_usp_free() -> u32;
    fn wavegraph_ssp_free() -> u32;
}

/// Release CPU1 once. One means the BSP verified both the challenge response
/// and the CPU1 ROM execution context. Every failure returns with CPU1 held.
///
/// # Safety
/// CPU0 only, once in RTICX init before Board::take configures TIMER4 and
/// before enabling interrupts. The BSP reset entry must have held the peer
/// before the CRT touched static storage.
pub unsafe fn start() -> u32 {
    // SAFETY: the caller provides the exclusive early boot context.
    unsafe { fm1_cpu1_probe_run(1) }
}

/// Permanently hold a failed peer before returning control to CPU0. Owned
/// Rust storage still belongs to its last owner after this call.
pub(super) fn stop() {
    assert_eq!(fm1_core_id(), 0, "only CPU0 may halt CPU1");
    // SAFETY: the native operation masks CPU0 interrupts for the hold and
    // restores the saved exception vector. It is idempotent after shutdown.
    unsafe { fm1_cpu1_probe_stop() };
}

/// Observe faults after a successful boot. The BSP status belongs to CPU0.
pub(super) fn healthy() -> bool {
    // SAFETY: CPU0 only reads the BSP-owned status via its native ABI.
    fm1_core_id() == 0 && unsafe { fm1_cpu1_probe_healthy() != 0 }
}

/// The ROM entry in SRAM invokes this symbol instead of the example DSP loop.
// SAFETY: only the checked BSP CPU1 entry calls this private C ABI.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".cpu1.text.wavegraph_ready")]
extern "C" fn fm1_cpu1_probe_word(challenge: u32) -> ! {
    // SAFETY: publish the checked response and leave the 1 KiB bootstrap
    // stack permanently. The next instruction stream is statically linked.
    unsafe {
        fm1_cpu1_audio_ready(challenge.rotate_left(7) ^ 0x000a_c791);
        wavegraph_dsp_enter()
    }
}

/// Number of contiguous untouched bytes at the bottom of the running DSP
/// stack. Only the owning core can inspect it.
///
/// # Panics
/// Called outside CPU1.
#[must_use]
pub fn cpu1_stack_free() -> u32 {
    assert_eq!(fm1_core_id(), 1, "DSP stack belongs to CPU1");
    // SAFETY: only CPU1 reads its own descending stack; bounds are fixed
    // linker symbols. The assembly scanner creates no Rust references.
    unsafe { wavegraph_cpu1_usp_free() }
}

/// Untouched bytes at the bottom of the CPU0 user and supervisor stacks, respectively.
///
/// # Panics
/// Called outside CPU0.
#[must_use]
pub fn cpu0_stack_free() -> (u32, u32) {
    assert_eq!(fm1_core_id(), 0, "CPU0 owns these stacks");
    // SAFETY: the assembly routines read only the linker-bounded stack paint
    // of the calling core, without forming references into either stack.
    unsafe { (wavegraph_usp_free(), wavegraph_ssp_free()) }
}

/// Return to m-bootloader without leaving a CPU fetching WaveGraph from flash.
pub fn download() -> ! {
    if fm1_core_id() != 0 {
        // SAFETY: CPU1 is the single writer of the diagnostic status. A
        // panicking DSP reports a fault then parks until CPU0 holds it.
        unsafe { fm1_cpu1_audio_ready(4) };
        loop {
            core::hint::spin_loop();
        }
    }
    // SAFETY: this CPU0 terminal path masks interrupts, holds the peer and
    // restores the shared exception vector before requesting a chip reset.
    unsafe {
        fm1_cli();
        fm1_cpu1_probe_stop();
    }
    bsp::enter_download_mode()
}
