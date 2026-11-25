#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod patch;

pub use patch::{Patch, Ratio, Shape, Time};
pub use wavegraph_adsr::Adsr;
use wavegraph_adsr::{Envelope, Timing};
use wavegraph_core::{Port, Process, ProcessContext, midi::Note, polyphony::Keyboard, sine_q15};
pub use wavegraph_core::{Tuning, polyphony::VoiceMode};
use wavegraph_sample::FromSample;

/// Maximum number of simultaneous notes, including release tails.
pub const POLYPHONY: usize = 4;

/// Frame-timed controls on inlet zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Strike a MIDI note. Zero velocity releases the note.
    NoteOn {
        /// MIDI note number.
        note: u8,
        /// Strike velocity, clamped to 127.
        velocity: u8,
    },
    /// Release a matching note.
    NoteOff(u8),
    /// Apply sound parameters to held and future notes.
    Patch(Patch),
    /// Change the allocation of the four voices and revoice held keys.
    VoiceMode(VoiceMode),
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    note: Note,
    phase: u32,
    step: u32,
    mod_phase: u32,
    mod_step: u32,
    previous_mod: i32,
    depth: f32,
    gain: f32,
    envelopes: [Envelope; 3], // carrier level, modulator level, common pitch
}

impl Voice {
    const EMPTY: Self = Self {
        note: Note {
            number: 0,
            velocity: 0,
        },
        phase: 0,
        step: 0,
        mod_phase: 0,
        mod_step: 0,
        previous_mod: 0,
        depth: 0.0,
        gain: 0.0,
        envelopes: [Envelope::IDLE; 3],
    };

    #[allow(
        clippy::cast_precision_loss,
        reason = "Phase increments need only relative precision for the index limit."
    )]
    fn tune_modulator(&mut self, patch: Patch, pitch_span: f32) {
        self.mod_step = patch.ratio.step(self.step);
        // Taper depth in the upper register as the first upper sideband approaches Nyquist.
        let headroom = if self.mod_step == 0 {
            self.previous_mod = 0;
            0.0
        } else {
            ((2_147_483_648.0 / (1.0 + pitch_span.max(0.0)) - self.step as f32)
                / self.mod_step as f32
                - 1.0)
                .max(0.0)
        };
        let touch = 0.2 + 0.8 * f32::from(self.note.velocity) / 127.0;
        let radians = f32::from(patch.index.min(127)) * (5.0 / 127.0) * touch;
        // Q15 modulator output times depth gives the phase offset in 16-bit turns.
        self.depth = radians.min(headroom) / core::f32::consts::PI;
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::arithmetic_side_effects,
        reason = "Q15 feedback times 32512 fits i32. Bent increments are nonnegative, and phase offsets wrap over one turn."
    )]
    fn sample(&mut self, timing: &[Timing; 3], pitch_span: f32, feedback: i32) -> f32 {
        let [carrier, modulator, pitch] = &mut self.envelopes;
        let level = carrier.advance(&timing[0]);
        let index = modulator.advance(&timing[1]) * self.depth;
        let bend = pitch.advance(&timing[2]) * pitch_span;
        // A common multiplier keeps the operator ratio intact throughout the bend.
        let (step, mod_step) = if bend == 0.0 {
            (self.step, self.mod_step)
        } else {
            let scale = 1.0 + bend;
            (
                (self.step as f32 * scale) as u32,
                (self.mod_step as f32 * scale) as u32,
            )
        };
        let modulation = sine_q15(
            self.mod_phase
                .wrapping_add_signed(self.previous_mod * feedback),
        );
        self.previous_mod = modulation;
        let offset = (modulation as f32 * index) as i32;
        let sample = sine_q15(self.phase.wrapping_add_signed(offset.wrapping_shl(16)));
        self.phase = self.phase.wrapping_add(step);
        self.mod_phase = self.mod_phase.wrapping_add(mod_step);
        if step < 1 << 31 {
            sample as f32 * self.gain * level
        } else {
            0.0
        }
    }

    fn release(&mut self, timing: &[Timing; 3]) {
        for (envelope, timing) in self.envelopes.iter_mut().zip(timing) {
            envelope.gate_off(timing);
        }
    }
}

/// Four voices, each with a sine carrier and a self-modulating sine modulator.
#[derive(Debug)]
pub struct Fm<'a> {
    tuning: &'a Tuning,
    patch: Patch,
    timing: [Timing; 3],
    pitch_span: f32,
    feedback: i32,
    keyboard: Keyboard,
    voices: [Voice; POLYPHONY],
}

