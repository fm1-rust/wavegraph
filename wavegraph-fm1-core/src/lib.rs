#![no_std]
//! Run an application-owned renderer on CPU1 and feed its blocks to the FM-1 DAC on CPU0.
//! See the crate README for the RTICX startup and refill sequence.

pub use fm1_audio::{OUTPUT_FRAMES, OutputBlock};
pub use wavegraph_dsp::control;

/// Application state that moves with a block between the two cores.
pub trait AudioBlock: Send + 'static {
    /// Write the rendered samples to CPU0's cleared stereo staging block before MASTER.
    fn write(&self, out: &mut OutputBlock);
}

impl AudioBlock for OutputBlock {
    fn write(&self, out: &mut OutputBlock) {
        out.copy_from_slice(self);
    }
}

impl AudioBlock for [i32; OUTPUT_FRAMES] {
    fn write(&self, out: &mut OutputBlock) {
        for (frame, &sample) in out.iter_mut().zip(self) {
            *frame = [sample; 2];
        }
    }
}

/// A synchronous DSP graph transferred to CPU1 once at boot.
pub trait Render: Send + 'static {
    /// Shared input snapshot, output samples, and optional application telemetry.
    type Block: AudioBlock;

    /// Render one DAC half. A jump in `first_frame` counts skipped samples.
    fn render(&mut self, first_frame: u64, block: &mut Self::Block);
}

#[cfg(any(target_os = "none", test))]
mod handoff;
#[cfg(any(target_os = "none", test))]
pub use handoff::{Job, OutputHealth};

#[cfg(target_os = "none")]
pub mod peer;
#[cfg(target_os = "none")]
mod runtime;
#[cfg(target_os = "none")]
pub use runtime::{Driver, Fault, Refilled, Runtime};

/// Start the FM-1 DAC muted and consume the half pending after codec startup.
/// Call on CPU0 with ALNK0 masked until the task has received its driver.
///
/// # Errors
/// Propagates the audio port's clock and busy errors.
#[cfg(target_os = "none")]
#[inline]
pub fn start_output(
    port: fm1_audio::AudioPort,
) -> Result<fm1_audio::Output, fm1_audio::StartError> {
    let mut output = fm1_audio::Output::start(port)?;
    output.render_with_interval(fm1_audio::MasterLevel::MIN, |_, _| {});
    Ok(output)
}
