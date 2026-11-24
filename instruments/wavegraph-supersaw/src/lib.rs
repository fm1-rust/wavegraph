#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod decimator;
mod filter;

pub use wavegraph_adsr::Adsr;
pub use wavegraph_core::Tuning;

use core::f32::consts::PI;
use decimator::Decimator;
use filter::{FilterParams, LowPass};
use micromath::F32;
use wavegraph_adsr::{Envelope, Timing};
pub use wavegraph_core::polyphony::VoiceMode;
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_core::{midi::Note, polyphony::Keyboard};
use wavegraph_sample::FromSample;

/// Initial detune on the 0–127 panel scale.
pub const DEFAULT_DETUNE: u8 = 80;
/// Initial mix on the 0–127 panel scale.
pub const DEFAULT_MIX: u8 = 96;
/// Initial low-pass cutoff at the foot of the filter envelope.
pub const DEFAULT_CUTOFF_HZ: u16 = 5_000;
/// The original short gate, with a velocity-sensitive peak.
pub const DEFAULT_AMP_ENVELOPE: Adsr = Adsr {
    attack_us: 2_000,
    decay_us: 160_000,
    sustain_percent: 100,
    release_us: 2_000,
};
/// Open the filter on the strike and settle to a brighter held tone.
pub const DEFAULT_FILTER_ENVELOPE: Adsr = Adsr {
    attack_us: 8_000,
    decay_us: 250_000,
    sustain_percent: 35,
    release_us: 125_000,
};

/// Notes and panel controls for the instrument's message inlet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Change the keyboard mode, revoicing held notes immediately.
    VoiceMode(VoiceMode),
    /// Play a MIDI note. A zero velocity releases the matching note.
    NoteOn {
        /// Note number. Values above 127 are ignored.
        note: u8,
        /// Strike velocity, clamped to 127.
        velocity: u8,
    },
    /// Release this note if it still owns the voice.
    NoteOff(u8),
    /// Set the pitch spread of the six side saws, clamped to 0–127.
    Detune(u8),
    /// Set the level of the six side saws, clamped to 0–127.
    Mix(u8),
    /// Set the base low-pass cutoff in Hz. Zero selects 1 Hz.
    CutoffHz(u16),
    /// Set the low-pass resonance, clamped to 0–127.
    Resonance(u8),
    /// Edit the amplitude contour, including during a note or release.
    AmpEnvelope(Adsr),
    /// Edit the contour that sweeps cutoff up to two octaves above its base.
    FilterEnvelope(Adsr),
}

// Side offsets from the oscillator DSP, in units of 1/128 of the detune base.
const SPREAD: [i32; 6] = [128, -128, 408, -412, 704, -720];

// Measured MIDI-to-detune curve, in units of 1/16384 of the note frequency.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "The MIDI segments produce integers in 1..=321."
)]
fn detune_amount(value: u8) -> i32 {
    let value = i32::from(value.min(127));
    match value {
        0..=63 => 1 + value / 2,
        64..=80 => value - 31,
        81..=120 => 49 + 2 * (value - 80),
        121..=123 => 129 + 8 * (value - 120),
        124 => 169,
        125 => 193,
        126 => 225,
        _ => 321,
    }
}

fn side_gain(mix: u8) -> i32 {
    i32::from(mix.min(127).saturating_add(6) / 4)
}

#[derive(Clone, Copy, Debug)]
struct Saw {
    phase: u32,
    step: u32,
}

impl Saw {
    #[allow(
        clippy::cast_possible_wrap,
        reason = "Reinterpret the wrapping phase as a bipolar ramp."
    )]
    fn tick(&mut self) -> i32 {
        self.phase = self.phase.wrapping_add(self.step);
        (self.phase as i32) >> 8
    }
}

/// Four independent seven-saw voices sharing a tuning table and panel controls.
#[derive(Clone, Debug)]
pub struct Supersaw<'a> {
    tuning: &'a Tuning,
    keyboard: Keyboard,
    voices: [Voice; 4],
    // Most recent strike first. Moving indices preserves the large FIR histories in place.
    order: [usize; 4],
    params: Params,
}

#[derive(Clone, Debug)]
struct Params {
    detune: i32,
    side_gain: i32,
    filter: FilterParams,
    filter_timing: Timing,
    amp_timing: Timing,
    velocity_slew: f32,
}

