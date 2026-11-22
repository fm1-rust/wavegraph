//! Sample-clocked amplitude envelope for the filtered voice.

/// VCA attack, decay, sustain and release. Times are in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Adsr {
    /// Time from note-on to peak gain.
    pub attack_us: u32,
    /// Time from peak gain to the sustain level.
    pub decay_us: u32,
    /// Held level as a percentage of peak gain, clamped to 100.
    pub sustain_percent: u8,
    /// Time from note-off to silence, starting at the current gain.
    pub release_us: u32,
}

impl Adsr {
    /// Immediate attack and release at full sustain, the standalone voice's default.
    pub const GATE: Self = Self {
        attack_us: 0,
        decay_us: 0,
        sustain_percent: 100,
        release_us: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release, // Holds at zero after the ramp ends.
}

#[derive(Clone, Debug)]
pub(super) struct Envelope {
    stage: Stage,
    level: f32,
    from: f32,
    left: u32,
    inv_frames: f32,
    attack: u32,
    decay: u32,
    sustain: f32,
    release: u32,
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Times are nonnegative. Frame counts round to the nearest frame and saturate at u32::MAX."
)]
impl Envelope {
    pub(super) const GATE: Self = Self {
        stage: Stage::Release,
        level: 0.0,
        from: 0.0,
        left: 0,
        inv_frames: 0.0,
        attack: 0,
        decay: 0,
        sustain: 1.0,
        release: 0,
    };

    pub(super) fn set(&mut self, adsr: Adsr, sample_period: f32) {
        let frames_per_us = 0.000_001 / sample_period;
        let frames = |us: u32| (us as f32 * frames_per_us + 0.5) as u32;
        let attack = frames(adsr.attack_us);
        let decay = frames(adsr.decay_us);
        let sustain = f32::from(adsr.sustain_percent.min(100)) / 100.0;
        let release = frames(adsr.release_us);

        // Keep the current level and the fraction of time still to run.
        // A sustain edit on a held plateau gets a short, click-free ramp.
        match self.stage {
            Stage::Attack if self.attack != attack => self.retime(self.attack, attack),
            Stage::Release if self.release != release => self.retime(self.release, release),
            Stage::Decay if self.decay != decay || self.sustain != sustain => {
                self.retime(self.decay, decay);
            }
            Stage::Sustain if self.sustain != sustain => self.begin(Stage::Sustain, frames(4_000)),
            _ => (),
        }
        self.attack = attack;
        self.decay = decay;
        self.sustain = sustain;
        self.release = release;
    }

    pub(super) fn gate_on(&mut self) {
        if self.stage == Stage::Release {
            self.begin(Stage::Attack, self.attack);
        }
    }

    pub(super) fn gate_off(&mut self) {
        if self.stage != Stage::Release {
            self.begin(Stage::Release, self.release);
            if self.release == 0 {
                self.level = 0.0;
            }
        }
    }

    fn retime(&mut self, old: u32, new: u32) {
        let left = (self.left as f32 * (new as f32 / old.max(1) as f32) + 0.5) as u32;
        self.begin(self.stage, left);
    }

    fn begin(&mut self, stage: Stage, frames: u32) {
        self.stage = stage;
        self.from = self.level;
        self.left = frames;
        self.inv_frames = 1.0 / frames.max(1) as f32;
    }

    pub(super) fn next(&mut self) -> f32 {
        if self.stage == Stage::Attack && self.left == 0 {
            self.level = 1.0;
            self.begin(Stage::Decay, self.decay);
        }
        let target = match self.stage {
            Stage::Attack => 1.0,
            Stage::Decay | Stage::Sustain => self.sustain,
            Stage::Release => 0.0,
        };
        if self.left == 0 {
            self.level = target;
        } else {
            self.left = self.left.wrapping_sub(1);
            let left = self.left as f32 * self.inv_frames;
            // Linear attack and curved decay/release, with exact finite endpoints.
            let weight = if self.stage == Stage::Attack {
                left
            } else {
                left * left
            };
            self.level = target + (self.from - target) * weight;
        }
        if self.stage == Stage::Decay && self.left == 0 {
            self.stage = Stage::Sustain;
        }
        self.level
    }
}
