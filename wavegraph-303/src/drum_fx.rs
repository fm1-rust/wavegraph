use crate::{
    drive::hard_clip,
    dsp::{BitCrusher, clamp, fast_exp2, tau_to_coef},
};
use core::f32::consts::PI;

/// Controls for the drum effects. Knob values range from zero to one.
#[derive(Clone, Copy, Debug)]
pub struct DrumFxSettings {
    /// Distortion amount.
    pub drive: f32,
    /// Low-pass below the center dead band, high-pass above it.
    pub filter: f32,
    /// Filter resonance.
    pub filter_res: f32,
    /// Depth of the per-hit filter envelope.
    pub env_amount: f32,
    /// Filter-envelope decay.
    pub env_decay: f32,
    /// Bit-depth reduction.
    pub crush_bits: f32,
    /// Sample-rate reduction.
    pub crush_rate: f32,
}

impl Default for DrumFxSettings {
    fn default() -> Self {
        Self {
            drive: 0.0,
            filter: 0.5,
            filter_res: 0.0,
            env_amount: 0.0,
            env_decay: 0.4,
            crush_bits: 0.0,
            crush_rate: 0.0,
        }
    }
}

/// CHOMPI's drum-bus distortion, resonant filter and bit crusher.
#[derive(Debug)]
pub struct DrumFx {
    rate: f32,
    f1: f32,
    f2: f32,
    pre: f32,
    tone1: f32,
    tone2: f32,
    crusher: BitCrusher,
    envelope: f32,
    trigger: Option<(usize, f32)>,
}

impl DrumFx {
    /// Initialize a silent effects processor for this sample rate in Hz.
    #[must_use]
    pub fn new(rate: f32) -> Self {
        Self {
            rate,
            f1: 0.0,
            f2: 0.0,
            pre: 0.0,
            tone1: 0.0,
            tone2: 0.0,
            crusher: BitCrusher::default(),
            envelope: 0.0,
            trigger: None,
        }
    }

    /// Trigger the envelope at a frame offset in the next nonempty block.
    /// An offset beyond that block fires on its last frame.
    pub fn trigger(&mut self, offset: usize, depth: f32) {
        self.trigger = Some((offset, depth));
    }

    /// Apply the settings and process a mono block in place.
    pub fn process(&mut self, settings: &DrumFxSettings, block: &mut [f32]) {
        let drive = settings.drive;
        let gain = 1.0 + 60.0 * drive * drive;
        let makeup = 0.2 + 0.8 * fast_exp2(-7.21 * drive);
        let pre_lp = tau_to_coef(1.0 / (2.0 * PI * 1000.0), self.rate);
        let tone_lp = tau_to_coef(1.0 / (2.0 * PI * (8000.0 - 2000.0 * drive)), self.rate);
        let f = settings.filter - 0.5;
        let low = f < -0.02;
        let high = f > 0.02;
        let hz = if low {
            200.0 * fast_exp2(6.64 * (1.0 + 2.0 * f))
        } else {
            20.0 * fast_exp2(7.97 * 2.0 * f)
        };
        let resonance = settings.filter_res.clamp(0.0, 1.0);
        let kq = 2.0 * fast_exp2(-4.2 * resonance);
        let coefficients = |hz| {
            let g = crate::dsp::filter_tan(PI * clamp(hz, 20.0, 0.45 * self.rate) / self.rate);
            let a1 = 1.0 / (1.0 + g * (g + kq));
            let a2 = g * a1;
            (a1, a2, g * a2)
        };
        let mut coeff = coefficients(hz);
        let trim = 1.0 / (1.0 + 1.5 * resonance);
        let octaves = 6.0 * settings.env_amount.clamp(0.0, 1.0) * if low { 1.0 } else { -1.0 };
        let decay = tau_to_coef(
            0.01 * fast_exp2(6.64 * settings.env_decay.clamp(0.0, 1.0)),
            self.rate,
        );
        let env_on = (low || high) && settings.env_amount > 0.005;
        let crush = BitCrusher::settings(settings.crush_bits, settings.crush_rate);
        let last = block.len().saturating_sub(1);
        for (i, y) in block.iter_mut().enumerate() {
            if let Some((at, depth)) = self.trigger {
                if i == at || i == last {
                    self.envelope = depth;
                    self.trigger = None;
                }
            }
            self.envelope -= self.envelope * decay;
            // The drum filter's four-frame phase restarts at each block.
            if env_on && i & 3 == 0 {
                coeff = coefficients(hz * fast_exp2(octaves * self.envelope));
            }
            if drive > 0.005 {
                self.pre += (*y - self.pre) * pre_lp;
                let d = hard_clip((*y + (*y - self.pre)) * gain);
                self.tone1 += (d - self.tone1) * tone_lp;
                self.tone2 += (self.tone1 - self.tone2) * tone_lp;
                *y = self.tone2 * makeup;
            }
            if low || high {
                let (a1, a2, a3) = coeff;
                let v3 = *y - self.f2;
                let v1 = a1 * self.f1 + a2 * v3;
                let v2 = self.f2 + a2 * self.f1 + a3 * v3;
                self.f1 = 2.0 * v1 - self.f1;
                self.f2 = 2.0 * v2 - self.f2;
                *y = (if low { v2 } else { *y - kq * v1 - v2 }) * trim;
            }
            *y = self.crusher.sample(*y, &crush);
        }
    }
}
