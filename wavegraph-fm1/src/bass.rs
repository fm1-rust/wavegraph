//! TB-303 panel values and their corresponding WaveGraph messages.

use crate::instrument::{EventBatch, Voice, at};
use wavegraph_303::{Message, VoiceParams, Waveform};

pub(super) mod envelope;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Bank {
    Filter,
    Voice,
}

impl Bank {
    pub(super) const fn knobs(self) -> [Knob; 4] {
        match self {
            Self::Filter => [Knob::Cutoff, Knob::Resonance, Knob::EnvMod, Knob::Decay],
            Self::Voice => [Knob::Tuning, Knob::Attack, Knob::Accent, Knob::Overdrive],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Knob {
    Cutoff,
    Resonance,
    EnvMod,
    Decay,
    Tuning,
    Attack,
    Accent,
    Overdrive,
}

impl Knob {
    pub(super) fn name(self) -> &'static [u8; 9] {
        match self {
            Self::Cutoff => b"CUTOFF   ",
            Self::Resonance => b"RESONANCE",
            Self::EnvMod => b"ENVMOD   ",
            Self::Decay => b"DECAY    ",
            Self::Tuning => b"TUNING   ",
            Self::Attack => b"ATTACK   ",
            Self::Accent => b"ACCENT   ",
            Self::Overdrive => b"OVERDRIVE",
        }
    }
}

// Sixth-octave detents, from 62.5 Hz to 8 kHz. The default is 500 Hz.
const CUTOFF_HZ: [u16; 43] = [
    63, 70, 79, 88, 99, 111, 125, 140, 157, 177, 198, 223, 250, 281, 315, 354, 397, 445, 500, 561,
    630, 707, 794, 891, 1000, 1122, 1260, 1414, 1587, 1782, 2000, 2245, 2520, 2828, 3175, 3564,
    4000, 4490, 5040, 5657, 6350, 7127, 8000,
];

// Finer steps around the stock 1.5 ms attack, then out to 60 ms.
const ATTACK_US: [u16; 18] = [
    0, 250, 500, 750, 1000, 1500, 2000, 3000, 4000, 6000, 8000, 10_000, 15_000, 20_000, 30_000,
    40_000, 50_000, 60_000,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    values: [u8; 8],
    pub(super) bank: Bank,
    last: [Knob; 2],
}

impl Controls {
    pub(super) const INITIAL: Self = Self {
        // 500 Hz, 50% resonance, 24 semitones, 600 ms, concert pitch, 1.5 ms, 50% accent, 0 dB.
        values: [18, 50, 24, 60, 120, 5, 50, 0],
        bank: Bank::Filter,
        last: [Knob::Cutoff, Knob::Tuning],
    };

    /// The factory sequence's resonant, driven voice, also available to live keys.
    pub(super) const DEEP_HOUSE: Self = Self {
        // 315 Hz, 82% resonance, 36 semitones, 330 ms, 1.5 ms, 85% accent, 18 dB.
        values: [14, 82, 36, 33, 120, 5, 85, 18],
        ..Self::INITIAL
    };

    /// Hand the three automated controls back to their retained panel settings.
    pub(super) fn automation_reset(self) -> [Message; 3] {
        let params = self.voice_params();
        [
            Message::CutoffHz(params.cutoff_hz),
            Message::Resonance(params.resonance),
            Message::OverdriveDb(params.overdrive_db),
        ]
    }

    pub(super) fn turn_select(&mut self, detents: i8) {
        if detents % 2 != 0 {
            self.bank = match self.bank {
                Bank::Filter => Bank::Voice,
                Bank::Voice => Bank::Filter,
            };
        }
    }

    pub(super) fn last(self) -> Knob {
        self.last[self.bank as usize]
    }

    pub(super) fn turn_knobs(&mut self, detents: [i8; 4]) {
        for (knob, delta) in self.bank.knobs().into_iter().zip(detents) {
            self.turn(knob, delta);
        }
    }

    fn turn(&mut self, knob: Knob, detents: i8) {
        if detents == 0 {
            return;
        }
        let (min, max) = match knob {
            Knob::Cutoff => (0, 42),
            Knob::Resonance | Knob::Accent => (0, 100),
            Knob::EnvMod => (0, 72),
            Knob::Decay => (5, 200),
            Knob::Tuning => (0, 240),
            Knob::Attack => (0, 17),
            Knob::Overdrive => (0, 36),
        };
        let value = &mut self.values[knob as usize];
        *value = value.saturating_add_signed(detents).clamp(min, max);
        self.last[self.bank as usize] = knob;
    }

    fn cutoff_hz(self) -> u16 {
        CUTOFF_HZ[usize::from(self.values[Knob::Cutoff as usize])]
    }

    fn attack_us(self) -> u16 {
        ATTACK_US[usize::from(self.values[Knob::Attack as usize])]
    }

    fn tuning_cents(self) -> i16 {
        let step = i16::from(self.values[Knob::Tuning as usize]);
        step.saturating_sub(120).saturating_mul(10)
    }

    pub(super) fn readout(self) -> (&'static [u8], i32, &'static [u8]) {
        let knob = self.last();
        let value = i32::from(self.values[knob as usize]);
        let (number, units): (_, &[u8; 5]) = match knob {
            Knob::Cutoff => (i32::from(self.cutoff_hz()), b"HZ   "),
            Knob::Resonance | Knob::Accent => (value, b"PCT  "),
            Knob::EnvMod => (value, b"ST   "),
            Knob::Decay => (value.saturating_mul(10), b"MS   "),
            Knob::Tuning => (i32::from(self.tuning_cents()), b"CENTS"),
            Knob::Attack => {
                let us = self.attack_us();
                if us.is_multiple_of(1000) {
                    (i32::from(us / 1000), b"MS   ")
                } else {
                    (i32::from(us), b"US   ")
                }
            }
            Knob::Overdrive => (value, b"DB   "),
        };
        (knob.name(), number, units)
    }

    fn voice_params(self) -> VoiceParams {
        let value = |knob: Knob| f32::from(self.values[knob as usize]);
        VoiceParams {
            cutoff_hz: f32::from(self.cutoff_hz()),
            resonance: value(Knob::Resonance) / 100.0,
            env_oct: value(Knob::EnvMod) / 12.0,
            decay_s: value(Knob::Decay) / 100.0,
            tuning_st: f32::from(self.tuning_cents()) / 100.0,
            attack_s: f32::from(self.attack_us()) / 1_000_000.0,
            accent: value(Knob::Accent) / 100.0,
            overdrive_db: value(Knob::Overdrive),
            ..VoiceParams::default()
        }
    }

    pub(super) fn events(self, previous: Self, voice: Voice, old_voice: Voice) -> EventBatch {
        let mut batch = EventBatch::new();
        // Edits to both banks can arrive between two audio blocks.
        if self.values != previous.values {
            let params = self.voice_params();
            for knob in Bank::Filter.knobs().into_iter().chain(Bank::Voice.knobs()) {
                let i = knob as usize;
                if self.values[i] == previous.values[i] {
                    continue;
                }
                let message = match knob {
                    Knob::Cutoff => Message::CutoffHz(params.cutoff_hz),
                    Knob::Resonance => Message::Resonance(params.resonance),
                    Knob::EnvMod => Message::EnvOctaves(params.env_oct),
                    Knob::Decay => Message::DecaySeconds(params.decay_s),
                    Knob::Tuning => Message::TuningSemitones(params.tuning_st),
                    Knob::Attack => Message::AttackSeconds(params.attack_s),
                    Knob::Accent => Message::Accent(params.accent),
                    Knob::Overdrive => Message::OverdriveDb(params.overdrive_db),
                };
                batch.push(at(message)).expect("eight panel controls");
            }
        }
        if voice.waveform != old_voice.waveform {
            let shape = if voice.waveform == 0 {
                Waveform::Sawtooth
            } else {
                Waveform::Square
            };
            batch
                .push(at(Message::Waveform(shape)))
                .expect("one waveform change");
        }
        let before = old_voice.sounding();
        let next = voice.sounding();
        if before != next || voice.gate.phrase != old_voice.gate.phrase {
            let message = match (before, next) {
                (_, Some(note)) => Message::NoteOn {
                    note: note.number,
                    accent: note.velocity >= 96,
                    slide: before.is_some() && voice.gate.phrase == old_voice.gate.phrase,
                },
                (Some(old), None) => Message::NoteOff(old.number),
                (None, None) => return batch,
            };
            batch.push(at(message)).expect("one note change");
        }
        batch
    }
}
