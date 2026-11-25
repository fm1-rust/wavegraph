//! Fixed-point oscillator phase shared by control-rate and audio-rate LFOs.

/// A frame-timed rate change. An optional phase anchors the next sample to transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhaseControl {
    /// Fraction of a full turn per audio frame in Q0.32.
    pub increment: u32,
    /// Phase of the next sample in Q0.32, or None to preserve continuity.
    pub phase: Option<u32>,
}

/// A wrapping oscillator whose full turn is 2³² units.
#[derive(Clone, Copy, Debug, Default)]
pub struct Phase {
    position: u32,
    increment: u32,
}

impl Phase {
    /// Apply control at the start of a render span.
    pub fn control(&mut self, control: PhaseControl) {
        self.increment = control.increment;
        if let Some(position) = control.phase {
            self.position = position;
        }
    }

    /// Current position before advancing the oscillator.
    #[must_use]
    pub const fn position(self) -> u32 {
        self.position
    }

    /// Current phase increase per audio frame.
    #[must_use]
    pub const fn increment(self) -> u32 {
        self.increment
    }

    /// Seek without changing rate.
    pub fn seek(&mut self, position: u32) {
        self.position = position;
    }

    /// Advance any number of frames without evaluating the waveform.
    #[allow(clippy::cast_possible_truncation, reason = "Q0.32 phase is modular")]
    pub fn advance(&mut self, frames: usize) {
        self.position = self
            .position
            .wrapping_add(self.increment.wrapping_mul(frames as u32));
    }

    /// Triangle starting at its negative peak, for signed Q15 audio modulation.
    #[must_use]
    #[allow(clippy::arithmetic_side_effects, reason = "The high word fits i32")]
    pub fn triangle_q15(self) -> i32 {
        let ramp = i32::from((self.position >> 16) as u16);
        if ramp < 32768 {
            2 * ramp - 32768
        } else {
            98302 - 2 * ramp
        }
    }
}
