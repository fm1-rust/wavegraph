use crate::Envelopes;
use crate::dsp::{clamp, fast_exp2, fast_tanh, midi_to_hz, tau_to_coef};
use crate::envelopes::CachedCoefficient;

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
    radians_per_hz: f32,
    cutoff_hz: f32,
    compact: bool,
    state: [f32; 4],
    coeff: [f32; 4],
    sigma_weights: [f32; 4],
    gamma: f32,
    feedback_recip: f32,
    hp: f32,
    hp_coef: f32,
    stage_drive: f32,
    stage_recip: f32,
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
    glide_coefficient: CachedCoefficient,
    post_hp_coefficient: CachedCoefficient,
    cutoff_octaves: CachedCoefficient,
    tuned_increment: CachedCoefficient,
    oscillator_drive: CachedCoefficient,
}

// Fit the composition tanh_approx(0.75*tanh_approx(x))/0.75 to a rational
// curve in x² over the original rails. Unity small-signal gain and odd symmetry
// are exact. The error against the two original curves is below 0.00003.
#[inline(always)]
fn stock_input(x: f32) -> f32 {
    if x.abs() >= 3.0 {
        return (49.0_f32 / 57.0).copysign(x);
    }
    let z = x * x;
    x * (1.0 + z * (0.377_580_77 + 0.011_424_99 * z)) / (1.0 + z * (0.840_034_1 + 0.123_675_29 * z))
}

// On the bounded part of an integrator's output a seventh-degree odd fit
// replaces a division. Use the original curve for excursions beyond that fit.
#[inline(always)]
fn stock_stage(x: f32) -> f32 {
    if x.abs() > 1.0 {
        return fast_tanh(0.75 * x) * (1.0 / 0.75);
    }
    let z = x * x;
    x * (1.0 + z * (-0.166_506_17 + z * (0.030_318_24 - 0.004_172_8 * z)))
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
                radians_per_hz: core::f32::consts::PI / (2.0 * sample_rate),
                cutoff_hz: f32::NAN,
                compact: false,
                state: [0.0; 4],
                coeff: [0.5; 4],
                sigma_weights: [0.0; 4],
                gamma: 0.0,
                feedback_recip: 1.0,
                hp: 0.0,
                hp_coef: tau_to_coef(
                    1.0 / (2.0 * core::f32::consts::PI * 12.0),
                    2.0 * sample_rate,
                ),
                stage_drive: 0.0,
                stage_recip: 0.0,
            },
            until_cutoff: 0,
            waveform_slew: tau_to_coef(0.0015, sample_rate),
            envelopes: Envelopes::new(sample_rate),
            post_lp: 0.0,
            sliding: false,
            glide_coefficient: CachedCoefficient::EMPTY,
            post_hp_coefficient: CachedCoefficient::EMPTY,
            cutoff_octaves: CachedCoefficient::EMPTY,
            tuned_increment: CachedCoefficient::EMPTY,
            oscillator_drive: CachedCoefficient::EMPTY,
        }
    }
}

impl DiodeLadder {
    fn reset(&mut self) {
        self.state = [0.0; 4];
        self.hp = 0.0;
    }

    fn set_feedback(&mut self, k: f32) {
        self.feedback_recip = 1.0 / (1.0 + k * self.gamma);
    }

    fn set_drive(&mut self, drive: f32) {
        if drive == self.stage_drive {
            return;
        }
        self.stage_drive = drive;
        self.stage_recip = if drive > 0.0 { 1.0 / drive } else { 0.0 };
    }

