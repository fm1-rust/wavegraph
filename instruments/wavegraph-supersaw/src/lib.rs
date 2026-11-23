#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod decimator;

pub use wavegraph_core::Tuning;

use core::f32::consts::PI;
use decimator::Decimator;
use micromath::F32;
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::FromSample;

/// Initial detune on the 0–127 panel scale.
pub const DEFAULT_DETUNE: u8 = 80;
/// Initial mix on the 0–127 panel scale.
pub const DEFAULT_MIX: u8 = 96;

/// Notes and panel controls for the instrument's message inlet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
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

/// Seven phase-continuous saws, a pitch-tracking high-pass filter and a mono
/// output. Instances can share one tuning table at the host's output rate.
#[derive(Clone, Debug)]
pub struct Supersaw<'a> {
    tuning: &'a Tuning,
    saws: [Saw; 7],
    note: u8,
    detune: i32,
    side_gain: i32,
    hp_coeff: f32,
    hp_state: f32,
    decimator: Decimator,
    level: f32,
    target_level: f32,
    slew: f32,
}

impl<'a> Supersaw<'a> {
    /// Create a silent voice clocked at twice the tuning table's output rate.
    #[must_use]
    pub fn new(tuning: &'a Tuning) -> Self {
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
            tuning,
            saws: phases.map(|phase| Saw { phase, step: 0 }),
            note: 69,
            detune: detune_amount(DEFAULT_DETUNE),
            side_gain: side_gain(DEFAULT_MIX),
            hp_coeff: 0.0,
            hp_state: 0.0,
            decimator: Decimator::new(),
            level: 0.0,
            target_level: 0.0,
            slew: 500.0 * tuning.sample_period(), // 2 ms to traverse full scale
        };
        voice.set_pitch(69);
        voice
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "The filter coefficient only needs f32 precision."
    )]
    fn set_pitch(&mut self, note: u8) {
        self.note = note;
        let step = self.tuning.phase_step(note) / 2;
        self.saws[0].step = step;
        self.retune_sides();
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
    fn retune_sides(&mut self) {
        let center = i64::from(self.saws[0].step);
        // The detune control and oscillator weights have 14 and 7 fractional bits.
        let detuned = center * i64::from(self.detune);
        for (saw, weight) in self.saws[1..].iter_mut().zip(SPREAD) {
            saw.step = (center + ((detuned * i64::from(weight)) >> 21)) as u32;
        }
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::Detune(value) => {
                self.detune = detune_amount(value);
                self.retune_sides();
            }
            Message::Mix(value) => self.side_gain = side_gain(value),
            Message::NoteOff(note) | Message::NoteOn { note, velocity: 0 } if note == self.note => {
                self.target_level = 0.0;
            }
            Message::NoteOn { note, velocity } if note < 128 && velocity != 0 => {
                self.set_pitch(note);
                if self.level == 0.0 {
                    self.hp_state = 0.0;
                    self.decimator = Decimator::new();
                }
                if self.saws[0].step == 0 {
                    self.level = 0.0;
                    self.target_level = 0.0;
                } else {
                    self.target_level = f32::from(velocity.min(127)) / 127.0;
                }
            }
            _ => {}
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_precision_loss,
        reason = "Seven signed 24-bit ramps weighted by at most 25 + 6*33 fit in i32. Only the mixed signal is rounded to f32."
    )]
    fn tick(&mut self) -> f32 {
        let [center, sides @ ..] = &mut self.saws;
        let sides: i32 = sides.iter_mut().map(Saw::tick).sum();
        let mixed = center.tick() * 25 + sides * self.side_gain;
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
    fn render<S: FromSample<f32>>(&mut self, output: &mut [S]) {
        if self.level == 0.0 && self.target_level == 0.0 {
            let ticks = (output.len() as u32).wrapping_mul(2);
            for saw in &mut self.saws {
                saw.phase = saw.phase.wrapping_add(saw.step.wrapping_mul(ticks));
            }
            output.fill_with(|| S::from_sample_(0.0));
            return;
        }
        for out in output {
            let pair = [self.tick(), self.tick()];
            let sample = self.decimator.process(pair);
            self.level += (self.target_level - self.level).clamp(-self.slew, self.slew);
            *out = S::from_sample_((sample * self.level).clamp(-1.0, 1.0 - f32::EPSILON));
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