#[derive(Clone, Debug)]
struct Voice {
    saws: [Saw; 7],
    note: u8,
    hp_coeff: f32,
    hp_state: f32,
    decimator: Decimator,
    filter: LowPass,
    filter_env: Envelope,
    amp_env: Envelope,
    velocity: f32,
    target_velocity: f32,
}

impl Voice {
    fn new(step: u32, detune: i32) -> Self {
        // Stagger the initial phases once. Subsequent notes and knob turns keep them.
        let phases = [
            0,
            0x1d83_0000,
            0xb710_0000,
            0x4839_0000,
            0x95c1_0000,
            0xf2ae_0000,
            0x68d4_0000,
        ];
        let mut voice = Self {
            saws: phases.map(|phase| Saw { phase, step: 0 }),
            note: 69,
            hp_coeff: 0.0,
            hp_state: 0.0,
            decimator: Decimator::new(),
            filter: LowPass::default(),
            filter_env: Envelope::IDLE,
            amp_env: Envelope::IDLE,
            velocity: 0.0,
            target_velocity: 0.0,
        };
        voice.set_pitch(step, detune);
        voice
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "The filter coefficient only needs f32 precision."
    )]
    fn set_pitch(&mut self, output_step: u32, detune: i32) {
        let step = output_step / 2;
        self.saws[0].step = step;
        self.retune_sides(detune);
        // Bilinear one-pole high-pass at the fundamental, running at the internal rate.
        let g = F32(step as f32 * (PI / 4_294_967_296.0)).tan().0;
        self.hp_coeff = g / (1.0 + g);
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The signed 64-bit product fits. All resulting steps are within 12% of the center step, which is at most 2^30."
    )]
    fn retune_sides(&mut self, detune: i32) {
        let center = i64::from(self.saws[0].step);
        // The detune control and oscillator weights have 14 and 7 fractional bits.
        let detuned = center * i64::from(detune);
        for (saw, weight) in self.saws[1..].iter_mut().zip(SPREAD) {
            saw.step = (center + ((detuned * i64::from(weight)) >> 21)) as u32;
        }
    }

    fn release(&mut self, p: &Params) {
        self.amp_env.gate_off(&p.amp_timing);
        self.filter_env.gate_off(&p.filter_timing);
    }

    fn strike(&mut self, note: Note, step: u32, p: &Params) {
        self.note = note.number;
        self.set_pitch(step, p.detune);
        self.target_velocity = f32::from(note.velocity.min(127)) / 127.0;
        if self.amp_env.is_idle() {
            self.hp_state = 0.0;
            self.decimator = Decimator::new();
            self.filter.reset(&p.filter);
            self.velocity = self.target_velocity;
        } else {
            self.release(p);
        }
        self.amp_env.gate_on(&p.amp_timing);
        self.filter_env.gate_on(&p.filter_timing);
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_precision_loss,
        reason = "Seven signed 24-bit ramps weighted by at most 25 + 6*33 fit in i32. Only the mixed signal is rounded to f32."
    )]
    fn tick(&mut self, side_gain: i32) -> f32 {
        let [center, sides @ ..] = &mut self.saws;
        let sides: i32 = sides.iter_mut().map(Saw::tick).sum();
        let mixed = center.tick() * 25 + sides * side_gain;
        // Fixed headroom keeps the center gain independent of MIX.
        let input = mixed as f32 * (1.0 / 4_294_967_296.0);
        let delta = input - self.hp_state;
        let low = delta * self.hp_coeff;
        self.hp_state += low + low;
        delta - low
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "Phase accumulation wraps modulo 2^32, including when skipping silent frames."
    )]
    fn render_add(&mut self, output: &mut [f32], p: &Params) {
        if self.amp_env.is_idle() {
            let ticks = (output.len() as u32).wrapping_mul(2);
            for saw in &mut self.saws {
                saw.phase = saw.phase.wrapping_add(saw.step.wrapping_mul(ticks));
            }
            // A short VCA release can leave the VCF releasing into the next strike.
            for _ in 0..output.len() {
                if self.filter_env.is_idle() {
                    break;
                }
                self.filter_env.advance(&p.filter_timing);
            }
            return;
        }
        for out in output {
            let pair = [self.tick(p.side_gain), self.tick(p.side_gain)];
            let sample = self.decimator.process(pair);
            let cutoff = self.filter_env.advance(&p.filter_timing);
            let filtered = self.filter.process(sample, cutoff, &p.filter);
            self.velocity +=
                (self.target_velocity - self.velocity).clamp(-p.velocity_slew, p.velocity_slew);
            let gain = self.velocity * self.amp_env.advance(&p.amp_timing);
            *out += filtered * gain;
        }
    }
}

