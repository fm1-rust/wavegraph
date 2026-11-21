//! Block operations used by the patch interpreter.
use wavegraph_sample::{I24, types::i24};

pub use wavegraph_core::Dsp;

/// Portable implementations for normalized `f32` and signed Q23 PCM.
#[derive(Clone, Copy, Debug, Default)]
pub struct Software;

impl Dsp<f32> for Software {
    fn gain(&mut self, input: &[f32], level: u8, output: &mut [f32]) {
        let scale = f32::from(level) / 128.0;
        for (out, &sample) in output.iter_mut().zip(input) {
            *out = (sample * scale).clamp(-1.0, 1.0);
        }
    }

    fn mix(&mut self, left: &[f32], right: &[f32], output: &mut [f32]) {
        for ((out, &a), &b) in output.iter_mut().zip(left).zip(right) {
            *out = (a + b).clamp(-1.0, 1.0);
        }
    }
}

#[inline]
fn q23(value: i32) -> I24 {
    I24::new_unchecked(value.clamp(i24::MIN.inner(), i24::MAX.inner()))
}

impl Dsp<I24> for Software {
    fn gain(&mut self, input: &[I24], level: u8, output: &mut [I24]) {
        for (out, &sample) in output.iter_mut().zip(input) {
            // The product of a signed 24-bit sample and a byte fits in i32.
            *out = q23(sample.inner().saturating_mul(i32::from(level)) >> 7);
        }
    }

    fn mix(&mut self, left: &[I24], right: &[I24], output: &mut [I24]) {
        for ((out, &a), &b) in output.iter_mut().zip(left).zip(right) {
            *out = q23(a.inner().saturating_add(b.inner()));
        }
    }
}
