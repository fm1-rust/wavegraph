//! A 31-tap half-band FIR. The middle tap is 1/2 and every other odd tap is zero.

// Parks–McClellan pass/stop edges at 0.20/0.30 of the internal sample rate,
// equal ripple weights. Scale the noncentral taps for unity DC gain.
const TAPS: [f32; 8] = [
    -0.002_109_92,
    0.004_659_05,
    -0.009_447_22,
    0.017_251_1,
    -0.029_858_35,
    0.051_660_03,
    -0.098_680_44,
    0.316_525_76,
];

#[derive(Clone, Debug)]
pub(super) struct Decimator {
    // Only the center tap reads the first sample of a pair, seven pairs later.
    first: [f32; 8],
    second: [f32; 16],
    next: usize,
}

impl Decimator {
    pub(super) const fn new() -> Self {
        Self {
            first: [0.0; 8],
            second: [0.0; 16],
            next: 0,
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The cursor is in 0..16 and the tap index is in 0..8."
    )]
    pub(super) fn process(&mut self, pair: [f32; 2]) -> f32 {
        let i = self.next;
        self.first[i & 7] = pair[0];
        self.second[i] = pair[1];
        self.next = (i + 1) & 15;
        // Evaluate the FIR at the second sample of the pair. The center tap
        // reads the first sample from seven pairs ago. Fold the eight symmetric
        // pairs of remaining taps, which all read second samples.
        let mut out = 0.5 * self.first[(i + 1) & 7];
        for (j, weight) in TAPS.into_iter().enumerate() {
            out += weight * (self.second[(i + 16 - j) & 15] + self.second[(i + 1 + j) & 15]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::FRAC_1_SQRT_2 as K;

    #[test]
    fn rejects_the_image_that_would_alias_onto_the_passband_tone() {
        let period = [0.0, K, 1.0, K, 0.0, -K, -1.0, -K];
        let peak = |stride: usize| {
            let mut filter = Decimator::new();
            let mut maximum = 0.0_f32;
            for i in 0..32 {
                let sample = |n: usize| period[n.wrapping_mul(stride) % 8];
                let value = filter.process([sample(2 * i), sample(2 * i + 1)]);
                if i >= 16 {
                    maximum = maximum.max(value.abs());
                }
            }
            maximum
        };
        // At 88.2 kHz these are 11.025 kHz and 33.075 kHz respectively.
        assert!((0.99..1.01).contains(&peak(1)));
        assert!(peak(3) < 0.003);
    }
}