impl<'a> Supersaw<'a> {
    /// Create four silent voices at the tuning table's output rate.
    #[must_use]
    pub fn new(tuning: &'a Tuning) -> Self {
        let period = tuning.sample_period();
        let detune = detune_amount(DEFAULT_DETUNE);
        let voice = Voice::new(tuning.phase_step(69), detune);
        Self {
            tuning,
            keyboard: Keyboard::EMPTY,
            voices: core::array::from_fn(|_| voice.clone()),
            order: [0, 1, 2, 3],
            params: Params {
                detune,
                side_gain: side_gain(DEFAULT_MIX),
                filter: FilterParams::new(period),
                filter_timing: DEFAULT_FILTER_ENVELOPE.at_sample_period(period),
                amp_timing: DEFAULT_AMP_ENVELOPE.at_sample_period(period),
                velocity_slew: 500.0 * period,
            },
        }
    }

    fn revoice(&mut self) {
        self.order = [0, 1, 2, 3];
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if let Some(note) = self.keyboard.voice(i) {
                let step = self.keyboard.step(i, self.tuning.phase_step(note.number));
                if step > 1 {
                    voice.strike(note, step, &self.params);
                    continue;
                }
            }
            voice.release(&self.params);
        }
    }

    fn apply(&mut self, message: Message) {
        let period = self.tuning.sample_period();
        let p = &mut self.params;
        match message {
            Message::Detune(value) => {
                p.detune = detune_amount(value);
                for voice in &mut self.voices {
                    voice.retune_sides(p.detune);
                }
            }
            Message::Mix(value) => p.side_gain = side_gain(value),
            Message::CutoffHz(hz) => p.filter.set_cutoff(hz, period),
            Message::Resonance(value) => p.filter.set_resonance(value),
            Message::AmpEnvelope(adsr) => {
                let next = adsr.at_sample_period(period);
                for v in &mut self.voices {
                    v.amp_env.retime(&p.amp_timing, &next);
                }
                p.amp_timing = next;
            }
            Message::FilterEnvelope(adsr) => {
                let next = adsr.at_sample_period(period);
                for v in &mut self.voices {
                    v.filter_env.retime(&p.filter_timing, &next);
                }
                p.filter_timing = next;
            }
            Message::VoiceMode(mode) => {
                if self.keyboard.mode != mode {
                    self.keyboard.mode = mode;
                    self.revoice();
                }
            }
            Message::NoteOn { note, velocity: 0 } | Message::NoteOff(note) => {
                let newest = self.keyboard.release(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    for v in &mut self.voices {
                        if v.note == note {
                            v.release(p);
                        }
                    }
                } else if newest {
                    self.revoice();
                }
            }
            Message::NoteOn { note, velocity } => {
                let step = self.tuning.phase_step(note);
                if step <= 1 {
                    return;
                }
                let key = Note {
                    number: note,
                    velocity,
                };
                self.keyboard.press(key);
                if self.keyboard.mode != VoiceMode::Poly {
                    self.revoice();
                    return;
                }
                let rank = self
                    .order
                    .iter()
                    .position(|&i| self.voices[i].amp_env.is_gated() && self.voices[i].note == note)
                    .or_else(|| {
                        self.order
                            .iter()
                            .position(|&i| self.voices[i].amp_env.is_idle())
                    })
                    .or_else(|| {
                        self.order
                            .iter()
                            .rposition(|&i| !self.voices[i].amp_env.is_gated())
                    })
                    .unwrap_or(3);
                self.order[..=rank].rotate_right(1);
                self.voices[self.order[0]].strike(key, step, p);
            }
        }
    }

    fn render<S: FromSample<f32>>(&mut self, output: &mut [S]) {
        for chunk in output.chunks_mut(32) {
            let mut mix = [0.0; 32];
            let mix = &mut mix[..chunk.len()];
            for voice in &mut self.voices {
                voice.render_add(mix, &self.params);
            }
            for (out, value) in chunk.iter_mut().zip(mix) {
                *out = S::from_sample_(value.clamp(-1.0, 1.0 - f32::EPSILON));
            }
        }
    }
}

impl<S: FromSample<f32>> Process<Message, S> for Supersaw<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, S>) {
        let output = &mut cx.outputs[0];
        let mut start = 0;
        for event in cx.events {
            self.render(&mut output[start..event.frame]);
            self.apply(event.message);
            start = event.frame;
        }
        self.render(&mut output[start..]);
    }
}
