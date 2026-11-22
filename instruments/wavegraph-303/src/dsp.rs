pub(crate) fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

pub(crate) fn fast_exp2(mut x: f32) -> f32 {
    x = clamp(x, -30.0, 30.0);
    // This bounded floor is exact on both the host and the FM-1.
    let fl = micromath::F32Ext::floor(x);
    let f = x - fl;
    let p = 1.0 + f * (core::f32::consts::LN_2 + f * (0.2402265 + f * (0.0555041 + f * 0.0096181)));
    // Every power of two from 2^-30 through 2^30 is a normal f32.
    // Scaling the polynomial this way gives the same result as ldexpf.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The biased exponent is an integer in 97..=157 for finite inputs."
    )]
    let exponent = (fl + 127.0) as u32;
    p * f32::from_bits(exponent << 23)
}

// Both filters clamp to a positive frequency below 0.45 times their internal
// rate. Reduce to [0, π/4] and evaluate the [7/6] Padé approximant for tan.
// A split π/2 preserves the residual in the reciprocal branch.
pub(crate) fn filter_tan(x: f32) -> f32 {
    const HALF_PI_HI: f32 = f32::from_bits(0x3fc9_0fda); // π/2 rounded down
    const HALF_PI_LO: f32 = 7.549_789_4e-8;
    let reciprocal = x > core::f32::consts::FRAC_PI_4;
    let y = if reciprocal {
        (HALF_PI_HI - x) + HALF_PI_LO
    } else {
        x
    };
    let z = y * y;
    let p = 135_135.0 + z * (-17_325.0 + z * (378.0 - z));
    let q = 135_135.0 + z * (-62_370.0 + z * (3_150.0 - 28.0 * z));
    let t = y * p / q;
    if reciprocal { 1.0 / t } else { t }
}

pub(crate) fn fast_tanh(x: f32) -> f32 {
    if x > 3.0 {
        return 1.0;
    }
    if x < -3.0 {
        return -1.0;
    }
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

pub(crate) fn midi_to_hz(note: f32) -> f32 {
    440.0 * fast_exp2((note - 69.0) * (1.0 / 12.0))
}

pub(crate) fn tau_to_coef(seconds: f32, sr: f32) -> f32 {
    if seconds <= 0.0 {
        return 1.0;
    }
    1.0 - decay_exp(-1.0 / (seconds * sr))
}

// The upstream envelope subtracts this exponential from one in f32. Keep
// that rounding step: it determines the sample at which a release ends.
pub(crate) fn decay_exp(x: f32) -> f32 {
    if x >= -0.0625 {
        return micromath::F32Ext::exp(x);
    }
    // Short filter time constants need a more accurate result than one
    // micromath exponential over the full argument. exp(x) = exp(x/8)^8.
    let eighth = micromath::F32Ext::exp(x * 0.125);
    let quarter = eighth * eighth;
    let half = quarter * quarter;
    half * half
}

// CHOMPI's parabola approximation, with phase measured in turns.
pub(crate) fn fast_sin(mut phase: f32) -> f32 {
    phase -= micromath::F32Ext::floor(phase);
    let t = if phase < 0.5 {
        2.0 * phase
    } else {
        2.0 * phase - 2.0
    };
    let y = 4.0 * t * (1.0 - t.abs());
    0.225 * (y * y.abs() - y) + y
}

pub(crate) fn fast_log2(x: f32) -> f32 {
    let bits = x.to_bits();
    let exponent = f32::from(((bits >> 23) & 255) as i16) - 127.0;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    exponent + (-0.344_848_42 * m + 2.024_665_8) * m - 1.674_877_6
}

#[derive(Debug, Default)]
pub(crate) struct BitCrusher {
    pub(crate) held: f32,
    remaining: i32,
}

pub(crate) struct CrushSettings {
    levels: Option<f32>,
    hold: i32,
}

impl BitCrusher {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "The sample-and-hold period is measured in whole frames."
    )]
    pub(crate) fn settings(bits: f32, rate: f32) -> CrushSettings {
        CrushSettings {
            levels: (bits > 0.005).then(|| fast_exp2(15.0 - 12.0 * bits)),
            hold: (31.0 * rate * rate) as i32,
        }
    }

    pub(crate) fn sample(&mut self, sample: f32, settings: &CrushSettings) -> f32 {
        if settings.levels.is_none() && settings.hold == 0 {
            return sample;
        }
        if self.remaining == 0 {
            self.remaining = settings.hold;
            self.held = if let Some(levels) = settings.levels {
                micromath::F32Ext::floor(sample.clamp(-1.0, 1.0) * levels + 0.5) / levels
            } else {
                sample
            };
        } else {
            self.remaining = self.remaining.saturating_sub(1);
        }
        self.held
    }
}

#[cfg(test)]
mod tests {
    use super::{filter_tan, tau_to_coef};
    use micromath::F32Ext;

    #[test]
    fn envelope_time_constants() {
        for rate in [44_100.0, 24_000_000.0 / 544.0, 48_000.0, 96_000.0] {
            for seconds in [0.0015, 0.002, 0.004, 0.012, 0.2 / 4.6, 1.34] {
                let actual = tau_to_coef(seconds, rate);
                let reference = 1.0 - (-1.0 / (seconds * rate)).exp();
                assert_eq!(
                    actual.to_bits(),
                    reference.to_bits(),
                    "{seconds}s at {rate} Hz"
                );
            }
        }
    }

    #[test]
    fn cutoff_in_octaves() {
        for i in 1..=32_000u16 {
            let x = f32::from(i);
            assert!((F32Ext::log2(x) - x.log2()).abs() < 2e-6);
        }
        assert_eq!(F32Ext::log2(f32::from_bits(1)), -149.0);
        assert_eq!(F32Ext::log2(0.0), f32::NEG_INFINITY);
    }

    #[test]
    fn ladder_prewarp_error() {
        // The ladder clamps each pole to at most 0.45 times its internal
        // sample rate. Include a near-zero input from the 96 kHz sweep.
        let upper = 0.45 * core::f32::consts::PI;
        let sweep = (1..=8192u16).map(|i| upper * f32::from(i) / 8192.0);
        for x in core::iter::once(f32::from_bits(0x392c_2fcb)).chain(sweep) {
            let exact = x.tan();
            let approx = filter_tan(x);
            let reference = exact / (1.0 + exact);
            let error = (approx / (1.0 + approx) - reference).abs();
            assert!(error <= 2e-7 && error / reference <= 4e-7, "x = {x}");
        }
    }
}
