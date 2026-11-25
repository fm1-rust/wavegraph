use super::{ModulationControls, blend, clamp_pcm, mul_q7, pcm, triangle};
use wavegraph_sample::I24;

/// Mono Q23 flanger with a triangle LFO and a linearly interpolated PCM16 tap.
/// Full depth sweeps from one sample to 2 ms, capped at 63 samples (1.43 ms
/// on the FM-1). Zero depth holds the delay at the center of that range.
#[derive(Clone, Copy, Debug)]
pub struct Flanger {
    line: [i16; 64],
    write: u8,
    rate_hz: u32,
    phase: u32,
    step: u32,
    center_q8: u16,
    sweep_q8: u16,
    feedback: u8,
    wet_mix: u8,
    enabled: bool,
    mix: i32,
}

impl Flanger {
    /// Construct a flanger with an empty delay line. Returns `None` for a zero sample rate.
    #[must_use]
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "The delay in Q8 is clamped to 63 samples"
    )]
    pub fn new(rate_hz: u32) -> Option<Self> {
        if rate_hz == 0 {
            return None;
        }
        let max_q8 = (u64::from(rate_hz) * 256 / 500).clamp(256, 63 * 256);
        let mut fx = Self {
            line: [0; 64],
            write: 0,
            rate_hz,
            phase: 0,
            step: 0,
            center_q8: ((256 + max_q8) / 2) as u16,
            sweep_q8: 0,
            feedback: 0,
            wet_mix: 0,
            enabled: true,
            mix: 0,
        };
        fx.set_controls(ModulationControls::INITIAL);
        Some(fx)
    }

    /// Update the sweep, feedback and mix while preserving phase and delay history.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "The sweep fits 63 samples in Q8 and feedback is at most 96/128"
    )]
    pub fn set_controls(&mut self, controls: ModulationControls) {
        self.step = controls.phase_step(self.rate_hz);
        self.sweep_q8 =
            ((u32::from(self.center_q8 - 256) * u32::from(controls.depth.min(128))) >> 7) as u16;
        self.feedback = (u16::from(controls.feedback.min(128)) * 3 / 4) as u8;
        self.wet_mix = controls.mix.min(128);
        self.set_enabled(controls.enabled);
    }

    pub(super) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Process one signed Q23 sample. Bypass ramps to the exact dry input.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Delay is in 1..=63 samples. Two PCM16 taps interpolate to Q23 and the write is clamped."
    )]
    pub fn tick(&mut self, sample: I24) -> I24 {
        let write = usize::from(self.write);
        let mask = self.line.len() - 1;
        self.write = ((write + 1) & mask) as u8;
        self.phase = self.phase.wrapping_add(self.step);
        let target = if self.enabled { self.wet_mix } else { 0 };
        let mix = blend(&mut self.mix, i32::from(target));
        let dry = sample.inner();
        if mix == 0 {
            self.line[write] = (dry / 256) as i16;
            return sample;
        }
        let delay =
            i32::from(self.center_q8) + ((triangle(self.phase) * i32::from(self.sweep_q8)) >> 15);
        let index = write.wrapping_sub((delay >> 8) as usize) & mask;
        let newer = i32::from(self.line[index]);
        let older = i32::from(self.line[index.wrapping_sub(1) & mask]);
        let wet = (newer << 8) + (older - newer) * (delay & 255);
        // Truncate toward zero so the last negative feedback repeat decays to silence.
        self.line[write] = (clamp_pcm(dry + mul_q7(wet, i32::from(self.feedback))) / 256) as i16;
        pcm(dry + mul_q7(wet - dry, mix))
    }
}