    // SAFETY: The FM-1 startup copies this section into executable SRAM.
    #[cfg_attr(
        all(target_os = "none", feature = "fm1-ram-code"),
        allow(unsafe_code),
        unsafe(link_section = ".data.wavegraph_audio"),
        inline(never)
    )]
    fn set_cutoff(&mut self, wc_hz: f32, k: f32) {
        if self.cutoff_hz == wc_hz {
            self.set_feedback(k);
            return;
        }
        self.cutoff_hz = wc_hz;
        for (coeff, pole) in self.coeff.iter_mut().zip(STAGE_POLES) {
            let f = clamp(wc_hz * pole, 10.0, 0.45 * self.rate);
            *coeff = crate::dsp::ladder_coefficient(f * self.radians_per_hz);
        }
        let [a, b, c, d] = self.coeff;
        let cd = c * d;
        self.gamma = (a * b) * cd;
        self.sigma_weights = [b * cd * (1.0 - a), cd * (1.0 - b), d * (1.0 - c), 1.0 - d];
        self.set_feedback(k);
    }

    fn saturate(&self, x: f32) -> f32 {
        if self.stage_drive > 0.0 {
            let y = fast_tanh(self.stage_drive * x);
            if self.stage_recip.is_finite() {
                y * self.stage_recip
            } else {
                // Preserve the original behavior for tiny positive raw API drives.
                y / self.stage_drive
            }
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

    // The FM-1 uses a positive drive with a finite reciprocal. Keep the raw
    // API's bypass and tiny-drive division fallback outside the SRAM renderer.
    #[inline(never)]
    fn exceptional_stages(&mut self, input: f32) -> f32 {
        let a = Self::stage(self.saturate(input), self.coeff[0], &mut self.state[0]);
        let b = Self::stage(self.saturate(a), self.coeff[1], &mut self.state[1]);
        let c = Self::stage(self.saturate(b), self.coeff[2], &mut self.state[2]);
        Self::stage(self.saturate(c), self.coeff[3], &mut self.state[3])
    }

    // Use one copy of the two-tick kernel for the clean-tap and single-output voices.
    // SAFETY: The FM-1 startup copies this section into executable SRAM.
    #[cfg_attr(
        all(target_os = "none", feature = "fm1-ram-code"),
        allow(unsafe_code),
        unsafe(link_section = ".data.wavegraph_audio"),
        inline(never)
    )]
    fn pair(&mut self, x: f32, k: f32) -> f32 {
        if self.compact && self.stage_drive == 0.75 {
            self.process::<true>(x, k) + self.process::<true>(x, k)
        } else {
            self.direct_pair(x, k)
        }
    }

    // Keep the direct calibration path out of the FM-1's limited audio SRAM.
    #[inline(never)]
    fn direct_pair(&mut self, x: f32, k: f32) -> f32 {
        self.process::<false>(x, k) + self.process::<false>(x, k)
    }

    #[inline(always)]
    fn process<const COMPACT: bool>(&mut self, x: f32, k: f32) -> f32 {
        // Zero-delay feedback: each stage is y = G x + (1 - G) z, so the
        // chain's output is Gamma u + Sigma, and the loop u = x - k (y - hp)
        // solves for this step's output directly. (Feeding back the last
        // step's output instead made the resonance run away above ~6 kHz.)
        // The input saturation is then applied to the solved input.
        let [a, b, c, d] = self.sigma_weights;
        let sigma = a * self.state[0] + b * self.state[1] + c * self.state[2] + d * self.state[3];
        let output = if COMPACT {
            // Solve directly for the feedback-corrected input, using
            // 1/(1+k*gamma). The input and first stage clips share no state.
            let u = (x + k * (self.hp - sigma)) * self.feedback_recip;
            let a = Self::stage(stock_input(u), self.coeff[0], &mut self.state[0]);
            let b = Self::stage(stock_stage(a), self.coeff[1], &mut self.state[1]);
            let c = Self::stage(stock_stage(b), self.coeff[2], &mut self.state[2]);
            Self::stage(stock_stage(c), self.coeff[3], &mut self.state[3])
        } else {
            let y_est = (self.gamma * (x + k * self.hp) + sigma) * self.feedback_recip;
            let input = fast_tanh(x - k * (y_est - self.hp));
            self.direct_stages(input)
        };
        self.hp += (output - self.hp) * self.hp_coef;
        output
    }

    #[inline(never)]
    fn direct_stages(&mut self, input: f32) -> f32 {
        if self.stage_drive > 0.0 && self.stage_recip.is_finite() {
            let drive = self.stage_drive;
            let reciprocal = self.stage_recip;
            let scaled = |value| fast_tanh(drive * value) * reciprocal;
            let a = Self::stage(scaled(input), self.coeff[0], &mut self.state[0]);
            let b = Self::stage(scaled(a), self.coeff[1], &mut self.state[1]);
            let c = Self::stage(scaled(b), self.coeff[2], &mut self.state[2]);
            Self::stage(scaled(c), self.coeff[3], &mut self.state[3])
        } else {
            self.exceptional_stages(input)
        }
    }
}

impl Voice {
    pub(crate) fn use_compact_ladder(&mut self) {
        self.ladder.compact = true;
    }

    /// Reset oscillator phase, gates and envelopes as in the upstream transport reset.
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.ladder.reset();
        self.until_cutoff = 0;
        self.envelopes.reset();
        self.pitch = self.target;
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

    pub(crate) fn skip_silence(&mut self, p: &VoiceParams, frames: usize) -> bool {
        if self.envelopes.silent() {
            self.envelopes.advance_silence(p, frames);
            return true;
        }
        false
    }

