//! Full-rate DCO, shared modulation and four-pole voice filter.

use super::{
    Patch,
    envelope::{Envelope, Timing},
};
use wavegraph_core::{midi::Note, polyphony::VoiceMode};

#[derive(Debug)]
pub(super) struct Tone {
    pwm_step: u32,
    pwm_depth: f32,
    pwm_moving: bool,
    noise_level: f32,
    pulse_wave: u8,
    saw_wave: u8,
    saw_mask: u32,
    sub_mask: u32,
    sub_gain: f32,
    sub_bias: f32,
    hpf_mix: f32,
    hpf_pole: f32,
    feedback: f32,
    cutoff: f32,
    env_depth: f32,
    g_max: f32,
    period: f32,
    sweep: f32,
    pitch_reference: f32,
}

impl Tone {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(super) fn new(p: Patch, period: f32) -> Self {
        let cutoff = f32::from(p.cutoff.min(127)) / 127.0;
        let feedback = f32::from(p.resonance.min(127)) * (2.0 / 127.0);
        let sub_gain = f32::from(p.sub_level.min(3)) / 6.0;
        let (sub_mask, mean) = match p.sub_wave {
            1 | 5 => (0xc000_0000, 0.25),
            2 => (0xa000_0000, 0.25),
            3 => (0x9000_0000, 0.25),
            _ => (0x8000_0000, 0.5),
        };
        let rate = f32::from(p.pwm_rate.min(127));
        let pitch_reference = f32::from(p.pitch_reference.clamp(64, 127)) / 127.0;
        Self {
            pwm_step: ((0.02 + 0.000355 * rate * rate) * period * 4294967296.0) as u32,
            pwm_moving: p.pwm_rate != 0,
            pwm_depth: 0.94 * f32::from(p.pwm.min(127)) / 127.0,
            noise_level: f32::from(p.noise.min(3)),
            pulse_wave: p.pulse_wave,
            saw_wave: p.saw_wave,
            // Bits high in the gated half/eighth-cycle saws. Saw 3 uses PWM.
            saw_mask: match p.saw_wave {
                2 => 1 << 30,
                4 => 1 << 28,
                5 => (1 << 30) | (1 << 28),
                _ => 0,
            },
            sub_mask: if p.sub_wave < 4 {
                sub_mask >> 1
            } else {
                sub_mask
            },
            sub_gain,
            sub_bias: mean * sub_gain,
            hpf_mix: match p.hpf {
                0 => 0.5,
                1 => 0.0,
                _ => -1.0,
            },
            hpf_pole: ((if p.hpf >= 3 { 3500.0 } else { 850.0 }) * period).min(1.0),
            feedback,
            cutoff: 700.0 + 90_000.0 * cutoff * cutoff,
            env_depth: 1.18 * (f32::from(p.filter_env.min(127)) / 127.0),
            g_max: 0.94 - 0.15 * feedback,
            period,
            sweep: (0.75 * f32::from(p.sweep.min(127)) / 127.0) / pitch_reference,
            pitch_reference,
        }
    }

    fn filter_coefficient(&self, level: f32) -> f32 {
        (self.cutoff * (0.30 + self.env_depth * level) * self.period).clamp(0.001, self.g_max)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    pulse_at: f32,
    saw_at: f32,
    bias: f32,
}
impl Frame {
    pub(super) const ZERO: Self = Self {
        pulse_at: 0.0,
        saw_at: 0.0,
        bias: 0.0,
    };
}

#[derive(Debug)]
pub(super) struct Modulation {
    phase: u32,
    noise: u32,
}

impl Modulation {
    pub(super) const RESET: Self = Self { phase: 0, noise: 1 };
    const LCG_MUL: u32 = 1_664_525;
    const LCG_ADD: u32 = 1_013_904_223;

    pub(super) fn frame(&mut self, tone: &Tone) -> Frame {
        let hi = (self.phase >> 16) as u16;
        let folded = if hi < 32768 { hi } else { !hi };
        let motion = if tone.pwm_moving {
            f32::from(folded) / 32768.0
        } else {
            1.0
        };
        self.phase = self.phase.wrapping_add(tone.pwm_step);
        let width = 1.0 - tone.pwm_depth * motion;
        let duty = match tone.pulse_wave {
            1 => 0.5,
            2 => 0.25,
            3 => width * 0.5,
            _ => 0.0,
        };
        let saw_dc = match tone.saw_wave {
            1 => 0.5,
            2 => 0.3125,
            3 => width * (3.0 - width) * 0.25,
            4 => 0.265625,
            5 => 0.1640625,
            _ => 0.0,
        };
        self.noise = self
            .noise
            .wrapping_mul(Self::LCG_MUL)
            .wrapping_add(Self::LCG_ADD);
        let noise = f32::from((self.noise >> 16) as u16) * (0.018 / 65536.0) - 0.009;
        Frame {
            pulse_at: 65536.0 * (1.0 - duty),
            saw_at: 65536.0 * (1.0 - width),
            // Remove the DC component of each waveform once for all voices.
            bias: noise * tone.noise_level - 0.30 * duty - 0.43 * saw_dc - tone.sub_bias,
        }
    }

