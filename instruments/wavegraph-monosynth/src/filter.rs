use micromath::F32;

/// Four identical one-pole low-pass sections, with resonance at zero.
#[derive(Clone, Debug)]
pub(super) struct LowPass {
    stages: [f32; 4],
    gain: f32,
    target: f32,
}

impl LowPass {
    pub(super) const OPEN: Self = Self {
        stages: [0.0; 4],
        gain: 1.0,
        target: 1.0,
    };

    pub(super) fn skip_silent(&mut self, frames: usize) {
        let distance =
            f32::from(u8::try_from(frames.min(128)).expect("up to one DMA half")) / 128.0;
        self.gain += (self.target - self.gain).clamp(-distance, distance);
    }

    pub(super) fn set_cutoff(&mut self, hz: u16, sample_period: f32) {
        let cycles = f32::from(hz.max(1)) * sample_period;
        self.target = if cycles >= 0.5 {
            1.0
        } else {
            // At cutoff the four sections together should be down 3 dB.
            // For one pole, |H|² = a² / (a² + 4(1-a) sin²(π f / fs)).
            let sin = F32(0.5 * cycles).sin_turns().0;
            let power = sin * sin + 0.189_207_12; // 2^(1/4) - 1
            let root = F32(power).sqrt().0;
            // Refine micromath's square-root estimate with one Newton step.
            let root = 0.5 * (root + power / root);
            2.0 * sin / (sin + root)
        };
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "The 24-bit oscillator fits exactly in f32. Each stage interpolates within those bounds."
    )]
    pub(super) fn process(&mut self, oscillator: i32) -> i32 {
        // Slew coefficient changes over at most one FM-1 DMA half (128 samples).
        let delta = self.target - self.gain;
        self.gain += delta.clamp(-1.0 / 128.0, 1.0 / 128.0);
        let mut sample = oscillator as f32;
        for stage in &mut self.stages {
            *stage += self.gain * (sample - *stage);
            sample = *stage;
        }
        sample as i32
    }
}
