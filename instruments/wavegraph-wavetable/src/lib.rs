#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod bank;
mod scan;

pub use bank::{BANKS, Bank};
pub use scan::{Mode, Scan};
pub use wavegraph_adsr::Adsr;
pub use wavegraph_core::Tuning;
pub use wavegraph_core::polyphony::{Chord, VoiceMode};
use wavegraph_core::{midi::Note, polyphony::Keyboard};

use bank::Level;
use scan::{Cursor, Travel};
use wavegraph_adsr::{Envelope, Timing};
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::FromSample;

/// Simultaneous voices, each with an independent oscillator and scan clock.
pub const POLYPHONY: usize = 4;

/// Amplitude contour shared by the two factory sounds.
pub const DEFAULT_ENVELOPE: Adsr = Adsr {
    attack_us: 12_000,
    decay_us: 125_000,
    sustain_percent: 85,
    release_us: 160_000,
};

/// Factory frame sweep in envelope mode. A held note settles halfway back.
pub const DEFAULT_POSITION_ENVELOPE: Adsr = Adsr {
    attack_us: 80_000,
    decay_us: 320_000,
    sustain_percent: 50,
    release_us: 125_000,
};

/// Sample-timed controls accepted on inlet zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Change voice allocation, immediately revoicing any held keys.
    VoiceMode(VoiceMode),
    /// Strike a MIDI note. Velocity zero releases it.
    NoteOn {
        /// MIDI note number in 0–127.
        note: u8,
        /// Strike velocity, clamped to 127.
        velocity: u8,
    },
    /// Release the matching pitch.
    NoteOff(u8),
    /// Select a bank from the supplied catalog. Unknown indices are ignored.
    Bank(u8),
    /// Change the frame traversal of every voice. Editing the time preserves progress.
    Scan(Scan),
    /// Change the amplitude envelope, including on sounding voices.
    Envelope(Adsr),
    /// Shape the frame position in [`Mode::Envelope`], including on sounding voices.
    PositionEnvelope(Adsr),
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    note: u8,
    phase: u32,
    step: u32,
    level: Level,
    gain: f32,
    cursor: Cursor,
    envelope: Envelope,
    position_envelope: Envelope,
}

impl Voice {
    fn idle() -> Self {
        Self {
            note: 0,
            phase: 0,
            step: 0,
            level: Level::for_step(0),
            gain: 0.0,
            cursor: Cursor::default(),
            envelope: Envelope::IDLE,
            position_envelope: Envelope::IDLE,
        }
    }
}

/// Four voices sharing read-only banks and tuning. The most recent strike
/// takes priority. Reuse a matching note, then an idle slot, then the oldest
/// release tail, then the oldest held note.
#[derive(Debug)]
pub struct Wavetable<'a> {
    tuning: &'a Tuning,
    banks: &'a [Bank<'a>],
    bank: Bank<'a>,
    scan: Scan,
    travel: Travel,
    timing: Timing,
    position_timing: Timing,
    keyboard: Keyboard,
    voices: [Voice; POLYPHONY],
}