    pub(crate) fn render_with<S>(
        &mut self,
        p: &VoiceParams,
        out: &mut [S],
        mut write: impl FnMut(&mut S, f32),
    ) -> Option<usize> {
        // Check public parameters at every span boundary, while retaining
        // coefficients whose individual inputs have not changed.
        let sample_rate = self.sr;
        let glide = if self.sliding {
            self.glide_coefficient
                .get(p.slide_s, || tau_to_coef(p.slide_s / 3.0, sample_rate))
        } else {
            1.0
        };
        let envelopes = self.envelopes.coefficients(p);
        let input_gain = p.ladder_in
            * self.oscillator_drive.get(p.overdrive_db, || {
                fast_exp2(p.overdrive_db / (20.0 * core::f32::consts::LOG10_2))
            });
        let k = p.max_loop_gain * p.resonance;
        self.ladder.set_drive(p.stage_drive);
        // Resonance can change on an event boundary before the next cutoff update.
        self.ladder.set_feedback(k);
        let post_hp = if p.post_hp_hz > 0.0 {
            self.post_hp_coefficient.get(p.post_hp_hz, || {
                tau_to_coef(
                    1.0 / (2.0 * core::f32::consts::PI * p.post_hp_hz),
                    sample_rate,
                )
            })
        } else {
            0.0
        };
        let base_oct = self.cutoff_octaves.get(p.cutoff_hz, || p.base_octaves());
        let note = self.target + p.tuning_st;
        let fixed_inc = self
            .tuned_increment
            .get(note, || midi_to_hz(note) / sample_rate);
        let mut last_cycle = None;
        for (frame, sample) in out.iter_mut().enumerate() {
            let inc = if self.sliding {
                self.pitch += (self.target - self.pitch) * glide;
                midi_to_hz(self.pitch + p.tuning_st) / self.sr
            } else {
                fixed_inc
            };
            self.envelopes.step(&envelopes);
            // Cutoff, at 2x the sample rate; the four stages' coefficients
            // are worked out every fourth sample (the envelope moves slowly).
            if self.until_cutoff == 0 {
                self.until_cutoff = 3;
                self.ladder
                    .set_cutoff(self.envelopes.cutoff_hz(p, base_oct), k);
            } else {
                self.until_cutoff = self.until_cutoff.saturating_sub(1);
            }
            self.square_mix +=
                ((if p.square { 1.0 } else { 0.0 }) - self.square_mix) * self.waveform_slew;
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                last_cycle = Some(frame);
            }
            let x = self.osc(inc, self.square_mix, p.pulse_width) * input_gain;
            // Hold the input for both filter steps and retain the upstream sum and gain.
            let mut y = self.ladder.pair(x, k);
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
        last_cycle
    }

    fn osc(&self, inc: f32, square: f32, pw: f32) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    // Keep the original four independent saturate calls as the reference for
    // both normal scaling and the raw API's bypass/division paths.
    fn original_ladder_process(ladder: &mut DiodeLadder, x: f32, k: f32) -> f32 {
        let [a, b, c, d] = ladder.sigma_weights;
        let sigma =
            a * ladder.state[0] + b * ladder.state[1] + c * ladder.state[2] + d * ladder.state[3];
        let y_est = (ladder.gamma * (x + k * ladder.hp) + sigma) * ladder.feedback_recip;
        let input = fast_tanh(x - k * (y_est - ladder.hp));
        let a = DiodeLadder::stage(
            ladder.saturate(input),
            ladder.coeff[0],
            &mut ladder.state[0],
        );
        let b = DiodeLadder::stage(ladder.saturate(a), ladder.coeff[1], &mut ladder.state[1]);
        let c = DiodeLadder::stage(ladder.saturate(b), ladder.coeff[2], &mut ladder.state[2]);
        let output = DiodeLadder::stage(ladder.saturate(c), ladder.coeff[3], &mut ladder.state[3]);
        ladder.hp += (output - ladder.hp) * ladder.hp_coef;
        output
    }

