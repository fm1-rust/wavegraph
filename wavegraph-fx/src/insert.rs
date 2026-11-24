//! Small, allocation-free Q23 insert effects.

use super::{Message, clamp_pcm, pcm};
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::I24;

/// Saturator controls, all in 0..=128 units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaturatorControls {
    /// Input drive, from 1x to 8x.
    pub drive: u8,
    /// Wet/dry mix.
    pub mix: u8,
    /// Wet output level, from silence to unity.
    pub level: u8,
    /// Enable with a short bypass crossfade.
    pub enabled: bool,
}

impl SaturatorControls {
    /// Factory controls.
    pub const INITIAL: Self = Self {
        drive: 32,
        mix: 128,
        level: 96,
        enabled: true,
    };
}

/// Phaser controls, all in 0..=128 units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhaserControls {
    /// LFO rate, from 0.10 to 3.94 Hz.
    pub rate: u8,
    /// Sweep depth.
    pub depth: u8,
    /// Feedback, capped at 75%.
    pub feedback: u8,
    /// Wet/dry mix.
    pub mix: u8,
    /// Enable with a short bypass crossfade.
    pub enabled: bool,
}

impl PhaserControls {
    /// Factory controls.
    pub const INITIAL: Self = Self {
        rate: 14,
        depth: 112,
        feedback: 48,
        mix: 64,
        enabled: true,
    };

    /// LFO rate in hundredths of a Hz.
    #[must_use]
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The clamped rate yields 10..=394 centihertz"
    )]
    pub fn centihz(self) -> u16 {
        10 + 3 * u16::from(self.rate.min(128))
    }
}

// Split the product into high and low limbs: exact arithmetic right shift
// without a wide multiply on the embedded target. Inputs are bounded below.
#[allow(clippy::arithmetic_side_effects)]
fn mul_q15(x: i32, gain: i32) -> i32 {
    (x >> 15) * gain + (((x & 0x7fff) * gain) >> 15)
}

#[allow(clippy::arithmetic_side_effects)]
fn mul_q7(x: i32, gain: i32) -> i32 {
    (x >> 7) * gain + (((x & 127) * gain) >> 7)
}

#[allow(clippy::arithmetic_side_effects)]
fn blend(current: &mut i32, target: i32) -> i32 {
    *current += (target - *current).clamp(-2, 2);
    *current
}

/// Memoryless, symmetric cubic soft clipper with a dry/wet crossfade.
/// No oversampling: high drive can produce aliasing.
#[derive(Clone, Copy, Debug)]
pub struct Saturator {
    controls: SaturatorControls,
    blend: i32,
}

impl Default for Saturator {
    fn default() -> Self {
        Self::new()
    }
}

impl Saturator {
    /// Construct a saturator that fades in on its first samples.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            controls: SaturatorControls::INITIAL,
            blend: 0,
        }
    }

    /// Update the controls without resetting the bypass ramp.
    pub fn set_controls(&mut self, controls: SaturatorControls) {
        self.controls = controls;
    }

    /// Process a single signed Q23 sample.
    #[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
    pub fn tick(&mut self, sample: I24) -> I24 {
        let c = self.controls;
        let mix = blend(
            &mut self.blend,
            if c.enabled {
                i32::from(c.mix.min(128))
            } else {
                0
            },
        );
        if mix == 0 {
            return sample;
        }
        let x = sample.inner();
        let driven = clamp_pcm(mul_q7(x, 128 + 7 * i32::from(c.drive.min(128))));
        let square = ((i64::from(driven) * i64::from(driven)) >> 23) as i32;
        let cube = ((i64::from(square) * i64::from(driven)) >> 23) as i32;
        let shaped = (3 * driven - cube) / 2;
        let wet = mul_q7(shaped, i32::from(c.level.min(128)));
        pcm(x + mul_q7(wet - x, mix))
    }
}

/// Four first-order all-pass stages, a triangle LFO and bounded feedback.
/// All audio and modulation arithmetic is fixed-point; no delay allocation.
#[derive(Clone, Copy, Debug)]
pub struct Phaser {
    rate_hz: u32,
    controls: PhaserControls,
    phase: u32,
    step: u32,
    state: [i32; 4],
    last: i32,
    blend: i32,
}

impl Phaser {
    /// Construct a phaser, or return `None` for a zero sample rate.
    #[must_use]
    pub fn new(rate_hz: u32) -> Option<Self> {
        if rate_hz == 0 {
            return None;
        }
        let mut fx = Self {
            rate_hz,
            controls: PhaserControls::INITIAL,
            phase: 0,
            step: 0,
            state: [0; 4],
            last: 0,
            blend: 0,
        };
        fx.set_controls(PhaserControls::INITIAL);
        Some(fx)
    }

