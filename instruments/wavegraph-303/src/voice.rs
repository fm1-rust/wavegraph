use crate::Envelopes;
use crate::dsp::{clamp, fast_exp2, fast_tanh, midi_to_hz, tau_to_coef};

const STAGE_POLES: [f32; 4] = [0.128, 1.0382, 2.3254, 3.2356];

/// Voice calibration and panel settings in the units of the upstream DSP.
/// Supply positive cutoff and ladder input gain, resonance and accent in
/// 0..=1, and pulse width in 0.05..=0.95.
#[derive(Debug, Clone, Copy)]
pub struct VoiceParams {
    /// Filter cutoff in Hz.
    pub cutoff_hz: f32,
    /// Resonance amount, from 0 to 1.
    pub resonance: f32,
    /// Filter-envelope depth in octaves.
    pub env_oct: f32,
    /// Unaccented filter-envelope decay in seconds.
    pub decay_s: f32,
    /// VCA attack time constant in seconds.
    pub attack_s: f32,
    /// Oscillator gain into the resonant filter, in dB above stock level (0 to 36).
    pub overdrive_db: f32,
    /// Accent amount, from 0 to 1.
    pub accent: f32,
    /// Tuning offset in semitones.
    pub tuning_st: f32,
    /// Slide time in seconds.
    pub slide_s: f32,
    /// Select the pulse waveform. The voice crossfades when this changes.
    pub square: bool,
    /// Pulse duty cycle, normally 0.05 to 0.95.
    pub pulse_width: f32,
    /// Ladder feedback gain at full resonance.
    pub max_loop_gain: f32,
    /// Saturation inside each ladder stage.
    pub stage_drive: f32,
    /// Fraction of the low-frequency loss compensated at resonance.
    pub bass_makeup: f32,
    /// Positive gain feeding the nonlinear ladder.
    pub ladder_in: f32,
    /// Post-filter high-pass cutoff in Hz. Zero bypasses it.
    pub post_hp_hz: f32,
}

impl Default for VoiceParams {
    fn default() -> Self {
        Self {
            cutoff_hz: 500.0,
            resonance: 0.5,
            env_oct: 2.0,
            decay_s: 0.6,
            attack_s: 0.0015,
            overdrive_db: 0.0,
            accent: 0.5,
            tuning_st: 0.0,
            slide_s: 0.06,
            square: false,
            pulse_width: 0.5,
            max_loop_gain: 16.5,
            stage_drive: 0.75,
            bass_makeup: 0.5,
            ladder_in: 0.4,
            post_hp_hz: 120.0,
        }
    }
}

#[derive(Debug)]
struct DiodeLadder {
    rate: f32,
    state: [f32; 4],
    coeff: [f32; 4],
    hp: f32,
    hp_coef: f32,
    stage_drive: f32,
}

/// Monophonic oscillator, resonant ladder, envelopes, accent circuit and glide.
#[derive(Debug)]
pub struct Voice {
    sr: f32,
    phase: f32,
    square_mix: f32,
    pitch: f32,
    target: f32,
    ladder: DiodeLadder,
    until_cutoff: u8,
    waveform_slew: f32,
    envelopes: Envelopes,
    post_lp: f32,
    sliding: bool,
}

impl Voice {
    /// Initialize at the host's physical sample rate, in Hz.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            phase: 0.0,
            square_mix: 0.0,
            pitch: 36.0,
            target: 36.0,
            ladder: DiodeLadder {
                rate: 2.0 * sample_rate,
                state: [0.0; 4],
                coeff: [0.5; 4],
                hp: 0.0,
                hp_coef: tau_to_coef(
                    1.0 / (2.0 * core::f32::consts::PI * 12.0),
                    2.0 * sample_rate,
                ),
                stage_drive: 0.0,
            },
            until_cutoff: 0,
            waveform_slew: tau_to_coef(0.0015, sample_rate),
            envelopes: Envelopes::new(sample_rate),
            post_lp: 0.0,
            sliding: false,
        }
    }
}

impl DiodeLadder {
    fn reset(&mut self) {
        self.state = [0.0; 4];
        self.hp = 0.0;
    }

    fn set_cutoff(&mut self, wc_hz: f32) {
        for (coeff, pole) in self.coeff.iter_mut().zip(STAGE_POLES) {
            let f = clamp(wc_hz * pole, 10.0, 0.45 * self.rate);
            let g = crate::dsp::filter_tan(core::f32::consts::PI * f / self.rate);
            *coeff = g / (1.0 + g);
        }
    }

    fn saturate(&self, x: f32) -> f32 {
        if self.stage_drive > 0.0 {
            fast_tanh(self.stage_drive * x) / self.stage_drive
        } else {
            x
        }
    }

    fn stage(x: f32, g: f32, z: &mut f32) -> f32 {
        let v = g * (x - *z);
        let y = v + *z;
        *z = y + v;
        y
    }

