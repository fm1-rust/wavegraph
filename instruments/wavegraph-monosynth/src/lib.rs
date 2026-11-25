#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod filter;

pub use wavegraph_adsr::Adsr;
pub use wavegraph_core::Tuning;

use filter::LowPass;
use wavegraph_adsr::{Envelope, Timing};
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
        reason = "The 16-bit ramp and triangle intermediates fit in i32."
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
            Self::Sine => wavegraph_core::sine_q15(phase),
        }
    }
}

/// Control messages for the voice's single inlet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Select an oscillator shape while retaining its phase.
    Waveform(Waveform),
    /// Set the four-pole low-pass cutoff in Hz. Zero selects 1 Hz.
    /// Values at or above Nyquist open the filter fully.
    CutoffHz(u16),
    /// Set the post-filter amplitude envelope, including during a note or its release.
    VcaEnvelope(Adsr),
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

/// A phase-continuous oscillator, four-pole low-pass filter and VCA envelope
/// with a message inlet and mono signal outlet. Starts silent with a sine wave,
/// a fully open filter and [`Adsr::GATE`]. Cloning copies the complete voice state.
#[derive(Clone, Debug)]
pub struct Monosynth<'a> {
    tuning: &'a Tuning,
    waveform: Waveform,
    filter: LowPass,
    envelope: Envelope,
    timing: Timing,
    note: u8,
    velocity: u8,
    suspend_idle: bool,
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
            filter: LowPass::OPEN,
            envelope: Envelope::IDLE,
            timing: Adsr::GATE.at_sample_period(tuning.sample_period()),
            note: 69,
            velocity: 0,
            suspend_idle: false,
            phase: 0,
            step: tuning.phase_step(69),
        }
    }

    /// Skip oscillator and filter rendering while the VCA is closed.
    /// The free-running oscillator phase still advances between notes.
    #[must_use]
    pub const fn with_idle_suspension(mut self) -> Self {
        self.suspend_idle = true;
        self
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::Waveform(shape) => self.waveform = shape,
            Message::CutoffHz(hz) => self.filter.set_cutoff(hz, self.tuning.sample_period()),
            Message::VcaEnvelope(adsr) => {
                let timing = adsr.at_sample_period(self.tuning.sample_period());
                self.envelope.retime(&self.timing, &timing);
                self.timing = timing;
            }
            Message::NoteOff(note) | Message::NoteOn { note, velocity: 0 } => {
                if self.note == note {
                    self.envelope.gate_off(&self.timing);
                }
            }
            Message::NoteOn { note, velocity } => {
                if note < 128 {
                    let step = self.tuning.phase_step(note);
                    self.note = note;
                    self.step = step;
                    self.envelope.gate_on(&self.timing);
                    self.velocity = if step == 0 { 0 } else { velocity.min(127) };
                }
            }
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "The velocity product fits in i32. The scaled Q23 level fits exactly in f32 and envelope gain is in 0..=1."
    )]
    fn render<S: FromSample<i32>>(&mut self, output: &mut [S]) {
        if self.suspend_idle && self.envelope.is_idle() {
            // Only the low 32 bits of elapsed frames affect the wrapping oscillator.
            self.phase = self
                .phase
                .wrapping_add(self.step.wrapping_mul(output.len() as u32));
            self.filter.skip_silent(output.len());
            output.fill_with(|| S::from_sample_(0));
            return;
        }
        for sample in output {
            // Preserve the FM-1 demo's Q23 level (about -24 dBFS at full velocity).
            let oscillator = self.waveform.sample(self.phase) << 8;
            let filtered = self.filter.process(oscillator);
            let q23 = (filtered * i32::from(self.velocity)) >> 11;
            *sample =
                S::from_sample_(((q23 as f32 * self.envelope.advance(&self.timing)) as i32) << 8);
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