impl<'a> Wavetable<'a> {
    /// Create a silent instrument on the first bank, using the resonant scan
    /// settings. Returns `None` for an empty catalog or more than 256 banks.
    #[must_use]
    pub fn new(tuning: &'a Tuning, banks: &'a [Bank<'a>]) -> Option<Self> {
        let &bank = banks.first()?;
        if banks.len() > 256 {
            return None;
        }
        Some(Self {
            tuning,
            banks,
            bank,
            scan: Scan::RESONANT,
            travel: Travel::new(Scan::RESONANT, bank.last, tuning.sample_period()),
            timing: DEFAULT_ENVELOPE.at_sample_period(tuning.sample_period()),
            position_timing: DEFAULT_POSITION_ENVELOPE.at_sample_period(tuning.sample_period()),
            keyboard: Keyboard::EMPTY,
            voices: [Voice::idle(); POLYPHONY],
        })
    }

    /// Current frame of the newest sounding voice, or the manual/start frame
    /// when idle. Fractional positions report their lower neighbor.
    #[must_use]
    pub fn frame(&self) -> u8 {
        let (cursor, level) = self
            .voices
            .iter()
            .find(|v| !v.envelope.is_idle())
            .map_or((Cursor::default(), 0.0), |v| {
                (v.cursor, v.position_envelope.level())
            });
        self.travel.position(cursor, level).to_be_bytes()[0]
    }

    fn scan(&mut self, scan: Scan) {
        let travel = Travel::new(scan, self.bank.last, self.tuning.sample_period());
        for voice in &mut self.voices {
            travel.retime(&mut voice.cursor, self.travel);
            if scan.mode == Mode::Envelope && self.scan.mode != Mode::Envelope {
                voice.position_envelope = Envelope::IDLE;
                if voice.envelope.is_gated() {
                    voice.position_envelope.gate_on(&self.position_timing);
                }
            }
        }
        self.scan = scan;
        self.travel = travel;
    }

    fn release_slot(&mut self, i: usize) {
        let v = &mut self.voices[i];
        v.envelope.gate_off(&self.timing);
        v.position_envelope.gate_off(&self.position_timing);
    }

    fn strike(&mut self, i: usize, note: Note, step: u32) {
        self.voices[i] = Voice {
            note: note.number,
            phase: 0,
            step,
            level: Level::for_step(step),
            gain: f32::from(note.velocity.min(127)) / (127.0 * 4.0 * 32768.0),
            cursor: Cursor::default(),
            envelope: Envelope::IDLE,
            position_envelope: Envelope::IDLE,
        };
        self.voices[i].envelope.gate_on(&self.timing);
        if self.scan.mode == Mode::Envelope {
            self.voices[i]
                .position_envelope
                .gate_on(&self.position_timing);
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
            if self.voices[i].envelope.is_gated() {
                self.release_slot(i);
            }
        }
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::Scan(scan) => self.scan(scan),
            Message::Bank(index) => {
                if let Some(&bank) = self.banks.get(usize::from(index)) {
                    self.bank = bank;
                    self.scan(self.scan);
                }
            }
            Message::Envelope(adsr) => {
                let timing = adsr.at_sample_period(self.tuning.sample_period());
                for voice in &mut self.voices {
                    voice.envelope.retime(&self.timing, &timing);
                }
                self.timing = timing;
            }
            Message::PositionEnvelope(adsr) => {
                let timing = adsr.at_sample_period(self.tuning.sample_period());
                for voice in &mut self.voices {
                    voice
                        .position_envelope
                        .retime(&self.position_timing, &timing);
                }
                self.position_timing = timing;
            }

            Message::VoiceMode(mode) => {
                if self.keyboard.mode != mode {
                    self.keyboard.mode = mode;
                    self.revoice();
                }
            }
            Message::NoteOff(note) | Message::NoteOn { note, velocity: 0 } => {
                let newest = self.keyboard.release(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    for i in 0..POLYPHONY {
                        if self.voices[i].note == note && self.voices[i].envelope.is_gated() {
                            self.release_slot(i);
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
                    velocity,
                };
                self.keyboard.press(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    let index = self
                        .voices
                        .iter()
                        .position(|v| !v.envelope.is_idle() && v.note == note.number)
                        .or_else(|| self.voices.iter().position(|v| v.envelope.is_idle()))
                        .or_else(|| self.voices.iter().rposition(|v| !v.envelope.is_gated()))
                        .unwrap_or(POLYPHONY - 1);
                    self.voices[..=index].rotate_right(1);
                    self.strike(0, note, step);
                } else {
                    self.revoice();
                }
            }
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "Interpolated PCM16 fits exactly in f32."
    )]
    fn render<S: FromSample<f32>>(&mut self, output: &mut [S]) {
        if self.voices.iter().all(|v| v.envelope.is_idle()) {
            output.fill_with(|| S::from_sample_(0.0));
            return;
        }
        for sample in output {
            let mut sum = 0.0;
            for v in &mut self.voices {
                if !v.envelope.is_idle() {
                    let level = if self.scan.mode == Mode::Envelope {
                        v.position_envelope.advance(&self.position_timing)
                    } else {
                        0.0
                    };
                    let position = self.travel.position(v.cursor, level);
                    let value = self.bank.sample(v.level, v.phase, position);
                    sum += value as f32 * v.gain * v.envelope.advance(&self.timing);
                    v.phase = v.phase.wrapping_add(v.step);
                    self.travel.advance(&mut v.cursor);
                }
            }
            *sample = S::from_sample_(sum);
        }
    }
}

impl<S: FromSample<f32>> Process<Message, S> for Wavetable<'_> {
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