impl<'a> Fm<'a> {
    /// Start silent with the Tine EP patch at the tuning table's sample rate.
    #[must_use]
    pub fn new(tuning: &'a Tuning) -> Self {
        let patch = Patch::TINE;
        Self {
            tuning,
            patch,
            timing: [patch.carrier, patch.modulator, patch.pitch]
                .map(|shape| shape.adsr().at_sample_period(tuning.sample_period())),
            pitch_span: patch::pitch_span(patch.pitch_cents),
            feedback: Self::feedback(patch),
            keyboard: Keyboard::EMPTY,
            voices: [Voice::EMPTY; POLYPHONY],
        }
    }

    fn feedback(patch: Patch) -> i32 {
        i32::from(patch.feedback.min(127)) << 8
    }

    fn set_patch(&mut self, patch: Patch) {
        for (i, (old, new)) in [
            (self.patch.carrier, patch.carrier),
            (self.patch.modulator, patch.modulator),
            (self.patch.pitch, patch.pitch),
        ]
        .into_iter()
        .enumerate()
        {
            if old != new {
                let timing = new.adsr().at_sample_period(self.tuning.sample_period());
                for v in &mut self.voices {
                    v.envelopes[i].retime(&self.timing[i], &timing);
                }
                self.timing[i] = timing;
            }
        }
        if (patch.ratio, patch.index, patch.pitch_cents)
            != (self.patch.ratio, self.patch.index, self.patch.pitch_cents)
        {
            self.pitch_span = patch::pitch_span(patch.pitch_cents);
            for v in self.voices.iter_mut().filter(|v| !v.envelopes[0].is_idle()) {
                v.tune_modulator(patch, self.pitch_span);
            }
        }
        self.feedback = Self::feedback(patch);
        self.patch = patch;
    }

    fn strike(&mut self, i: usize, note: Note, step: u32) {
        let v = &mut self.voices[i];
        *v = Voice {
            note,
            step,
            gain: f32::from(note.velocity) / (127.0 * 4.0 * 32768.0),
            ..Voice::EMPTY
        };
        v.tune_modulator(self.patch, self.pitch_span);
        for (envelope, timing) in v.envelopes.iter_mut().zip(&self.timing) {
            envelope.gate_on(timing);
        }
    }

    fn revoice(&mut self) {
        for i in 0..POLYPHONY {
            if let Some(note) = self.keyboard.voice(i) {
                let step = self.keyboard.step(i, self.tuning.phase_step(note.number));
                if step != 0 {
                    self.strike(i, note, step);
                    continue;
                }
            }
            self.voices[i].release(&self.timing);
        }
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::Patch(patch) => self.set_patch(patch),
            Message::VoiceMode(mode) => {
                if self.keyboard.mode != mode {
                    self.keyboard.mode = mode;
                    self.revoice();
                }
            }
            Message::NoteOff(note) | Message::NoteOn { note, velocity: 0 } => {
                let newest = self.keyboard.release(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    for v in &mut self.voices {
                        if v.note.number == note {
                            v.release(&self.timing);
                        }
                    }
                } else if newest {
                    self.revoice();
                }
            }
            Message::NoteOn { note, velocity } => {
                let step = self.tuning.phase_step(note);
                if step == 0 {
                    return;
                }
                let note = Note {
                    number: note,
                    velocity: velocity.min(127),
                };
                self.keyboard.press(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    let slot = self
                        .voices
                        .iter()
                        .position(|v| !v.envelopes[0].is_idle() && v.note.number == note.number)
                        .or_else(|| self.voices.iter().position(|v| v.envelopes[0].is_idle()))
                        .or_else(|| self.voices.iter().rposition(|v| !v.envelopes[0].is_gated()))
                        .unwrap_or(POLYPHONY - 1);
                    self.voices[..=slot].rotate_right(1);
                    self.strike(0, note, step);
                } else {
                    self.revoice();
                }
            }
        }
    }

    fn render<S: FromSample<f32>>(&mut self, output: &mut [S]) {
        if self.voices.iter().all(|v| v.envelopes[0].is_idle()) {
            output.fill_with(|| S::from_sample_(0.0));
            return;
        }
        for sample in output {
            let mut sum = 0.0;
            for v in &mut self.voices {
                if !v.envelopes[0].is_idle() {
                    sum += v.sample(&self.timing, self.pitch_span, self.feedback);
                }
            }
            *sample = S::from_sample_(sum);
        }
    }
}

impl<S: FromSample<f32>> Process<Message, S> for Fm<'_> {
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
