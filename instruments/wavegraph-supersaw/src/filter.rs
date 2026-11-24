//! Two-pole low-pass at the output sample rate, after the half-band decimator.

use core::f32::consts::{PI, SQRT_2};
use micromath::F32;

#[derive(Clone, Debug)]
pub(super) struct FilterParams {
    // Prewarp the cutoff at quarter-octave intervals across the two-octave sweep.
    warp: [f32; 9],
    slew: f32,
    damping: f32,
}

impl FilterParams {
    pub(super) fn new(period: f32) -> Self {
        let mut filter = Self {
            warp: [0.0; 9],
            slew: (2_000.0 * period).min(1.0),
            damping: SQRT_2,
        };
        filter.set_cutoff(super::DEFAULT_CUTOFF_HZ, period);
        filter
    }

    pub(super) fn base_warp(&self) -> f32 {
        self.warp[0]
    }

    pub(super) fn set_cutoff(&mut self, hz: u16, period: f32) {
        let mut cycles = f32::from(hz.max(1)) * period;
        for g in &mut self.warp {
            *g = F32(PI * cycles.min(0.475)).tan().0;
            cycles *= 1.189_207_1; // 2^(1/4)
        }
    }

    pub(super) fn set_resonance(&mut self, resonance: u8) {
        // Q rises from Butterworth to 5 at the top of the dial.
        self.damping = SQRT_2 + (0.2 - SQRT_2) * (f32::from(resonance.min(127)) / 127.0);
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The envelope is in 0..=1 and selects one of eight intervals."
    )]
    pub(super) fn coefficients(&self, g: &mut f32, envelope: f32) -> Coefficients {
        let position = envelope * 8.0;
        let i = (position as u8).min(7);
        let fraction = position - f32::from(i);
        let i = usize::from(i);
        let (lo, hi) = (self.warp[i], self.warp[i + 1]);
        // Smooth panel steps over about half a millisecond.
        *g += self.slew * (lo + fraction * (hi - lo) - *g);
        Coefficients {
            g: *g,
            denominator: 1.0 + *g * (*g + self.damping),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Coefficients {
    g: f32,
    denominator: f32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct LowPass {
    band: f32,
    low: f32,
}

impl LowPass {
    pub(super) fn process(&mut self, input: f32, coeff: Coefficients) -> f32 {
        // Solve the feedback loop of two trapezoidal integrators for this sample.
        let g = coeff.g;
        let band = (self.band + g * (input - self.low)) / coeff.denominator;
        let low = self.low + g * band;
        self.band = 2.0 * band - self.band;
        self.low = 2.0 * low - self.low;
        low
    }
}
