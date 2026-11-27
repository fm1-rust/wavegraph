//! Three complementary bands shared by the master bus and the channel inserts.

use crate::pcm;
use wavegraph_sample::I24;

/// Whole-dB gains in low, mid, high order. Each band supports −12 to +12 dB.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EqControls {
    /// Low, mid and high band gains.
    pub db: [i8; 3],
    /// When disabled, ramp all three bands back to unity.
    pub enabled: bool,
}

impl EqControls {
    /// Enabled with a flat response.
    pub const INITIAL: Self = Self {
        db: [0; 3],
        enabled: true,
    };
}

// 10^(dB/20), indexed by whole dB. Knob edits run on the audio thread.
pub(super) fn band_gain(db: i8) -> f32 {
    const GAINS: [f32; 25] = [
        0.25118864, 0.2818383, 0.31622776, 0.3548134, 0.39810717, 0.4466836, 0.5011872, 0.56234133,
        0.63095737, 0.70794576, 0.7943282, 0.8912509, 1.0, 1.1220185, 1.2589254, 1.4125376,
        1.5848932, 1.7782794, 1.9952623, 2.2387211, 2.5118864, 2.818383, 3.1622777, 3.5481339,
        3.9810717,
    ];
    GAINS[usize::from(db.clamp(-12, 12).saturating_add(12).cast_unsigned())]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bands {
    poles: [f32; 2],
    lowpass: [f32; 2],
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Bounded floating-point filter coefficients"
)]
impl Bands {
    pub(super) fn new(rate: f32) -> Self {
        let pole = |hz: f32| -libm::expm1f(-core::f32::consts::TAU * hz / rate);
        Self {
            poles: [pole(250.0), pole(3200.0)],
            lowpass: [0.0; 2],
        }
    }

    pub(super) fn tick(&mut self, x: f32, [lo, mid, hi]: [f32; 3]) -> f32 {
        let [low, upper] = &mut self.lowpass;
        *low += self.poles[0] * (x - *low);
        *upper += self.poles[1] * (x - *upper);
        // low + (upper - low) + (x - upper) reconstructs the dry input at unity.
        x * hi + *upper * (mid - hi) + *low * (lo - mid)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GainRamp<const N: usize> {
    pub(super) current: [f32; N],
    pub(super) target: [f32; N],
    step: [f32; N],
    remaining: u8,
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Bounded gain steps and a nonzero countdown"
)]
impl<const N: usize> GainRamp<N> {
    pub(super) const UNITY: Self = Self {
        current: [1.0; N],
        target: [1.0; N],
        step: [0.0; N],
        remaining: 0,
    };

    pub(super) fn set(&mut self, target: [f32; N]) {
        if target != self.target {
            self.target = target;
            self.step = core::array::from_fn(|i| (target[i] - self.current[i]) / 64.0);
            self.remaining = 64;
        }
    }

    pub(super) fn advance(&mut self) {
        if self.remaining != 0 {
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
            } else {
                for (gain, step) in self.current.iter_mut().zip(self.step) {
                    *gain += step;
                }
            }
        }
    }
}

/// Mono Q23 three-band EQ with 250 Hz and 3.2 kHz crossovers.
#[derive(Clone, Copy, Debug)]
pub struct ThreeBandEq {
    bands: Bands,
    gains: GainRamp<3>,
}

impl ThreeBandEq {
    /// Construct a flat EQ. Returns `None` for a zero sample rate.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "Audio clocks fit the filter precision"
    )]
    pub fn new(rate_hz: u32) -> Option<Self> {
        (rate_hz != 0).then(|| Self {
            bands: Bands::new(rate_hz as f32),
            gains: GainRamp::UNITY,
        })
    }

    /// Apply a gain or bypass edit over the next 64 samples.
    pub fn set_controls(&mut self, controls: EqControls) {
        self.gains.set(if controls.enabled {
            controls.db.map(band_gain)
        } else {
            [1.0; 3]
        });
    }

    /// Process one sample, retaining filter history through edits and bypass.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "Q23 input is exact in f32; boosted output is clipped to Q23"
    )]
    pub fn tick(&mut self, sample: I24) -> I24 {
        self.gains.advance();
        pcm(self.bands.tick(sample.inner() as f32, self.gains.current) as i32)
    }
}