    fn process(&mut self, x: f32, k: f32) -> f32 {
        // Zero-delay feedback: each stage is y = G x + (1 - G) z, so the
        // chain's output is Gamma u + Sigma, and the loop u = x - k (y - hp)
        // solves for this step's output directly. (Feeding back the last
        // step's output instead made the resonance run away above ~6 kHz.)
        // The input saturation is then applied to the solved input.
        let g01 = self.coeff[0] * self.coeff[1];
        let g23 = self.coeff[2] * self.coeff[3];
        let gamma = g01 * g23;
        let sigma = self.coeff[1] * g23 * (1.0 - self.coeff[0]) * self.state[0]
            + g23 * (1.0 - self.coeff[1]) * self.state[1]
            + self.coeff[3] * (1.0 - self.coeff[2]) * self.state[2]
            + (1.0 - self.coeff[3]) * self.state[3];
        let y_est = (gamma * (x + k * self.hp) + sigma) / (1.0 + k * gamma);
        let input = fast_tanh(x - k * (y_est - self.hp));
        let a = Self::stage(self.saturate(input), self.coeff[0], &mut self.state[0]);
        let b = Self::stage(self.saturate(a), self.coeff[1], &mut self.state[1]);
        let c = Self::stage(self.saturate(b), self.coeff[2], &mut self.state[2]);
        let output = Self::stage(self.saturate(c), self.coeff[3], &mut self.state[3]);
        self.hp += (output - self.hp) * self.hp_coef;
        output
    }
}

impl Voice {
    /// Reset oscillator phase, gates and envelopes as in the upstream transport reset.
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.ladder.reset();
        self.until_cutoff = 0;
        self.envelopes.reset();
        self.sliding = false;
    }

    /// Trigger a MIDI note. A slide from an open gate carries the envelopes across.
    #[allow(
        clippy::cast_precision_loss,
        reason = "MIDI notes fit exactly in f32; the raw API accepts the same i32 range."
    )]
    pub fn note_on(&mut self, note: i32, accent: bool, slide: bool) {
        self.target = note as f32;
        self.sliding = slide && self.envelopes.gate;
        if !self.sliding {
            self.pitch = self.target;
        }
        self.envelopes.note_on(accent, slide);
    }

    /// Release the gate.
    pub fn note_off(&mut self) {
        self.envelopes.note_off();
    }

    /// Whether the gate is open.
    #[must_use]
    pub fn gate(&self) -> bool {
        self.envelopes.gate
    }

    /// Whether the upstream gate or envelope is active.
    #[must_use]
    pub fn sounding(&self) -> bool {
        self.envelopes.sounding()
    }

    /// Add mono samples to the output, keeping oscillator and filter history between calls.
    pub fn render_add(&mut self, p: &VoiceParams, out: &mut [f32]) {
        self.render_with(p, out, |dst, sample| *dst += sample);
    }

    pub(crate) fn render_with<S>(
        &mut self,
        p: &VoiceParams,
        out: &mut [S],
        mut write: impl FnMut(&mut S, f32),
    ) {
        // The upstream scheduler updates panel coefficients once per call.
        let glide = if self.sliding {
            tau_to_coef(p.slide_s / 3.0, self.sr)
        } else {
            1.0
        };
        let envelopes = self.envelopes.coefficients(p);
        let input_gain =
            p.ladder_in * fast_exp2(p.overdrive_db / (20.0 * core::f32::consts::LOG10_2));
        let k = p.max_loop_gain * p.resonance;
        self.ladder.stage_drive = p.stage_drive;
        let post_hp = if p.post_hp_hz > 0.0 {
            tau_to_coef(1.0 / (2.0 * core::f32::consts::PI * p.post_hp_hz), self.sr)
        } else {
            0.0
        };
        let base_oct = p.base_octaves();
        for sample in out {
            self.pitch += (self.target - self.pitch) * glide;
            let inc = midi_to_hz(self.pitch + p.tuning_st) / self.sr;
            self.envelopes.step(&envelopes);
            // Cutoff, at 2x the sample rate; the four stages' coefficients
            // are worked out every fourth sample (the envelope moves slowly).
            if self.until_cutoff == 0 {
                self.until_cutoff = 3;
                self.ladder
                    .set_cutoff(self.envelopes.cutoff_hz(p, base_oct));
            } else {
                self.until_cutoff = self.until_cutoff.saturating_sub(1);
            }
            self.square_mix +=
                ((if p.square { 1.0 } else { 0.0 }) - self.square_mix) * self.waveform_slew;
            let x = self.osc(inc, self.square_mix, p.pulse_width) * input_gain;
            // Hold the input for both filter steps and retain the upstream sum and gain.
            let mut y = 0.0;
            for _ in 0..2 {
                y += self.ladder.process(x, k);
            }
            y /= p.ladder_in;
            // Make up some of the level resonance takes (a ladder's gain at
            // low frequencies is 1 / (1 + k)); the rest of the bass loss is
            // the 303's own.
            y *= 0.4 * (1.0 + p.bass_makeup * k);
            if post_hp > 0.0 {
                self.post_lp += (y - self.post_lp) * post_hp;
                y = 1.25 * (y - self.post_lp);
            }
            write(sample, self.envelopes.amplify(y) * 0.265);
        }
    }

    fn osc(&mut self, inc: f32, square: f32, pw: f32) -> f32 {
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let saw = 2.0 * self.phase - 1.0 - Self::blep(self.phase, inc);
        if square < 1e-4 {
            return saw;
        }
        let mut p2 = self.phase + pw;
        if p2 >= 1.0 {
            p2 -= 1.0;
        }
        let saw2 = 2.0 * p2 - 1.0 - Self::blep(p2, inc);
        // The difference of the saws is a pulse between -2pw and 2 - 2pw,
        // whose average is zero at any width.
        let pulse = 0.7 * (saw - saw2);
        if square > 0.9999 {
            pulse
        } else {
            saw + (pulse - saw) * square
        }
    }

    fn blep(mut t: f32, dt: f32) -> f32 {
        if t < dt {
            t /= dt;
            return t + t - t * t - 1.0;
        }
        if t > 1.0 - dt {
            t = (t - 1.0) / dt;
            return t * t + t + t + 1.0;
        }
        0.0
    }
}