    #[test]
    fn ladder_stage_specialization_preserves_output_and_all_states() {
        // Include both sides of reciprocal overflow among the subnormal
        // drives, signed zero, bypass, and the FM-1's fixed drive of 0.75.
        const DRIVES: [f32; 19] = [
            0.0,
            -0.0,
            -0.75,
            f32::from_bits(1),
            f32::from_bits(0x001f_ffff),
            f32::from_bits(0x0020_0000),
            f32::from_bits(0x0020_0001),
            f32::from_bits(0x007f_ffff),
            f32::MIN_POSITIVE,
            1e-30,
            0.0001,
            0.125,
            0.75,
            1.0,
            1.25,
            4.0,
            16.0,
            256.0,
            f32::MAX,
        ];
        const INPUTS: [f32; 16] = [
            -16.0,
            -3.000_001,
            -3.0,
            -2.999_999,
            -1.0,
            -f32::MIN_POSITIVE,
            -f32::from_bits(1),
            -0.0,
            0.0,
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            1.0,
            2.999_999,
            3.0,
            3.000_001,
            16.0,
        ];
        let mut seed = 0x3a03_7da9_u32;
        for rate in [8000.0, 44_118.0, 96_000.0] {
            for start in 0..DRIVES.len() {
                let mut actual = Voice::new(rate).ladder;
                let mut expected = Voice::new(rate).ladder;
                for frame in 0..2048 {
                    let drive = DRIVES[(start + frame / 101) % DRIVES.len()];
                    let k = [0.0, 0.495, 4.95, 14.85, 16.5][frame / 13 % 5];
                    for ladder in [&mut actual, &mut expected] {
                        ladder.set_drive(drive);
                        if frame % 17 == 0 {
                            let cutoff =
                                [62.0, 110.0, 630.0, 2000.0, 6000.0, 16_000.0][frame / 17 % 6];
                            ladder.set_cutoff(cutoff, k);
                        } else {
                            ladder.set_feedback(k);
                        }
                        if frame % 509 == 0 {
                            ladder.reset();
                        }
                    }
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let bytes = seed.to_le_bytes();
                    let random = f32::from(u16::from_le_bytes([bytes[2], bytes[3]]));
                    let input = if frame % 4 == 0 {
                        INPUTS[frame / 4 % INPUTS.len()]
                    } else {
                        random * (1.0 / 1024.0) - 32.0
                    };
                    let output = actual.process::<false>(input, k);
                    let reference = original_ladder_process(&mut expected, input, k);
                    assert!(
                        output.is_finite(),
                        "rate={rate}, drive={drive}, frame={frame}"
                    );
                    assert_eq!(output.to_bits(), reference.to_bits());
                    // These are all the state variables process can change;
                    // retaining them exactly also checks subsequent samples.
                    assert_eq!(
                        actual.state.map(f32::to_bits),
                        expected.state.map(f32::to_bits)
                    );
                    assert_eq!(actual.hp.to_bits(), expected.hp.to_bits());
                }
            }
        }
    }

    #[test]
    fn cached_coefficients_preserve_audio_and_clock_across_public_parameter_edits() {
        // Recomputing every coefficient is the original control-rate behavior.
        // Compare whole sample streams, not just cached coefficient values.
        for rate in [8000.0, 44_118.0, 96_000.0] {
            let mut cached = Voice::new(rate);
            let mut original = Voice::new(rate);
            let mut p = VoiceParams::default();
            for block in 0..180 {
                if block % 17 == 0 {
                    p.decay_s = [0.0, 0.05, 0.2, 0.6, 2.0][block / 17 % 5];
                }
                if block % 19 == 0 {
                    p.attack_s = [0.0, 0.0015, 0.060][block / 19 % 3];
                }
                if block % 23 == 0 {
                    p.slide_s = [0.0, 0.006, 0.06, 0.2][block / 23 % 4];
                }
                if block % 13 == 0 {
                    p.post_hp_hz = [0.0, 20.0, 120.0, 600.0][block / 13 % 4];
                }
                // Include continuous resonance modulation while other inputs
                // remain fixed, and return to previously cached settings.
                p.resonance = [0.0, 0.3, 0.6, 0.9, 1.0, 0.9, 0.6, 0.3][block % 8];
                p.square = block % 29 >= 14;
                for voice in [&mut cached, &mut original] {
                    match block % 30 {
                        0 => voice.note_on(36, false, false),
                        4 => voice.note_on(43, true, true),
                        8 => voice.note_on(50, false, true),
                        12 => voice.note_off(),
                        22 => voice.reset(),
                        _ => {}
                    }
                }
                p.cutoff_hz = [80.0, 500.0, 1600.0][block / 15 % 3];
                p.tuning_st = [0.0, -1.0, 0.5][block / 11 % 3];
                p.overdrive_db = [0.0, 12.0, 36.0][block / 19 % 3];
                original.cutoff_octaves = CachedCoefficient::EMPTY;
                original.tuned_increment = CachedCoefficient::EMPTY;
                original.oscillator_drive = CachedCoefficient::EMPTY;
                original.glide_coefficient = CachedCoefficient::EMPTY;
                original.post_hp_coefficient = CachedCoefficient::EMPTY;
                original.envelopes.invalidate_coefficients();
                let frames = [1, 17, 32, 127, 128, 257][block % 6];
                let mut actual = [0.0_f32; 257];
                let mut expected = [0.0_f32; 257];
                let idle = cached.skip_silence(&p, frames);
                assert_eq!(idle, original.skip_silence(&p, frames));
                if !idle {
                    let cycle = cached.render_with(&p, &mut actual[..frames], |out, x| *out = x);
                    let reference =
                        original.render_with(&p, &mut expected[..frames], |out, x| *out = x);
                    assert_eq!(cycle, reference);
                }
                for (sample, reference) in actual[..frames].iter().zip(&expected[..frames]) {
                    assert!(sample.is_finite());
                    assert_eq!(sample.to_bits(), reference.to_bits());
                }
            }
        }
    }

