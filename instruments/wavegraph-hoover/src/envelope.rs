//! Four timed linear segments with independent targets and a held third level.

#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    frames: [u32; 4],
    targets: [f32; 4],
}

impl Timing {
    pub(crate) fn new(times: [u8; 4], levels: [u8; 3], period: f32) -> Self {
        // Approximate panel time law. Zero is immediate, 127 is about 8 s.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frames = times.map(|t| {
            let x = f32::from(t.min(127)) / 127.0;
            (8.0 * x * x * x / period + 0.5) as u32
        });
        let [a, b, c] = levels.map(|l| f32::from(l.min(127)) / 127.0);
        Self {
            frames,
            targets: [a, b, c, 0.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    One,
    Two,
    Three,
    Hold,
    Release,
    Idle,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Envelope {
    stage: Stage,
    left: u32,
    value: f32,
    target: f32,
    delta: f32,
}

impl Envelope {
    pub(crate) const IDLE: Self = Self {
        stage: Stage::Idle,
        left: 0,
        value: 0.0,
        target: 0.0,
        delta: 0.0,
    };
    pub(crate) fn idle(self) -> bool {
        self.stage == Stage::Idle
    }
    pub(crate) fn held_level(&self) -> Option<f32> {
        (self.stage == Stage::Hold).then_some(self.value)
    }
    pub(crate) fn releasing(self) -> bool {
        self.stage == Stage::Release
    }
    pub(crate) fn strike(&mut self, t: &Timing) {
        *self = Self::IDLE;
        self.enter(Stage::One, t);
    }
    pub(crate) fn release(&mut self, t: &Timing) {
        if !self.idle() && !self.releasing() {
            self.enter(Stage::Release, t);
        }
    }
    fn next(stage: Stage) -> Stage {
        match stage {
            Stage::One => Stage::Two,
            Stage::Two => Stage::Three,
            Stage::Three => Stage::Hold,
            Stage::Release => Stage::Idle,
            other => other,
        }
    }
    #[allow(clippy::cast_precision_loss)]
    fn enter(&mut self, mut stage: Stage, t: &Timing) {
        loop {
            self.stage = stage;
            let i = match stage {
                Stage::One => 0,
                Stage::Two => 1,
                Stage::Three => 2,
                Stage::Release => 3,
                _ => return,
            };
            self.left = t.frames[i];
            self.target = t.targets[i];
            if self.left > 0 {
                self.delta = (self.target - self.value) / self.left as f32;
                return;
            }
            self.value = self.target;
            stage = Self::next(stage);
        }
    }
    pub(crate) fn sample(&mut self, t: &Timing) -> f32 {
        if self.left > 0 {
            self.left = self.left.saturating_sub(1);
            self.value += self.delta;
            if self.left == 0 {
                self.value = self.target;
                self.enter(Self::next(self.stage), t);
            }
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valley_then_second_peak_holds_until_key_up() {
        let t = Timing {
            frames: [2, 2, 2, 2],
            targets: [1.0, 0.25, 0.75, 0.0],
        };
        let mut env = Envelope::IDLE;
        env.strike(&t);
        for expected in [0.5, 1.0, 0.625, 0.25, 0.5, 0.75, 0.75] {
            assert_eq!(env.sample(&t), expected);
        }
        env.release(&t);
        assert_eq!(env.sample(&t), 0.375);
        assert_eq!(env.sample(&t), 0.0);
        assert!(env.idle());
    }

    #[test]
    fn early_key_up_and_zero_length_segments() {
        let t = Timing {
            frames: [0, 0, 4, 2],
            targets: [1.0, 0.0, 1.0, 0.0],
        };
        let mut env = Envelope::IDLE;
        env.strike(&t);
        assert_eq!(env.sample(&t), 0.25);
        env.release(&t);
        assert_eq!(env.sample(&t), 0.125);
        env.release(&t);
        assert_eq!(env.sample(&t), 0.0);
        assert!(env.idle());
    }
}
