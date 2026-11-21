#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::FromSample;

/// The four oscillator shapes from the FM-1 waveform demo.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Waveform {
    /// Quarter-wave lookup table with linear interpolation.
    #[default]
    Sine,
    /// Linear rising and falling ramps.
    Triangle,
    /// A rising ramp.
    Sawtooth,
    /// A square wave with a 50% duty cycle.
    Square,
}

impl Waveform {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The ramp is 16 bits and interpolation uses an 8-bit fraction; intermediates fit in i32."
    )]
    fn sample(self, phase: u32) -> i32 {
        let ramp = i32::from((phase >> 16) as u16);
        match self {
            Self::Sawtooth => ramp - 32768,
            Self::Square => {
                if phase < 1 << 31 {
                    32767
                } else {
                    -32767
                }
            }
            Self::Triangle => 32767 - (2 * ramp - 65535).abs(),
            Self::Sine => {
                let part = phase & 0x3fff_ffff;
                let folded = if phase & (1 << 30) == 0 {
                    part
                } else {
                    (1 << 30) - part
                };
                let index = (folded >> 24) as usize;
                let a = i32::from(SINE[index]);
                let b = i32::from(SINE[(index + 1).min(64)]);
                let fraction = i32::from(((folded >> 16) & 255) as u8);
                let value = a + (((b - a) * fraction) >> 8);
                if phase & (1 << 31) == 0 {
                    value
                } else {
                    -value
                }
            }
        }
    }
}

// One quarter of a sine, round(32767 * sin(i * π / 128)).
const SINE: [i16; 65] = [
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602, 6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530, 18204, 18868, 19519, 20159, 20787,
    21403, 22005, 22594, 23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790, 27245, 27683,
    28105, 28510, 28898, 29268, 29621, 29956, 30273, 30571, 30852, 31113, 31356, 31580, 31785,
    31971, 32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757, 32767,
];

/// Equal-tempered phase increments for a device's sample clock. One table
/// can be shared by all voices running at the same rate.
#[derive(Debug)]
pub struct Tuning([u32; 128]);

impl Tuning {
    /// Construct a table for `numerator / denominator` frames per second,
    /// with MIDI note 69 at 440 Hz. Returns `None` for a zero clock component.
    /// Pitches at or above Nyquist are silent.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The rounded phase increment is positive and at most 2^31."
    )]
    pub const fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        let mut steps = [0; 128];
        // MIDI note 0, in cycles of a 32-bit phase accumulator per frame.
        let mut step =
            8.175_798_915_643_707 * 4_294_967_296.0 * denominator as f64 / numerator as f64;
        let mut note: usize = 0;
        while note < steps.len() {
            if step < 2_147_483_648.0 {
                steps[note] = (step + 0.5) as u32;
            }
            step *= 1.059_463_094_359_295_3;
            note = note.wrapping_add(1);
        }
        Some(Self(steps))
    }
}

/// Control messages for the voice's single inlet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Select an oscillator shape while retaining its phase.
    Waveform(Waveform),
    /// Play the given MIDI note. Velocity is clamped to 127. Zero releases
    /// the matching note. Note numbers outside 0..=127 are ignored.
    NoteOn {
        /// MIDI note number.
        note: u8,
        /// MIDI strike velocity.
        velocity: u8,
    },
    /// Release the matching note. A release for an older note is ignored.
    NoteOff(u8),
}

/// One phase-continuous oscillator with a message inlet and a mono signal
/// outlet. The initial waveform is a sine and the voice starts silent.
#[derive(Debug)]
pub struct Monosynth<'a> {
    tuning: &'a Tuning,
    waveform: Waveform,
    note: u8,
    velocity: u8,
    phase: u32,
    step: u32,
}

impl<'a> Monosynth<'a> {
    /// Create a voice using a precomputed tuning table.
    #[must_use]
    pub const fn new(tuning: &'a Tuning) -> Self {
        Self {
            tuning,
            waveform: Waveform::Sine,
            note: 69,
            velocity: 0,
            phase: 0,
            step: tuning.0[69],
        }
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::Waveform(shape) => self.waveform = shape,
            Message::NoteOff(note) | Message::NoteOn { note, velocity: 0 } => {
                if self.note == note {
                    self.velocity = 0;
                }
            }
            Message::NoteOn { note, velocity } => {
                if let Some(&step) = self.tuning.0.get(usize::from(note)) {
                    self.note = note;
                    self.step = step;
                    self.velocity = if step == 0 { 0 } else { velocity.min(127) };
                }
            }
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A Q15 waveform times a 7-bit velocity fits in i32."
    )]
    fn render<S: FromSample<i32>>(&mut self, output: &mut [S]) {
        for sample in output {
            // Preserve the FM-1 demo's Q23 level (about -24 dBFS at full velocity).
            let q23 = (self.waveform.sample(self.phase) * i32::from(self.velocity)) >> 3;
            *sample = S::from_sample_(q23 << 8);
            self.phase = self.phase.wrapping_add(self.step);
        }
    }
}

impl<S: FromSample<i32>> Process<Message, S> for Monosynth<'_> {
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
