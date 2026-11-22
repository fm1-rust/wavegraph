use core::f32::consts::PI;

use crate::dsp::{fast_exp2, fast_tanh, tau_to_coef};

/// The CHOMPI post-VCA bass drive. The input and tone filters retain their
/// history while bypassed. This stage needs no stereo or delay storage.
#[derive(Clone, Copy, Debug)]
pub struct Drive {
    rate: f32,
    high_pass: f32,
    pre_emphasis: f32,
    hp: f32,
    pre: f32,
    dc: f32,
    tone1: f32,
    tone2: f32,
}

pub(crate) struct DriveCoefficients {
    gain: f32,
    bias: f32,
    makeup: f32,
    tone: f32,
}

impl Drive {
    /// Initialize the drive for a sample rate in Hz.
    #[must_use]
    pub fn new(rate: f32) -> Self {
        Self {
            rate,
            high_pass: tau_to_coef(1.0 / (2.0 * PI * 120.0), rate),
            pre_emphasis: tau_to_coef(1.0 / (2.0 * PI * 1000.0), rate),
            hp: 0.0,
            pre: 0.0,
            dc: 0.0,
            tone1: 0.0,
            tone2: 0.0,
        }
    }

    /// Process a mono block in place. Amount is in 0..=1. Values at or below
    /// 0.005 bypass the drive and leave its filter history in place.
    pub fn process(&mut self, amount: f32, samples: &mut [f32]) {
        if let Some(settings) = self.coefficients(amount) {
            for x in samples {
                *x = self.sample(*x, &settings);
            }
        }
        let level = self.state_level(0.0);
        if level >= 1e6 || level.is_nan() {
            self.clear();
        }
    }

    pub(crate) fn coefficients(&self, amount: f32) -> Option<DriveCoefficients> {
        (amount > 0.005).then(|| DriveCoefficients {
            gain: 1.0 + 60.0 * amount * amount,
            bias: 0.08 * amount,
            makeup: 0.135 + 0.865 * fast_exp2(-14.43 * amount),
            tone: tau_to_coef(1.0 / (2.0 * PI * (8000.0 - 2000.0 * amount)), self.rate),
        })
    }

    pub(crate) fn sample(&mut self, x: f32, c: &DriveCoefficients) -> f32 {
        self.hp += (x - self.hp) * self.high_pass;
        let tight = x - self.hp;
        self.pre += (tight - self.pre) * self.pre_emphasis;
        let bright = tight + (tight - self.pre);
        let clipped = hard_clip(bright * c.gain + c.bias);
        self.dc += (clipped - self.dc) * 0.0005;
        self.tone1 += (clipped - self.dc - self.tone1) * c.tone;
        self.tone2 += (self.tone1 - self.tone2) * c.tone;
        self.tone2 * c.makeup
    }

    pub(crate) fn clear(&mut self) {
        self.hp = 0.0;
        self.pre = 0.0;
        self.dc = 0.0;
        self.tone1 = 0.0;
        self.tone2 = 0.0;
    }

    pub(crate) fn state_level(&self, initial: f32) -> f32 {
        initial
            + self.hp.abs()
            + self.pre.abs()
            + self.tone1.abs()
            + self.tone2.abs()
            + self.dc.abs()
    }
}

pub(crate) fn hard_clip(x: f32) -> f32 {
    let abs = x.abs();
    if abs <= 0.9 {
        return x;
    }
    let knee = 0.9 + 0.1 * fast_tanh((abs - 0.9) * 10.0);
    if x < 0.0 { -knee } else { knee }
}
