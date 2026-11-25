#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

/// Envelope times in microseconds, spaced approximately three steps per octave.
pub const TIMES_US: [u32; 39] = [
    0, 1_500, 2_000, 3_000, 4_000, 5_000, 6_000, 8_000, 10_000, 12_000, 16_000, 20_000, 25_000,
    32_000, 40_000, 50_000, 64_000, 80_000, 100_000, 125_000, 160_000, 200_000, 250_000, 320_000,
    400_000, 500_000, 640_000, 800_000, 1_000_000, 1_250_000, 1_600_000, 2_000_000, 2_500_000,
    3_200_000, 4_000_000, 5_000_000, 6_400_000, 8_000_000, 10_000_000,
];

/// Attack, decay, sustain and release settings, independent of sample rate.
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
    /// Immediate attack and release at full sustain.
    pub const GATE: Self = Self {
        attack_us: 0,
        decay_us: 0,
        sustain_percent: 100,
        release_us: 0,
    };

    /// Prepare the segment lengths for a clock in seconds per sample.
    /// Durations round to the nearest frame. A nonpositive or nonfinite
    /// period gives instantaneous segments.
    #[must_use]
    // Multiple instrument editors share this control-rate conversion in flash.
    #[cfg_attr(target_os = "none", inline(never))]
    #[allow(clippy::cast_lossless, reason = "From is not const on the MSRV.")]
    pub const fn at_sample_period(self, sample_period: f32) -> Timing {
        let frames_per_us = if sample_period > 0.0 && sample_period.is_finite() {
            0.000_001 / sample_period
        } else {
            0.0
        };
        let sustain = if self.sustain_percent > 100 {
            100
        } else {
            self.sustain_percent
        };
        Timing {
            attack: Ramp::from_us(self.attack_us, frames_per_us),
            decay: Ramp::from_us(self.decay_us, frames_per_us),
            sustain: sustain as f32 / 100.0,
            release: Ramp::from_us(self.release_us, frames_per_us),
            sustain_slew: Ramp::from_us(4_000, frames_per_us),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Ramp {
    frames: u32,
    inv_frames: f32,
}

#[allow(
    clippy::cast_precision_loss,
    reason = "Envelope interpolation uses f32."
)]
impl Ramp {
    const fn new(frames: u32) -> Self {
        Self {
            frames,
            inv_frames: if frames == 0 {
                0.0
            } else {
                1.0 / frames as f32
            },
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Positive frame counts round to the nearest frame and saturate at u32::MAX."
    )]
    const fn from_us(us: u32, frames_per_us: f32) -> Self {
        Self::new((us as f32 * frames_per_us + 0.5) as u32)
    }
}

/// Sample-rate-specific ADSR settings. All voices of a polyphonic instrument
/// can use the same timing, including its precomputed ramp reciprocals.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    attack: Ramp,
    decay: Ramp,
    sustain: f32,
    release: Ramp,
    sustain_slew: Ramp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release, // Holds at zero after the ramp ends.
}

/// The gain and gate state of one voice. Advance once per audio frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    stage: Stage,
    level: f32,
    from: f32,
    left: u32,
    inv_frames: f32,
}

impl Envelope {
    /// A closed gate at zero gain, also suitable for a freshly stolen voice.
    pub const IDLE: Self = Self {
        stage: Stage::Release,
        level: 0.0,
        from: 0.0,
        left: 0,
        inv_frames: 0.0,
    };

    /// Current envelope level in 0..=1, without advancing the clock.
    #[must_use]
    pub const fn level(&self) -> f32 {
        self.level
    }

    /// Whether the key is still held, including at zero sustain.
    #[must_use]
    pub fn is_gated(&self) -> bool {
        self.stage != Stage::Release
    }

    /// Whether a released voice has reached silence and can be reclaimed.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        !self.is_gated() && self.level == 0.0
    }

    /// Begin attack from the current level. A gate already held plays legato.
    pub fn gate_on(&mut self, timing: &Timing) {
        if !self.is_gated() {
            self.begin(Stage::Attack, timing.attack);
        }
    }

    /// Release from the current level. Repeated note-offs do not extend the tail.
    pub fn gate_off(&mut self, timing: &Timing) {
        if self.is_gated() {
            self.begin(Stage::Release, timing.release);
            if self.left == 0 {
                self.level = 0.0;
            }
        }
    }

    /// Apply an edit before advancing with the new timing. Preserve the
    /// current gain and the fraction of the active segment still to run.
    /// A sustain edit on a held plateau slews over four milliseconds.
    // Keep one copy of the live-edit path across voices and instruments.
    #[cfg_attr(target_os = "none", inline(never))]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The remaining duration rounds to the nearest nonnegative frame count."
    )]
    pub fn retime(&mut self, old: &Timing, new: &Timing) {
        let (before, after) = match self.stage {
            Stage::Attack if old.attack.frames != new.attack.frames => (old.attack, new.attack),
            Stage::Release if old.release.frames != new.release.frames => {
                (old.release, new.release)
            }
            Stage::Decay if old.decay.frames != new.decay.frames || old.sustain != new.sustain => {
                (old.decay, new.decay)
            }
            Stage::Sustain if old.sustain != new.sustain => {
                self.begin(Stage::Sustain, new.sustain_slew);
                return;
            }
            _ => return,
        };
        let left =
            (self.left as f32 * (after.frames as f32 / before.frames.max(1) as f32) + 0.5) as u32;
        self.begin(self.stage, Ramp::new(left));
    }

    fn begin(&mut self, stage: Stage, ramp: Ramp) {
        self.stage = stage;
        self.from = self.level;
        self.left = ramp.frames;
        self.inv_frames = ramp.inv_frames;
    }

    /// Advance one sample and return its gain in 0..=1.
    #[allow(
        clippy::cast_precision_loss,
        reason = "Envelope interpolation uses f32."
    )]
    pub fn advance(&mut self, timing: &Timing) -> f32 {
        if self.stage == Stage::Attack && self.left == 0 {
            self.level = 1.0;
            self.begin(Stage::Decay, timing.decay);
        }
        let target = match self.stage {
            Stage::Attack => 1.0,
            Stage::Decay | Stage::Sustain => timing.sustain,
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
