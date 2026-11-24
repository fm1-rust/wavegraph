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

// Reuse the history prefix and working area across successive voices and blocks.
pub(super) struct Scratch {
    first: [f32; 7 + super::BLOCK_FRAMES],
    second: [f32; 15 + super::BLOCK_FRAMES],
}

impl Scratch {
    pub(super) const fn new() -> Self {
        Self {
            first: [0.0; 7 + super::BLOCK_FRAMES],
            second: [0.0; 15 + super::BLOCK_FRAMES],
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Decimator {
    first: [f32; 7],
    second: [f32; 15],
}

impl Decimator {
    pub(super) const fn new() -> Self {
        Self {
            first: [0.0; 7],
            second: [0.0; 15],
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The block contains at most BLOCK_FRAMES pairs and each tap offset is in 0..16."
    )]
    pub(super) fn process<'a>(
        &mut self,
        scratch: &'a mut Scratch,
        len: usize,
        mut next: impl FnMut() -> [f32; 2],
    ) -> &'a [f32] {
        let Scratch { first, second } = scratch;
        first[..7].copy_from_slice(&self.first);
        second[..15].copy_from_slice(&self.second);
        for (a, b) in first[7..].iter_mut().zip(&mut second[15..]).take(len) {
            [*a, *b] = next();
        }

        // Each window ends at the second sample of the current pair. The
        // center tap reads a first sample from seven pairs ago. Contiguous
        // history makes every folded tap a fixed offset from the window.
        for (out, window) in first[..len].iter_mut().zip(second.windows(16)) {
            let mut sum = 0.5 * *out;
            for (j, weight) in TAPS.into_iter().enumerate() {
                sum += weight * (window[15 - j] + window[j]);
            }
            *out = sum;
        }
        self.first.copy_from_slice(&first[len..len + 7]);
        self.second.copy_from_slice(&second[len..len + 15]);
        &first[..len]
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
            let mut scratch = Scratch::new();
            let mut phase = 0_usize;
            let mut next = || {
                let value = period[phase % period.len()];
                phase = phase.wrapping_add(stride);
                value
            };
            // Prime the history in uneven blocks, then measure steady state.
            for len in [3, 13, 1] {
                filter.process(&mut scratch, len, || [next(), next()]);
            }
            filter
                .process(&mut scratch, 16, || [next(), next()])
                .iter()
                .fold(0.0_f32, |maximum, v| maximum.max(v.abs()))
        };
        // At 88.2 kHz these are 11.025 kHz and 33.075 kHz respectively.
        assert!((0.99..1.01).contains(&peak(1)));
        assert!(peak(3) < 0.003);
    }
}