    /// Update parameters without resetting phase or filter history.
    #[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
    pub fn set_controls(&mut self, controls: PhaserControls) {
        self.controls = controls;
        self.step =
            ((u64::from(controls.centihz()) << 32) / (100 * u64::from(self.rate_hz))) as u32;
    }

    /// Process a single signed Q23 sample.
    #[allow(clippy::arithmetic_side_effects, clippy::cast_possible_wrap)]
    pub fn tick(&mut self, sample: I24) -> I24 {
        let c = self.controls;
        let mix = blend(
            &mut self.blend,
            if c.enabled {
                i32::from(c.mix.min(128))
            } else {
                0
            },
        );
        self.phase = self.phase.wrapping_add(self.step);
        if mix == 0 {
            self.state = [0; 4];
            self.last = 0;
            return sample;
        }
        let ramp = (self.phase >> 16) as i32;
        let triangle = if ramp < 32768 {
            2 * ramp - 32768
        } else {
            98302 - 2 * ramp
        };
        // g = 0.55 +/- 0.35; never reaches either unstable endpoint.
        let depth = (11469 * i32::from(c.depth.min(128))) >> 7;
        let g = 18022 + mul_q15(triangle, depth);
        let x = sample.inner();
        let feedback = (96 * i32::from(c.feedback.min(128))) >> 7;
        let mut wet = clamp_pcm(x + mul_q7(self.last, feedback));
        // Allow internal headroom; clamp only at an explicit safety bound.
        const LIMIT: i32 = 1 << 27;
        for state in &mut self.state {
            let y = (*state - mul_q15(wet, g)).clamp(-LIMIT, LIMIT);
            *state = (wet + mul_q15(y, g)).clamp(-LIMIT, LIMIT);
            wet = y;
        }
        self.last = clamp_pcm(wet);
        pcm(x + mul_q7(self.last - x, mix))
    }
}

// Also expose these as normal WaveGraph nodes with sample-offset control.
macro_rules! node {
    ($ty:ty, $control:ident) => {
        impl Process<Message, I24> for $ty {
            fn inlets(&self) -> &[Port] {
                &[Port::Signal]
            }
            fn outlets(&self) -> &[Port] {
                &[Port::Signal]
            }
            fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
                let mut start = 0;
                for event in cx.events {
                    for (out, input) in cx.outputs[0][start..event.frame]
                        .iter_mut()
                        .zip(&cx.inputs[0][start..event.frame])
                    {
                        *out = self.tick(*input);
                    }
                    if let Message::$control(controls) = event.message {
                        self.set_controls(controls);
                    }
                    start = event.frame;
                }
                for (out, input) in cx.outputs[0][start..cx.frames]
                    .iter_mut()
                    .zip(&cx.inputs[0][start..cx.frames])
                {
                    *out = self.tick(*input);
                }
            }
        }
    };
}
node!(Saturator, Saturator);
node!(Phaser, Phaser);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturator_is_nonlinear_and_bypass_is_exact() {
        let mut fx = Saturator::new();
        let input = I24::new_unchecked(1 << 20);
        for _ in 0..128 {
            fx.tick(input);
        }
        assert_ne!(fx.tick(input), input);
        assert_eq!(fx.tick(I24::default()), I24::default());
        fx.set_controls(SaturatorControls {
            enabled: false,
            ..SaturatorControls::INITIAL
        });
        for _ in 0..64 {
            fx.tick(input);
        }
        assert_eq!(fx.tick(input), input);
    }

    #[test]
    fn phaser_modulates_and_bypass_is_exact() {
        let mut fx = Phaser::new(44_118).expect("sample rate");
        let mut changed = false;
        for i in 0..8192 {
            let x = I24::new_unchecked(if i % 64 < 32 { 1 << 21 } else { -(1 << 21) });
            changed |= fx.tick(x) != x;
        }
        assert!(changed);
        assert_ne!(fx.phase, 0);
        fx.set_controls(PhaserControls {
            enabled: false,
            ..PhaserControls::INITIAL
        });
        let x = I24::new_unchecked(1 << 20);
        for _ in 0..64 {
            fx.tick(x);
        }
        assert_eq!(fx.tick(x), x);
        assert!(Phaser::new(0).is_none());
    }
}
