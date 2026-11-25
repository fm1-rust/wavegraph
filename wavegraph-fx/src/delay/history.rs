//! Longer echo taps. Both rings continuously record the feedback signal, so
//! changing time across a rate boundary can use the same live-tap crossfade.

/// Box-filter at write time, store logarithmic eight-bit PCM, and interpolate
/// adjacent group centers at read time. The native PCM16 tap covers short times.
#[derive(Debug)]
pub(super) struct History<'a, const STRIDE: usize> {
    line: &'a mut [u8],
    write: usize,
    phase: usize,
    sum: i32,
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Strides are 4 or 16 and tap distances are bounded by the allocated line"
)]
impl<'a, const STRIDE: usize> History<'a, STRIDE> {
    pub(super) fn new(line: &'a mut [u8]) -> Option<Self> {
        if line.len() < 2 || line.len().checked_mul(STRIDE).is_none() {
            return None;
        }
        line.fill(0);
        Some(Self {
            line,
            write: 0,
            phase: 0,
            sum: 0,
        })
    }

    pub(super) fn capacity(&self) -> usize {
        (self.line.len() - 1) * STRIDE
    }
    pub(super) fn push(&mut self, sample: i16) {
        self.sum += i32::from(sample);
        self.phase += 1;
        if self.phase == STRIDE {
            let mean = self.sum >> STRIDE.trailing_zeros();
            self.line[self.write] = encode(mean);
            self.write += 1;
            if self.write == self.line.len() {
                self.write = 0;
            }
            self.phase = 0;
            self.sum = 0;
        }
    }

    pub(super) fn tap(&self, frames: usize) -> i32 {
        let age = frames.saturating_sub(self.phase + STRIDE / 2);
        let back = 1 + age / STRIDE;
        let a = if self.write >= back {
            self.write - back
        } else {
            self.line.len() - (back - self.write)
        };
        let b = if a == 0 { self.line.len() - 1 } else { a - 1 };
        let near = decode(self.line[a]);
        let fraction = i32::try_from(age % STRIDE).expect("fraction of one group");
        near + (((decode(self.line[b]) - near) * fraction) >> STRIDE.trailing_zeros())
    }
}

// Sign, a three-bit exponent and a four-bit mantissa. Zero is acoustic silence.
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Biased PCM16 magnitude fits the eight logarithmic segments"
)]
fn encode(sample: i32) -> u8 {
    let mag = sample.unsigned_abs().min(32_635) + 132;
    let exponent = 24 - mag.leading_zeros();
    let mantissa = (mag >> (exponent + 3)) & 15;
    let code = (exponent << 4 | mantissa) as u8;
    // Quantize toward silence so a quiet repeat cannot sustain itself in the feedback loop.
    let code = if decode(code).unsigned_abs() > sample.unsigned_abs() {
        code.saturating_sub(1)
    } else {
        code
    };
    (u8::from(sample < 0) << 7) | code
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Eight logarithmic segments decode into signed PCM16"
)]
fn decode(code: u8) -> i32 {
    let mag = ((132 + 8 * i32::from(code & 15)) << ((code >> 4) & 7)) - 132;
    if code & 128 == 0 { mag } else { -mag }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn companding_never_adds_magnitude_to_feedback() {
        for sample in i16::MIN..=i16::MAX {
            let sample = i32::from(sample);
            let decoded = decode(encode(sample));
            assert!(decoded.unsigned_abs() <= sample.unsigned_abs());
            assert!(decoded == 0 || decoded.signum() == sample.signum());
        }
    }
}
