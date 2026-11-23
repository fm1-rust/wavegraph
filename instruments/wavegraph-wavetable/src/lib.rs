#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod bank;
mod scan;

pub use bank::{BANKS, Bank};
pub use scan::{Mode, Scan};
pub use wavegraph_adsr::Adsr;
pub use wavegraph_core::Tuning;

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

/// Sample-timed controls accepted on inlet zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
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
            voices: [Voice::idle(); POLYPHONY],
        })
    }

    /// Current frame of the newest sounding voice, or the manual/start frame
    /// when idle. Fractional positions report their lower neighbor.
    #[must_use]
    pub fn frame(&self) -> u8 {
        let cursor = self
            .voices
            .iter()
            .find(|v| !v.envelope.is_idle())
            .map_or(Cursor::default(), |v| v.cursor);
        self.travel.position(cursor).to_be_bytes()[0]
    }

    fn scan(&mut self, scan: Scan) {
        let travel = Travel::new(scan, self.bank.last, self.tuning.sample_period());
        for voice in &mut self.voices {
            travel.retime(&mut voice.cursor, self.travel);
        }
        self.scan = scan;
        self.travel = travel;
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
            Message::NoteOff(note) | Message::NoteOn { note, velocity: 0 } => {
                if let Some(v) = self
                    .voices
                    .iter_mut()
                    .find(|v| v.note == note && !v.envelope.is_idle())
                {
                    v.envelope.gate_off(&self.timing);
                }
            }
            Message::NoteOn { note, velocity } => {
                let step = self.tuning.phase_step(note);
                if step == 0 {
                    return;
                }
                let index = self
                    .voices
                    .iter()
                    .position(|v| !v.envelope.is_idle() && v.note == note)
                    .or_else(|| self.voices.iter().position(|v| v.envelope.is_idle()))
                    .or_else(|| self.voices.iter().rposition(|v| !v.envelope.is_gated()))
                    .unwrap_or(POLYPHONY - 1);
                self.voices[..=index].rotate_right(1);
                let mut envelope = Envelope::IDLE;
                envelope.gate_on(&self.timing);
                self.voices[0] = Voice {
                    note,
                    phase: 0,
                    step,
                    level: Level::for_step(step),
                    gain: f32::from(velocity.min(127)) / (127.0 * 4.0 * 32768.0),
                    cursor: Cursor::default(),
                    envelope,
                };
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
                    let value = self
                        .bank
                        .sample(v.level, v.phase, self.travel.position(v.cursor));
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
