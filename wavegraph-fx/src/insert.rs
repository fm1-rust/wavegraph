//! Small, allocation-free Q23 insert effects.

mod chorus;
pub use chorus::{Chorus, ChorusControls};
mod flanger;
mod pattern_filter;
mod slicer;
pub use flanger::Flanger;
pub use pattern_filter::{
    FILTER_PATTERNS, FilterMode, FilterPattern, PatternFilter, PatternFilterControls,
};
pub use slicer::{Slicer, SlicerControls};

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
    /// Clipping curve, from the original cubic soft knee to a hard knee.
    pub shape: u8,
    /// Enable with a short bypass crossfade.
    pub enabled: bool,
}

impl SaturatorControls {
    /// Factory controls.
    pub const INITIAL: Self = Self {
        drive: 32,
        mix: 128,
        level: 96,
        shape: 0,
        enabled: true,
    };
}

/// Controls shared by the phaser and flanger, all in 0..=128 units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModulationControls {
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

impl ModulationControls {
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

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "Sample rate is nonzero and phase increments wrap in one turn"
    )]
    fn phase_step(self, rate_hz: u32) -> u32 {
        ((u64::from(self.centihz()) << 32) / (100 * u64::from(rate_hz))) as u32
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

/// Symmetric soft-to-hard clipper with smoothed shape and wet/dry controls.
/// No oversampling: high drive can produce aliasing.
#[derive(Clone, Copy, Debug)]
pub struct Saturator {
    controls: SaturatorControls,
    blend: i32,
    shape: i32,
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
            shape: 0,
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
        let shape = blend(&mut self.shape, i32::from(c.shape.min(128)));
        if mix == 0 {
            return sample;
        }
        let x = sample.inner();
        let driven = clamp_pcm(mul_q7(x, 128 + 7 * i32::from(c.drive.min(128))));
        let square = ((i64::from(driven) * i64::from(driven)) >> 23) as i32;
        let cube = ((i64::from(square) * i64::from(driven)) >> 23) as i32;
        // Both curves have a slope of 1.5 at zero and reach the same peak.
        let soft = (3 * driven - cube) / 2;
        let hard = clamp_pcm(3 * driven / 2);
        let shaped = soft + mul_q7(hard - soft, shape);
        let wet = mul_q7(shaped, i32::from(c.level.min(128)));
        pcm(x + mul_q7(wet - x, mix))
    }

    /// Process a span in place, with the same samples and control-ramp timing
    /// as repeated calls to [`Self::tick`]. Steady spans select one clipping
    /// curve and prepare their gains once, outside the sample loop.
    #[inline(never)]
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "Byte controls bound Q7 gains; the private 0..=128 ramp states need at most 64 samples."
    )]
    pub fn process_in_place(&mut self, samples: &mut [I24]) {
        let c = self.controls;
        let mix = if c.enabled {
            i32::from(c.mix.min(128))
        } else {
            0
        };
        let shape = i32::from(c.shape.min(128));
        let ramp = self
            .blend
            .abs_diff(mix)
            .max(self.shape.abs_diff(shape))
            .div_ceil(2);
        let ramp = (ramp as usize).min(samples.len());
        let (changing, steady) = samples.split_at_mut(ramp);
        for sample in changing {
            *sample = self.tick(*sample);
        }
        if steady.is_empty() || mix == 0 {
            return;
        }
        let drive = 128 + 7 * i32::from(c.drive.min(128));
        let level = i32::from(c.level.min(128));
        // The constant parameters remove unused clipping and blend arithmetic
        // from each kernel, including its inner loop's branches.
        match (shape, mix) {
            (0, 128) => saturator_span::<0, true>(steady, drive, level, shape, mix),
            (128, 128) => saturator_span::<1, true>(steady, drive, level, shape, mix),
            (_, 128) => saturator_span::<2, true>(steady, drive, level, shape, mix),
            (0, _) => saturator_span::<0, false>(steady, drive, level, shape, mix),
            (128, _) => saturator_span::<1, false>(steady, drive, level, shape, mix),
            _ => saturator_span::<2, false>(steady, drive, level, shape, mix),
        }
    }
}

// CURVE selects soft (0), hard (1), or their existing interpolation (2).
// Keep the small steady kernels separate from the insert rack's dispatch code.
#[inline(never)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "The same bounded Q23 products and signed rounding as Saturator::tick."
)]
fn saturator_span<const CURVE: u8, const FULL_WET: bool>(
    samples: &mut [I24],
    drive: i32,
    level: i32,
    shape: i32,
    mix: i32,
) {
    for sample in samples {
        let x = sample.inner();
        let driven = clamp_pcm(mul_q7(x, drive));
        let shaped = if CURVE == 1 {
            clamp_pcm(3 * driven / 2)
        } else {
            let square = ((i64::from(driven) * i64::from(driven)) >> 23) as i32;
            let cube = ((i64::from(square) * i64::from(driven)) >> 23) as i32;
            let soft = (3 * driven - cube) / 2;
            if CURVE == 0 {
                soft
            } else {
                let hard = clamp_pcm(3 * driven / 2);
                soft + mul_q7(hard - soft, shape)
            }
        };
        let wet = mul_q7(shaped, level);
        *sample = if FULL_WET {
            pcm(wet)
        } else {
            pcm(x + mul_q7(wet - x, mix))
        };
    }
}

