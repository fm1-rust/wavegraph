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
    attack: f32,
    hold: f32,
    release: f32,
    accent_charge: f32,
    accent_glide: f32,
    fenv: f32,
    aenv: f32,
    acc_cap: f32,
    acc_amp: f32,
    post_lp: f32,
    gate: bool,
    amp_on: bool,
    accent: bool,
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
            attack: tau_to_coef(0.0015, sample_rate),
            hold: tau_to_coef(1.34, sample_rate),
            release: tau_to_coef(0.004, sample_rate),
            accent_charge: tau_to_coef(0.012, sample_rate),
            accent_glide: tau_to_coef(0.002, sample_rate),
            fenv: 0.0,
            aenv: 0.0,
            acc_cap: 0.0,
            acc_amp: 1.0,
            post_lp: 0.0,
            gate: false,
            amp_on: false,
            accent: false,
            sliding: false,
        }
    }
}

impl DiodeLadder {
    fn reset(&mut self) {
        self.state = [0.0; 4];
        self.hp = 0.0;
    }

    fn set_cutoff(&mut self, mut wc_hz: f32) {
        // Kept below 16 kHz, so the resonant peak (~1.1 x) stays clear of
        // the top of the audio band.
        wc_hz = clamp(wc_hz, 5.0, 16000.0);
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
        self.acc_cap = 0.0;
        self.aenv = 0.0;
        self.fenv = 0.0;
        self.sliding = false;
        self.accent = false;
        self.gate = false;
    }

    /// Trigger a MIDI note. A slide from an open gate carries the envelopes across.
    #[allow(
        clippy::cast_precision_loss,
        reason = "MIDI notes fit exactly in f32; the raw API accepts the same i32 range."
    )]
    pub fn note_on(&mut self, note: i32, accent: bool, slide: bool) {
        self.target = note as f32;
        self.accent = accent;
        if slide && self.gate {
            self.sliding = true;
        } else {
            self.pitch = self.target;
            self.sliding = false;
            self.fenv = 1.0;
            self.amp_on = true;
        }
        self.gate = true;
    }

    /// Release the gate.
    pub fn note_off(&mut self) {
        self.gate = false;
    }

    /// Whether the gate is open.
    #[must_use]
    pub fn gate(&self) -> bool {
        self.gate
    }

    /// Whether the upstream gate or envelope is active.
    #[must_use]
    pub fn sounding(&self) -> bool {
        self.gate || self.aenv > 0.0001
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
        let decay = if self.accent { 0.2 } else { p.decay_s };
        let fdec = tau_to_coef(decay / 4.6, self.sr);
        // Resonance slows the accent capacitor's drain, as on the 303.
        let acc_drain = tau_to_coef(0.08 + 0.35 * p.resonance, self.sr);
        let k = p.max_loop_gain * p.resonance;
        self.ladder.stage_drive = p.stage_drive;
        // Accent's level boost glides (~2 ms) rather than jumps: a slide
        // between an accented and a plain note changes it mid-note, and a
        // jump there is a click.
        let amp_acc = if self.accent {
            1.0 + 0.9 * p.accent
        } else {
            1.0
        };
        let post_hp = if p.post_hp_hz > 0.0 {
            tau_to_coef(1.0 / (2.0 * core::f32::consts::PI * p.post_hp_hz), self.sr)
        } else {
            0.0
        };
        let base_oct = crate::dsp::cutoff_octaves(p.cutoff_hz * 1.6);
        for sample in out {
            self.pitch += (self.target - self.pitch) * glide;
            let inc = midi_to_hz(self.pitch + p.tuning_st) / self.sr;
            // Envelopes
            self.fenv -= self.fenv * fdec;
            if self.gate {
                if self.amp_on {
                    self.aenv += (1.05 - self.aenv) * self.attack;
                    if self.aenv >= 1.0 {
                        self.aenv = 1.0;
                        self.amp_on = false;
                    }
                } else {
                    self.aenv -= self.aenv * self.hold;
                }
            } else {
                self.aenv -= self.aenv * self.release;
            }
            let acc_in = if self.accent { self.fenv } else { 0.0 };
            self.acc_cap += (acc_in - self.acc_cap)
                * (if acc_in > self.acc_cap {
                    self.accent_charge
                } else {
                    acc_drain
                });
            // Cutoff, at 2x the sample rate; the four stages' coefficients
            // are worked out every fourth sample (the envelope moves slowly).
            if self.until_cutoff == 0 {
                self.until_cutoff = 3;
                let oct = base_oct + p.env_oct * self.fenv + 3.0 * p.accent * self.acc_cap;
                self.ladder.set_cutoff(fast_exp2(oct));
            } else {
                self.until_cutoff = self.until_cutoff.saturating_sub(1);
            }
            self.square_mix += ((if p.square { 1.0 } else { 0.0 }) - self.square_mix) * self.attack;
            let x = self.osc(inc, self.square_mix, p.pulse_width);
            // Hold the input for both filter steps and retain the upstream sum and gain.
            let mut y = 0.0;
            for _ in 0..2 {
                y += self.ladder.process(x * p.ladder_in, k);
            }
            y /= p.ladder_in;
            // Make up some of the level resonance takes (a ladder's gain at
            // low frequencies is 1 / (1 + k)); the rest of the bass loss is
            // the 303's own.
            y *= 0.4 * (1.0 + p.bass_makeup * k);
            self.acc_amp += (amp_acc - self.acc_amp) * self.accent_glide;
            if post_hp > 0.0 {
                self.post_lp += (y - self.post_lp) * post_hp;
                y = 1.25 * (y - self.post_lp);
            }
            write(sample, y * self.aenv * self.acc_amp * 0.265);
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
