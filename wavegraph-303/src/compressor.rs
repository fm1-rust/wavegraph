use crate::{
    StereoFrame,
    dsp::{fast_exp2, fast_log2, tau_to_coef},
};

/// Stereo-linked soft-knee master compressor.
#[derive(Clone, Copy, Debug)]
pub struct Compressor {
    amount: f32,
    threshold: f32,
    ratio: f32,
    makeup: f32,
    attack: f32,
    release: f32,
    reduction: f32,
}

impl Compressor {
    /// Initialize a bypassed compressor for this sample rate in Hz.
    #[must_use]
    pub fn new(rate: f32) -> Self {
        Self {
            amount: 0.0,
            threshold: 0.0,
            ratio: 1.0,
            makeup: 0.0,
            attack: tau_to_coef(0.008, rate),
            release: tau_to_coef(0.12, rate),
            reduction: 0.0,
        }
    }

    /// Set compression from bypass (zero) to the highest ratio (one).
    pub fn set_amount(&mut self, amount: f32) {
        self.amount = amount.clamp(0.0, 1.0);
        self.threshold = -6.0 - 24.0 * self.amount;
        self.ratio = 1.5 + 6.5 * self.amount;
        self.makeup = -self.threshold * (1.0 - 1.0 / self.ratio) * 0.45;
    }

    /// Current gain reduction in dB.
    #[must_use]
    pub fn reduction(&self) -> f32 {
        self.reduction
    }

    /// Compress a stereo block in place.
    pub fn process(&mut self, block: &mut [StereoFrame]) {
        if self.amount < 0.005 {
            self.reduction = 0.0;
            return;
        }
        let slope = 1.0 - 1.0 / self.ratio;
        for frame in block {
            let peak = frame.left.abs().max(frame.right.abs());
            let over = 6.0206 * fast_log2(peak + 1e-9) - self.threshold;
            let want = if over > 3.0 {
                over * slope
            } else if over > -3.0 {
                slope * (over + 3.0) * (over + 3.0) / 12.0
            } else {
                0.0
            };
            let k = if want > self.reduction {
                self.attack
            } else {
                self.release
            };
            self.reduction += (want - self.reduction) * k;
            let gain = fast_exp2((self.makeup - self.reduction) * 0.166_096_4);
            frame.left *= gain;
            frame.right *= gain;
        }
    }
}