    /// Skip an inaudible span without resetting the PWM or noise on the next key.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "phase arithmetic is modulo 2^32"
    )]
    pub(super) fn advance(&mut self, mut n: usize, tone: &Tone) {
        self.phase = self
            .phase
            .wrapping_add(tone.pwm_step.wrapping_mul(n as u32));
        // Square the LCG's affine transform to advance by powers of two.
        let (mut mul, mut add) = (Self::LCG_MUL, Self::LCG_ADD);
        while n != 0 {
            if n & 1 != 0 {
                self.noise = self.noise.wrapping_mul(mul).wrapping_add(add);
            }
            add = add.wrapping_mul(mul.wrapping_add(1));
            mul = mul.wrapping_mul(mul);
            n >>= 1;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Voice {
    pub(super) note: Note,
    pub(super) env: Envelope,
    phase: u32,
    base_step: f32,
    filter: [f32; 4],
    low: f32,
    gain: f32,
}
impl Voice {
    pub(super) const EMPTY: Self = Self {
        note: Note {
            number: 0,
            velocity: 0,
        },
        env: Envelope::IDLE,
        phase: 0,
        base_step: 0.0,
        filter: [0.0; 4],
        low: 0.0,
        gain: 0.0,
    };

    #[allow(clippy::cast_precision_loss)]
    pub(super) fn strike(note: Note, step: u32, t: &Timing, mode: VoiceMode) -> Self {
        let gain = f32::from(note.velocity) / (127.0 * 4.0);
        let mut voice = Self {
            note,
            // The accumulator runs at the two-octaves-down sub frequency.
            base_step: step as f32 * 0.25,
            // Four nearly phase-aligned unison DCOs can add coherently on attack.
            gain: if mode == VoiceMode::Unison {
                gain * 0.5
            } else {
                gain
            },
            ..Self::EMPTY
        };
        voice.env.strike(t);
        voice
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the quarter-rate increment is finite and fits u32 even at maximum pitch overshoot"
    )]
    fn increment(&self, level: f32, tone: &Tone) -> u32 {
        (self.base_step * (1.0 - tone.sweep * (tone.pitch_reference - level))) as u32
    }

    #[inline(always)]
    fn sample(&mut self, frame: &Frame, tone: &Tone, step: u32, g: f32) -> f32 {
        self.phase = self.phase.wrapping_add(step);
        let phase = self.phase.wrapping_shl(2);
        let ramp = f32::from((phase >> 16) as u16);
        let saw_open = if tone.saw_wave == 3 {
            f32::from((phase.wrapping_shl(1) >> 16) as u16) >= frame.saw_at
        } else {
            (1..=5).contains(&tone.saw_wave) && phase & tone.saw_mask == tone.saw_mask
        };
        let mut x = frame.bias;
        if ramp >= frame.pulse_at {
            x += 0.30;
        }
        if saw_open {
            x += (0.43 / 65536.0) * ramp;
        }
        if self.phase & tone.sub_mask == tone.sub_mask {
            x += tone.sub_gain;
        }
        self.low += tone.hpf_pole * (x - self.low);
        x += tone.hpf_mix * self.low;
        x -= tone.feedback * self.filter[3];
        for state in &mut self.filter {
            *state += g * (x - *state);
            x = *state;
        }
        x
    }

    pub(super) fn mix(&mut self, frames: &[Frame], dry: &mut [f32], tone: &Tone, t: &Timing) {
        if self.env.idle() {
            return;
        }
        if let Some(level) = self.env.held_level() {
            let step = self.increment(level, tone);
            let g = tone.filter_coefficient(level);
            for (frame, out) in frames.iter().zip(dry) {
                *out += self.sample(frame, tone, step, g) * level * self.gain;
            }
        } else {
            for (frame, out) in frames.iter().zip(dry) {
                let level = self.env.sample(t);
                let step = self.increment(level, tone);
                *out += self.sample(frame, tone, step, tone.filter_coefficient(level))
                    * level
                    * self.gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominator_overshoots_and_settles_at_the_played_pitch() {
        let patch = Patch::DOMINATOR;
        let voice = Voice {
            base_step: 1_000_000.0,
            ..Voice::EMPTY
        };
        let tone = Tone::new(patch, 1.0 / 44_118.0);
        let hold = f32::from(patch.levels[2]) / 127.0;
        assert!(voice.increment(0.0, &tone).abs_diff(250_000) <= 1);
        assert!((1_330_000..1_350_000).contains(&voice.increment(1.0, &tone)));
        assert_eq!(voice.increment(hold, &tone), 1_000_000);

        let dry_pitch = Tone::new(Patch { sweep: 0, ..patch }, 1.0 / 44_118.0);
        for level in [0.0, hold, 1.0] {
            assert_eq!(voice.increment(level, &dry_pitch), 1_000_000);
        }
    }
}
