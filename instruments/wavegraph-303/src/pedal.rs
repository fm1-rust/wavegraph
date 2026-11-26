//! Compact post-VCA overdrive voiced for the 303's resonant output.
//! This is a calibrated musical model, not a transistor or TB-03 firmware model.

use crate::dsp::tau_to_coef;

// The native resonant voice has intentional headroom above +/-1. Normalize it
// to a fixed pedal reference, independently of Tb303's final output attenuation.
const INPUT_REFERENCE: f32 = 0.0625;
const OUTPUT_REFERENCE: f32 = 16.0 * 0.14;
const RAMP: u8 = 64;
const QUIET: f32 = 1e-10;

#[derive(Debug)]
pub(crate) struct Pedal {
    pre: f32,
    tone: f32,
    dc: f32,
    coefficients: [f32; 3],
    gain: f32,
    mix: f32,
    amount: f32,
    target_gain: f32,
    target_mix: f32,
    gain_step: f32,
    mix_step: f32,
    remaining: u8,
}

impl Pedal {
    pub(crate) fn new(rate: f32) -> Self {
        // A bass-tightened preamp, smooth clip and restrained tone retain the
        // earlier pedal's midrange drive with three states and one clip. The
        // output coupling also removes the DC produced by asymmetric waveforms.
        // Frequencies/gain were calibrated against dry 303 phrases, not inferred
        // from an undocumented TB-03 algorithm.
        let coefficients = [330.0_f32, 1800.0, 110.0].map(|hz| {
            tau_to_coef(
                1.0 / (2.0 * core::f32::consts::PI * hz.min(0.45 * rate)),
                rate,
            )
        });
        Self {
            pre: 0.0,
            tone: 0.0,
            dc: 0.0,
            coefficients,
            gain: 12.0,
            mix: 0.0,
            amount: 0.0,
            target_gain: 12.0,
            target_mix: 0.0,
            gain_step: 0.0,
            mix_step: 0.0,
            remaining: 0,
        }
    }

    #[inline]
    pub(crate) fn set_amount(&mut self, amount: f32) {
        if self.amount != amount {
            self.change_amount(amount);
        }
    }

    // Keep the settled voice in SRAM and update the ramp only on a changed control.
    #[inline(never)]
    fn change_amount(&mut self, amount: f32) {
        self.amount = amount;
        let gain = 12.0 + 660.0 * amount * amount;
        let mix = if amount == 0.0 { 0.0 } else { 1.0 };
        if gain != self.target_gain || mix != self.target_mix {
            self.target_gain = gain;
            self.target_mix = mix;
            self.begin_ramp();
        }
    }

    fn begin_ramp(&mut self) {
        self.gain_step = (self.target_gain - self.gain) * (1.0 / 64.0);
        self.mix_step = (self.target_mix - self.mix) * (1.0 / 64.0);
        self.remaining = RAMP;
    }

    #[inline]
    fn advance(&mut self) {
        if self.remaining != 0 {
            self.remaining = self.remaining.saturating_sub(1);
            if self.remaining == 0 {
                self.finish_ramp();
            } else {
                self.gain += self.gain_step;
                self.mix += self.mix_step;
            }
        }
    }

    fn finish_ramp(&mut self) {
        self.gain = self.target_gain;
        self.mix = self.target_mix;
        self.remaining = 0;
        if self.mix == 0.0 {
            self.clear_filters();
        }
    }

    fn clear_filters(&mut self) {
        self.pre = 0.0;
        self.tone = 0.0;
        self.dc = 0.0;
    }

    pub(crate) fn reset(&mut self) {
        self.clear_filters();
        self.gain = 12.0;
        self.mix = 0.0;
        if self.target_mix == 0.0 {
            self.finish_ramp();
        } else {
            self.begin_ramp();
        }
    }

    /// Discard only sub-PCM filter tails, then advance the control ramp cheaply.
    #[inline(never)]
    pub(crate) fn skip_silence(&mut self, frames: usize) -> bool {
        let level = self.pre.abs() + self.tone.abs() + self.dc.abs();
        if level > QUIET {
            return false;
        }
        self.clear_filters();
        if frames >= usize::from(self.remaining) {
            self.finish_ramp();
        } else {
            for _ in 0..frames {
                self.advance();
            }
        }
        true
    }

