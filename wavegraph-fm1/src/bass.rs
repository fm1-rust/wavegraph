//! TB-303 panel values and their corresponding WaveGraph messages.

use crate::instrument::{EventBatch, Voice, at};
use wavegraph_303::{Message, Waveform};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Knob {
    #[default]
    Cutoff,
    Resonance,
    EnvMod,
    Decay,
}

impl Knob {
    pub(super) const ALL: [Self; 4] = [Self::Cutoff, Self::Resonance, Self::EnvMod, Self::Decay];
}

// Sixth-octave detents, from 62.5 Hz to 8 kHz. The default is 500 Hz.
const CUTOFF_HZ: [u16; 43] = [
    63, 70, 79, 88, 99, 111, 125, 140, 157, 177, 198, 223, 250, 281, 315, 354, 397, 445, 500, 561,
    630, 707, 794, 891, 1000, 1122, 1260, 1414, 1587, 1782, 2000, 2245, 2520, 2828, 3175, 3564,
    4000, 4490, 5040, 5657, 6350, 7127, 8000,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    // Cutoff index, resonance percent, envelope depth in semitones, decay in 10 ms.
    values: [u8; 4],
    last: Knob,
}

impl Controls {
    pub(super) const INITIAL: Self = Self {
        values: [18, 50, 24, 60],
        last: Knob::Cutoff,
    };

    pub(super) fn turn(&mut self, knob: Knob, detents: i8) {
        if detents == 0 {
            return;
        }
        let (min, max) = match knob {
            Knob::Cutoff => (0, 42),
            Knob::Resonance => (0, 100),
            Knob::EnvMod => (0, 72),
            Knob::Decay => (5, 200),
        };
        let value = &mut self.values[knob as usize];
        *value = value.saturating_add_signed(detents).clamp(min, max);
        self.last = knob;
    }

    fn cutoff_hz(self) -> u16 {
        CUTOFF_HZ[usize::from(self.values[Knob::Cutoff as usize])]
    }

    pub(super) fn readout(self) -> (&'static [u8], u16, &'static [u8]) {
        let value = u16::from(self.values[self.last as usize]);
        match self.last {
            Knob::Cutoff => (b"CUTOFF   ", self.cutoff_hz(), b"HZ "),
            Knob::Resonance => (b"RESONANCE", value, b"PCT"),
            Knob::EnvMod => (b"ENV MOD  ", value, b"ST "),
            Knob::Decay => (b"DECAY    ", value * 10, b"MS "),
        }
    }

    pub(super) fn events(self, previous: Self, voice: Voice, old_voice: Voice) -> EventBatch {
        let mut batch = EventBatch::new();
        for knob in Knob::ALL {
            let i = knob as usize;
            if self.values[i] == previous.values[i] {
                continue;
            }
            let value = f32::from(self.values[i]);
            let message = match knob {
                Knob::Cutoff => Message::CutoffHz(f32::from(self.cutoff_hz())),
                Knob::Resonance => Message::Resonance(value / 100.0),
                Knob::EnvMod => Message::EnvOctaves(value / 12.0),
                Knob::Decay => Message::DecaySeconds(value / 100.0),
            };
            batch.push(at(message)).expect("four panel controls");
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