    #[test]
    fn stock_ladder_fits_follow_the_two_direct_clippers() {
        for i in i16::MIN..=i16::MAX {
            let x = f32::from(i) / 4096.0;
            let first = fast_tanh(0.75 * fast_tanh(x)) / 0.75;
            let single = fast_tanh(0.75 * x) / 0.75;
            assert!((stock_input(x) - first).abs() < 0.000_03);
            assert!((stock_stage(x) - single).abs() < 0.000_011);
        }
        for drive in [0.0, 0.125, 0.74, 0.76, 2.0] {
            let mut a = Voice::new(44_118.0);
            let mut b = Voice::new(44_118.0);
            b.use_compact_ladder();
            let p = VoiceParams {
                stage_drive: drive,
                resonance: 0.95,
                square: true,
                ..VoiceParams::default()
            };
            for v in [&mut a, &mut b] {
                v.note_on(43, true, false);
            }
            let mut a_out = [0.0; 512];
            let mut b_out = [0.0; 512];
            a.render_add(&p, &mut a_out);
            b.render_add(&p, &mut b_out);
            assert_eq!(a_out, b_out);
        }
    }

    #[test]
    fn stage_reciprocal_matches_direct_scaling() {
        let mut voice = Voice::new(44_118.0);
        for drive in [0.0, f32::from_bits(1), 0.125, 0.75, 1.25, 4.0] {
            voice.ladder.set_drive(drive);
            for x in [-4.0, -1.0, -0.125, 0.0, 0.125, 1.0, 4.0] {
                let expected = if drive > 0.0 {
                    fast_tanh(drive * x) / drive
                } else {
                    x
                };
                let actual = voice.ladder.saturate(x);
                assert!((actual - expected).abs() <= 2e-7 * expected.abs().max(1.0));
            }
        }
    }

    #[test]
    fn feedback_reciprocal_tracks_resonance_between_cutoff_updates() {
        let mut p = VoiceParams::default();
        let mut voice = Voice::new(44_118.0);
        voice.note_on(36, true, false);
        voice.render_add(&p, &mut [0.0]);
        let gamma = voice.ladder.gamma;
        assert_eq!(voice.until_cutoff, 3);
        p.resonance = 0.93;
        voice.render_add(&p, &mut [0.0]);
        assert_eq!(voice.ladder.gamma, gamma);
        let k = p.max_loop_gain * p.resonance;
        assert_eq!(voice.ladder.feedback_recip, 1.0 / (1.0 + k * gamma));
        voice.render_add(&p, &mut [0.0; 4]);
        assert_eq!(
            voice.ladder.feedback_recip,
            1.0 / (1.0 + k * voice.ladder.gamma)
        );
    }

    #[test]
    fn silent_voice_suspends_the_ladder_but_discharges_accent() {
        let p = VoiceParams::default();
        let mut voice = Voice::new(44_100.0);
        let mut block = [0.0; 128];
        assert!(voice.skip_silence(&p, block.len()));
        assert_eq!(voice.phase, 0.0);
        voice.note_on(36, true, false);
        assert!(!voice.skip_silence(&p, block.len()));
        voice.render_add(&p, &mut block);
        voice.note_off();
        assert!(!voice.skip_silence(&p, block.len()));
        for _ in 0..40 {
            voice.render_add(&p, &mut block);
        }
        let phase = voice.phase;
        let before = voice.envelopes.advance(&p, 0).accent_octaves;
        assert!(before > 0.01);
        assert!(voice.skip_silence(&p, 44_100));
        assert_eq!(voice.phase, phase);
        assert!(voice.envelopes.advance(&p, 0).accent_octaves < before / 2.0);
        voice.note_on(48, false, false);
        assert!(!voice.skip_silence(&p, block.len()));
        block.fill(0.0);
        voice.render_add(&p, &mut block);
        assert!(block.iter().any(|v| v.abs() > 0.001));
    }
}