/// Four first-order all-pass stages, a triangle LFO and bounded feedback.
/// All audio and modulation arithmetic is fixed-point; no delay allocation.
#[derive(Clone, Copy, Debug)]
pub struct Phaser {
    rate_hz: u32,
    controls: ModulationControls,
    phase: wavegraph_lfo::Phase,
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
            controls: ModulationControls::INITIAL,
            phase: wavegraph_lfo::Phase::default(),
            state: [0; 4],
            last: 0,
            blend: 0,
        };
        fx.set_controls(ModulationControls::INITIAL);
        Some(fx)
    }

    /// Update parameters without resetting phase or filter history.
    pub fn set_controls(&mut self, controls: ModulationControls) {
        self.controls = controls;
        self.phase.control(wavegraph_lfo::PhaseControl {
            increment: controls.phase_step(self.rate_hz),
            phase: None,
        });
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
        let depth = (11469 * i32::from(c.depth.min(128))) >> 7;
        let feedback = (96 * i32::from(c.feedback.min(128))) >> 7;
        self.sample(sample, mix, depth, feedback)
    }

    /// Apply the current controls to a span, preserving sample-by-sample mix ramps.
    #[allow(clippy::arithmetic_side_effects)]
    // SAFETY: FM-1 startup copies this hot audio path into executable SRAM.
    #[cfg_attr(
        all(target_os = "none", feature = "fm1-ram-code"),
        allow(unsafe_code),
        unsafe(link_section = ".data.wavegraph_audio"),
        inline(never)
    )]
    pub fn process_in_place(&mut self, samples: &mut [I24]) {
        let c = self.controls;
        let target = if c.enabled {
            i32::from(c.mix.min(128))
        } else {
            0
        };
        let ramp = self.blend.abs_diff(target).div_ceil(2) as usize;
        let (changing, steady) = samples.split_at_mut(ramp.min(samples.len()));
        for sample in changing {
            *sample = self.tick(*sample);
        }
        if steady.is_empty() {
            return;
        }
        if target == 0 {
            self.phase.advance(steady.len());
            self.state = [0; 4];
            self.last = 0;
            return;
        }
        let depth = (11469 * i32::from(c.depth.min(128))) >> 7;
        let feedback = (96 * i32::from(c.feedback.min(128))) >> 7;
        for sample in steady {
            *sample = self.sample(*sample, target, depth, feedback);
        }
    }

    #[inline]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_possible_wrap)]
    fn sample(&mut self, sample: I24, mix: i32, depth: i32, feedback: i32) -> I24 {
        self.phase.advance(1);
        if mix == 0 {
            self.state = [0; 4];
            self.last = 0;
            return sample;
        }
        // g = 0.55 +/- 0.35; never reaches either unstable endpoint.
        let g = 18022 + mul_q15(self.phase.triangle_q15(), depth);
        let x = sample.inner();
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
    ($ty:ty, $($message:ident => $handler:ident),+) => {
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
                    match event.message {
                        $(Message::$message(value) => self.$handler(value),)+
                        _ => (),
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
node!(Saturator, Saturator => set_controls);
node!(Phaser, Phaser => set_controls);
node!(Chorus, Chorus => set_controls, Enabled => set_enabled);
node!(Flanger, Flanger => set_controls, Enabled => set_enabled, BeatTime => set_clock, BeatPhase => anchor);
node!(Slicer, Slicer => set_controls, StepPattern => set_pattern, Enabled => set_enabled, BeatTime => set_clock, BeatPhase => anchor);
node!(super::ThreeBandEq, Eq => set_controls);
node!(PatternFilter, PatternFilter => set_controls, FilterTrigger => trigger, Enabled => set_enabled);

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
    fn saturator_shape_morphs_between_soft_and_hard_at_the_same_level() {
        let mut fx = Saturator::new();
        let controls = SaturatorControls {
            drive: 0,
            mix: 128,
            level: 128,
            ..SaturatorControls::INITIAL
        };
        let sample = I24::new_unchecked(1 << 22);
        for (shape, expected) in [(0, 5_767_168), (64, 6_029_312), (128, 6_291_456)] {
            fx.set_controls(SaturatorControls { shape, ..controls });
            for _ in 0..64 {
                fx.tick(sample);
            }
            assert_eq!(fx.tick(sample).inner(), expected);
            assert_eq!(fx.tick(I24::new_unchecked(-(1 << 22))).inner(), -expected);
            assert_eq!(fx.tick(I24::default()).inner(), 0);
        }
        fx.set_controls(controls);
        assert_eq!(fx.tick(sample).inner(), 6_283_264); // first of 64 steps back
        for _ in 1..64 {
            fx.tick(sample);
        }
        assert_eq!(fx.tick(sample).inner(), 5_767_168);
    }

    #[test]
    fn phaser_spans_match_sample_edits_through_dry_bypass() {
        let mut block = Phaser::new(44_118).expect("valid audio clock");
        let mut reference = block;
        let signal: [I24; 128] =
            core::array::from_fn(|i| pcm(if i % 3 == 0 { -8_388_608 } else { 6_000_000 }));
        for (mix, enabled, frames) in [
            (100, true, 13),
            (100, true, 128),
            (7, false, 0),
            (7, false, 17),
            (7, false, 128),
            (65, true, 128),
        ] {
            let c = ModulationControls {
                mix,
                enabled,
                depth: 111,
                feedback: 103,
                ..ModulationControls::INITIAL
            };
            block.set_controls(c);
            reference.set_controls(c);
            let mut actual = signal;
            block.process_in_place(&mut actual[..frames]);
            for (s, input) in actual[..frames].iter().zip(signal) {
                assert_eq!(*s, reference.tick(input));
            }
            assert_eq!(block.phase.position(), reference.phase.position());
        }
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
        assert_ne!(fx.phase.position(), 0);
        fx.set_controls(ModulationControls {
            enabled: false,
            ..ModulationControls::INITIAL
        });
        let x = I24::new_unchecked(1 << 20);
        for _ in 0..64 {
            fx.tick(x);
        }
        assert_eq!(fx.tick(x), x);
        assert!(Phaser::new(0).is_none());
    }
}
