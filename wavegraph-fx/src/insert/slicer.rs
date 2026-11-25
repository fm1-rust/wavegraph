//! A transport-synchronized, sixteen-step amplitude gate.
use super::mul_q15;
use crate::{BeatClock, BeatFx, StepPattern};
use wavegraph_lfo::{Phase, PhaseControl};
use wavegraph_sample::I24;

/// Slice depth and bypass for one input. The step mask is independently editable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlicerControls {
    /// Attenuation on unlit steps, from unity (0) to silence (128).
    pub depth: u8,
    /// Apply the step pattern when enabled.
    pub enabled: bool,
}

impl SlicerControls {
    /// Full-depth gating, initially bypassed.
    pub const INITIAL: Self = Self {
        depth: 128,
        enabled: false,
    };
}

/// A mono Q23 gate. One turn of the shared Beat FX clock plays all sixteen cells.
#[derive(Clone, Copy, Debug)]
pub struct Slicer {
    rate_hz: u32,
    phase: Phase,
    pattern: StepPattern,
    controls: SlicerControls,
    gain: i32,
    slew: i32,
}

impl Slicer {
    /// Construct a bypassed gate with approximately half-millisecond edges.
    #[must_use]
    pub fn new(rate_hz: u32) -> Option<Self> {
        if rate_hz == 0 {
            return None;
        }
        Some(Self {
            rate_hz,
            phase: Phase::default(),
            pattern: StepPattern::FIRST_HALF,
            controls: SlicerControls::INITIAL,
            gain: 32768,
            slew: i32::try_from(32768u32.div_ceil(rate_hz.div_ceil(2000))).ok()?,
        })
    }

    /// The grid cell that contains this Q0.32 cycle position.
    #[must_use]
    pub const fn step_at(phase: u32) -> u8 {
        (phase >> 28) as u8
    }

    /// Change depth and bypass without restarting the pattern.
    pub fn set_controls(&mut self, controls: SlicerControls) {
        self.controls = controls;
    }

    /// Set the sixteen pass/rest cells, with bit zero at the downbeat.
    pub fn set_pattern(&mut self, pattern: StepPattern) {
        self.pattern = pattern;
    }

    /// Apply an already converted master clock to an audio span.
    pub fn set_phase(&mut self, control: PhaseControl) {
        self.phase.control(control);
    }

    pub(super) fn set_clock(&mut self, clock: BeatClock) {
        self.sync(clock, None);
    }

    pub(super) fn anchor(&mut self, phase: u32) {
        self.phase.seek(phase);
    }

    pub(super) fn set_enabled(&mut self, enabled: bool) {
        self.controls.enabled = enabled;
    }

    /// Apply the current cell, then advance by one audio frame.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Gain and target are bounded to 0..=32768"
    )]
    pub fn tick(&mut self, sample: I24) -> I24 {
        let pass = self.pattern.active(Self::step_at(self.phase.position()));
        self.phase.advance(1);
        let target = if self.controls.enabled && !pass {
            32768 - (i32::from(self.controls.depth.min(128)) << 8)
        } else {
            32768
        };
        self.gain += (target - self.gain).clamp(-self.slew, self.slew);
        match self.gain {
            32768 => sample,
            0 => I24::default(),
            g => I24::new_unchecked(mul_q15(sample.inner(), g)),
        }
    }

    /// Gate a strip in place. A fully bypassed strip advances in constant time.
    pub fn process_in_place(&mut self, samples: &mut [I24]) {
        if self.gain == 32768
            && (!self.controls.enabled
                || self.controls.depth == 0
                || self.pattern == StepPattern::ALL)
        {
            self.phase.advance(samples.len());
        } else {
            for sample in samples {
                *sample = self.tick(*sample);
            }
        }
    }
}

impl BeatFx for Slicer {
    fn sync(&mut self, clock: BeatClock, phase: Option<u32>) {
        self.set_phase(clock.lfo(self.rate_hz, phase));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BeatDivision, Message};
    use wavegraph_core::{Event, Process, ProcessContext};

    #[test]
    fn sample_offset_phase_and_bypass_ramp_at_both_polarities() {
        for signal in [-(1 << 23), (1 << 23) - 1] {
            let mut fx = Slicer::new(8000).expect("sample rate");
            let input = [I24::new_unchecked(signal); 16];
            let mut out = input;
            let events = [
                (
                    0,
                    Message::BeatTime(BeatClock {
                        bpm: 120,
                        beats: BeatDivision::ONE,
                    }),
                ),
                (0, Message::Enabled(true)),
                (4, Message::BeatPhase(1 << 31)),
                (10, Message::Enabled(false)),
            ]
            .map(|(frame, message)| Event {
                frame,
                port: 0,
                message,
            });
            fx.process(&mut ProcessContext {
                frames: input.len(),
                inputs: &[&input],
                outputs: &mut [&mut out],
                events: &events,
                emit: &mut Err,
            });
            let quarters = [4, 4, 4, 4, 3, 2, 1, 0, 0, 0, 1, 2, 3, 4, 4, 4];
            assert_eq!(out.map(I24::inner), quarters.map(|q| (signal * q) >> 2));
        }
    }

    #[test]
    fn all_sixteen_cells_follow_the_cycle_across_unequal_blocks() {
        let clock = BeatClock {
            bpm: 120,
            beats: BeatDivision::ONE,
        };
        let mut fx = Slicer::new(32_768).expect("sample rate");
        let pattern = StepPattern::from_bits(0x9125);
        fx.set_controls(SlicerControls {
            enabled: true,
            depth: 64,
        });
        fx.set_pattern(pattern);
        fx.sync(clock, Some(0));
        let dry = I24::new_unchecked(1 << 20);
        let mut split = fx;
        let mut block = [dry; 73];
        for start in (0..32_768).step_by(block.len()) {
            block.fill(dry);
            split.process_in_place(&mut block);
            for (offset, sample) in block.iter().enumerate() {
                let frame = start + offset;
                assert_eq!(*sample, fx.tick(dry));
                if frame % 1024 >= 17 {
                    let open =
                        pattern.active(u8::try_from((frame / 1024) & 15).expect("grid cell"));
                    assert_eq!(sample.inner(), if open { 1 << 20 } else { 1 << 19 });
                }
            }
        }
        // A mask edit takes effect in the current cell without restarting the clock.
        fx.set_pattern(StepPattern::from_bits(0));
        for _ in 0..17 {
            fx.tick(dry);
        }
        assert_eq!(fx.tick(dry).inner(), 1 << 19);
        fx.set_controls(SlicerControls {
            enabled: true,
            depth: 0,
        });
        for _ in 0..17 {
            fx.tick(dry);
        }
        assert_eq!(fx.tick(dry), dry);
    }
}