    // The compact state fits directly in the voice's output closure. There is
    // no second audio pass or per-sample out-of-line pedal call.
    #[inline(always)]
    pub(crate) fn sample(&mut self, dry: f32) -> f32 {
        if self.remaining != 0 {
            return self.transition(dry);
        }
        if self.mix == 0.0 { dry } else { self.wet(dry) }
    }

    // Keep the 64-sample control transition outside the settled voice loop.
    #[inline(never)]
    fn transition(&mut self, dry: f32) -> f32 {
        self.advance();
        if self.mix == 0.0 && self.remaining == 0 {
            return dry;
        }
        let wet = self.wet(dry);
        if self.mix == 1.0 {
            wet
        } else {
            dry + self.mix * (wet - dry)
        }
    }

    #[inline(always)]
    fn wet(&mut self, dry: f32) -> f32 {
        let [pre, tone, dc] = self.coefficients;
        let x = (dry * INPUT_REFERENCE).clamp(-8.0, 8.0);
        self.pre += (x - self.pre) * pre;
        let x = rail(self.gain * (x - self.pre));
        self.tone += (x - self.tone) * tone;
        self.dc += (self.tone - self.dc) * dc;
        (self.tone - self.dc) * OUTPUT_REFERENCE
    }
}

/// Smooth finite-rail saturation with unity small-signal gain and no bias.
/// A bounded polynomial avoids a tanh/division in each gain stage.
#[inline]
fn rail(x: f32) -> f32 {
    if x >= 1.5 {
        1.0
    } else if x <= -1.5 {
        -1.0
    } else {
        x * (1.0 - (4.0 / 27.0) * x * x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_no_bias_at_any_amount_and_bypass_is_exact() {
        let mut pedal = Pedal::new(44_118.0);
        for amount in [0.0, 0.01, 0.5, 1.0, 0.0] {
            pedal.set_amount(amount);
            for _ in 0..256 {
                assert_eq!(pedal.sample(0.0), 0.0);
            }
        }
        for sample in [-100.0, -0.0, 0.0, 0.125, 3.0, 100.0] {
            assert_eq!(pedal.sample(sample).to_bits(), sample.to_bits());
        }
    }

    #[test]
    #[allow(
        clippy::cast_precision_loss,
        reason = "The bounded test sweep fits f32 exactly"
    )]
    fn finite_rails_are_odd_monotone_and_bounded() {
        let mut previous = -1.0;
        for i in -300..=300 {
            let x = i as f32 / 100.0;
            let y = rail(x);
            assert!((-1.0..=1.0).contains(&y));
            assert!(y >= previous);
            assert_eq!(y, -rail(-x));
            previous = y;
        }
        assert_eq!(rail(-f32::MAX), -1.0);
        assert_eq!(rail(f32::MAX), 1.0);
    }

    #[test]
    fn constant_inputs_settle_without_dc_or_stuck_filter_tails() {
        for rate in [8000.0, 44_118.0, 96_000.0] {
            let mut pedal = Pedal::new(rate);
            pedal.set_amount(1.0);
            assert!(pedal.skip_silence(64));
            for input in [-128.0, -1.0, 0.25, 128.0] {
                for _ in 0..8192 {
                    let output = pedal.sample(input);
                    assert!(output.is_finite());
                    assert!(output.abs() <= 4.49);
                }
                assert!(pedal.sample(input).abs() < 1e-4);
            }
            for _ in 0..8192 {
                pedal.sample(0.0);
            }
            assert!(pedal.skip_silence(128));
            assert_eq!(pedal.sample(0.0), 0.0);
        }
    }

    #[test]
    fn initialized_and_reset_bypass_preserve_signed_zero_and_overrange() {
        let mut pedal = Pedal::new(44_118.0);
        pedal.set_amount(0.0);
        for reset in [false, true] {
            if reset {
                pedal.reset();
            }
            for input in [-f32::MAX, -0.0, 0.0, 100.0, f32::MAX] {
                assert_eq!(pedal.sample(input).to_bits(), input.to_bits());
            }
        }
    }
}
