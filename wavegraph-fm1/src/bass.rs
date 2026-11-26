//! TB-303 panel values and their corresponding WaveGraph messages.

use crate::instrument::{EventBatch, Voice, at};
use wavegraph_303::{Message, Muffler, VoiceParams, Waveform};

pub(super) mod envelope;
pub(super) mod presets;
pub(super) mod space;

/// A selected dial or switch on the 303 faceplate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Control {
    Knob(Knob),
    Muffler,
    Waveform,
}

impl Control {
    pub(super) const PANEL: [Self; 10] = [
        Self::Knob(Knob::Tuning),
        Self::Knob(Knob::Cutoff),
        Self::Knob(Knob::Resonance),
        Self::Knob(Knob::EnvMod),
        Self::Knob(Knob::Decay),
        Self::Knob(Knob::Accent),
        Self::Knob(Knob::Attack),
        Self::Knob(Knob::Overdrive),
        Self::Muffler,
        Self::Waveform,
    ];
}

/// A recall request retained in panel snapshots, including when a scan is dropped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PresetSelection {
    pub(super) index: u8,
    revision: u32,
}

impl PresetSelection {
    pub(super) const FIRST: Self = Self {
        index: 0,
        revision: 0,
    };
    pub(super) const SECOND: Self = Self {
        index: 1,
        revision: 0,
    };

    pub(super) fn turn(&mut self, detents: i8) {
        if detents != 0 {
            self.index = presets::turn(self.index, detents);
            // A whole bank turn must still recall the sound after live edits.
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

pub(super) const CC_SLIDE: u8 = 102;
pub(super) const CC_WAVE: u8 = 70;
/// In Knob order, followed by the waveform switch. Attack and waveform are WaveGraph extensions.
pub(super) const CONTROLLERS: [u8; 9] = [74, 71, 12, 75, 104, 73, 16, 17, CC_WAVE];
// TB-03 delay time and feedback are sent only to the external instrument.
const DELAY_CCS: [u8; 2] = [18, 19];
pub(super) const AUTOMATION_COUNT: usize = CONTROLLERS.len() + DELAY_CCS.len();

pub(super) fn automation_ccs() -> impl Iterator<Item = u8> {
    CONTROLLERS.into_iter().chain(DELAY_CCS)
}

/// MIDI cutoff follows the same seven octaves as the panel, at seven-bit resolution.
const MIDI_CUTOFF: [u16; 128] = [
    62, 65, 67, 70, 73, 76, 79, 82, 85, 88, 92, 95, 99, 103, 107, 111, 115, 120, 124, 129, 134,
    139, 145, 150, 156, 162, 169, 175, 182, 189, 197, 204, 212, 221, 229, 238, 247, 257, 267, 277,
    288, 299, 311, 323, 336, 349, 362, 376, 391, 406, 422, 439, 456, 473, 492, 511, 531, 552, 573,
    595, 619, 643, 668, 694, 721, 749, 778, 808, 840, 872, 906, 942, 978, 1017, 1056, 1097, 1140,
    1184, 1230, 1278, 1328, 1380, 1434, 1489, 1547, 1608, 1670, 1735, 1803, 1873, 1946, 2022, 2101,
    2183, 2267, 2356, 2448, 2543, 2642, 2745, 2852, 2963, 3078, 3198, 3323, 3452, 3586, 3726, 3871,
    4022, 4179, 4341, 4510, 4686, 4868, 5058, 5255, 5460, 5672, 5893, 6123, 6361, 6609, 6866, 7134,
    7411, 7700, 8000,
];

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
    /// The six original dials, then Attack and OD on the row below.
    pub(super) const PANEL: [Self; 8] = [
        Self::Tuning,
        Self::Cutoff,
        Self::Resonance,
        Self::EnvMod,
        Self::Decay,
        Self::Accent,
        Self::Attack,
        Self::Overdrive,
    ];

    /// The four middle dials are wired to the numbered encoders in both 303 views.
    pub(super) fn direct() -> &'static [Self] {
        &Self::PANEL[1..5]
    }

    pub(super) const fn limits(self) -> (u8, u8) {
        match self {
            Self::Cutoff => (0, 42),
            Self::Resonance | Self::Accent => (0, 100),
            Self::EnvMod => (0, 72),
            Self::Decay => (5, 200),
            Self::Tuning => (0, 240),
            Self::Attack => (0, 17),
            Self::Overdrive => (0, 36),
        }
    }

    pub(super) fn from_cc(cc: u8) -> Option<Self> {
        Some(match cc {
            74 => Self::Cutoff,
            71 => Self::Resonance,
            12 => Self::EnvMod,
            75 => Self::Decay,
            104 => Self::Tuning,
            73 => Self::Attack,
            16 => Self::Accent,
            17 => Self::Overdrive,
            _ => return None,
        })
    }

    pub(super) fn from_message(message: Message) -> Option<u8> {
        Some(match message {
            Message::CutoffHz(_) => 74,
            Message::Resonance(_) => 71,
            Message::EnvOctaves(_) => 12,
            Message::DecaySeconds(_) => 75,
            Message::TuningSemitones(_) => 104,
            Message::AttackSeconds(_) => 73,
            Message::Accent(_) => 16,
            Message::PedalDrive(_) => 17,
            Message::Waveform(_) => CC_WAVE,
            _ => return None,
        })
    }

    fn message(self, controls: &Controls) -> Message {
        let i = self as usize;
        if let Some(value) = controls.midi[i] {
            return midi_parameter(CONTROLLERS[i], value).expect("each panel knob has a MIDI CC");
        }
        let value = f32::from(controls.values[i]);
        match self {
            Self::Cutoff => Message::CutoffHz(f32::from(controls.cutoff_hz())),
            Self::Resonance => Message::Resonance(value / 100.0),
            Self::EnvMod => Message::EnvOctaves(value / 12.0),
            Self::Decay => Message::DecaySeconds(value / 100.0),
            Self::Tuning => Message::TuningSemitones(f32::from(controls.tuning_cents()) / 100.0),
            Self::Attack => Message::AttackSeconds(f32::from(controls.attack_us()) / 1_000_000.0),
            Self::Accent => Message::Accent(value / 100.0),
            Self::Overdrive => Message::PedalDrive(value / 36.0),
        }
    }

    pub(super) fn name(self) -> &'static [u8; 9] {
        match self {
            Self::Cutoff => b"CUTOFF   ",
            Self::Resonance => b"RESONANCE",
            Self::EnvMod => b"ENVMOD   ",
            Self::Decay => b"DECAY    ",
            Self::Tuning => b"TUNING   ",
            Self::Attack => b"ATTACK   ",
            Self::Accent => b"ACCENT   ",
            Self::Overdrive => b"OD       ",
        }
    }
}

// Sixth-octave detents, from 62.5 Hz to 8 kHz. The default is 500 Hz.
const CUTOFF_HZ: [u16; 43] = [
    63, 70, 79, 88, 99, 111, 125, 140, 157, 177, 198, 223, 250, 281, 315, 354, 397, 445, 500, 561,
    630, 707, 794, 891, 1000, 1122, 1260, 1414, 1587, 1782, 2000, 2245, 2520, 2828, 3175, 3564,
    4000, 4490, 5040, 5657, 6350, 7127, 8000,
];

/// Closest panel detent for a cutoff set between detents by the sequencer.
pub(super) fn cutoff_detent(hz: u16) -> u8 {
    nearest(&CUTOFF_HZ, hz)
}

fn nearest(table: &[u16], value: u16) -> u8 {
    let upper = table
        .partition_point(|&point| point < value)
        .min(table.len().saturating_sub(1));
    let lower = upper.saturating_sub(1);
    let index = if value.abs_diff(table[lower]) < value.abs_diff(table[upper]) {
        lower
    } else {
        upper
    };
    u8::try_from(index).expect("at most 128 entries")
}

// Finer steps around the stock 1.5 ms attack, then out to 60 ms.
const ATTACK_US: [u16; 18] = [
    0, 250, 500, 750, 1000, 1500, 2000, 3000, 4000, 6000, 8000, 10_000, 15_000, 20_000, 30_000,
    40_000, 50_000, 60_000,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    values: [u8; 8],
    midi: [Option<u8>; 8],
    pub(super) slide: bool,
    pub(super) muffler: Muffler,
    pub(super) last: Knob,
}

impl Controls {
    const VOICE_KNOBS: [Knob; 7] = [
        Knob::Cutoff,
        Knob::Resonance,
        Knob::EnvMod,
        Knob::Decay,
        Knob::Tuning,
        Knob::Attack,
        Knob::Accent,
    ];
    pub(super) const INITIAL: Self = Self {
        // 500 Hz, 50% resonance, 24 semitones, 600 ms, concert pitch, 1.5 ms, 50% accent, OD off.
        values: [18, 50, 24, 60, 120, 5, 50, 0],
        midi: [None; 8],
        slide: false,
        muffler: Muffler::Off,
        last: Knob::Cutoff,
    };

    /// The factory sequence's resonant, driven voice, also available to live keys.
    pub(super) const DEEP_HOUSE: Self = Self {
        // 315 Hz, 82% resonance, 36 semitones, 330 ms, 1.5 ms, 85% accent, 50% OD.
        values: [14, 82, 36, 33, 120, 5, 85, 18],
        ..Self::INITIAL
    };

    /// Shorter, less driven voice for the upper Dm7/Cmaj7 line.
    pub(super) const HARMONY: Self = Self {
        values: [21, 72, 26, 21, 120, 4, 68, 16],
        ..Self::INITIAL
    };

    /// Hand the three automated controls back to their retained panel settings.
    #[cfg(test)]
    pub(super) fn automation_reset(self) -> [Message; 3] {
        [Knob::Cutoff, Knob::Resonance, Knob::Overdrive].map(|knob| knob.message(&self))
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A seven-bit value scaled to the panel range fits u16."
    )]
    pub(super) fn set_midi(&mut self, cc: u8, value: u8, voice: &mut Voice) {
        if cc == CC_SLIDE {
            self.slide = value >= 64;
        } else if cc == CC_WAVE {
            voice.waveform = u8::from(value >= 64);
        } else if let Some(knob) = Knob::from_cc(cc) {
            let (lo, hi) = knob.limits();
            self.values[knob as usize] = match knob {
                Knob::Cutoff => cutoff_detent(MIDI_CUTOFF[usize::from(value)]),
                Knob::Attack => nearest(
                    &ATTACK_US,
                    u16::try_from(u32::from(value) * 60_000 / 127).expect("attack range"),
                ),
                _ => lo.saturating_add(
                    u8::try_from((u16::from(hi - lo) * u16::from(value) + 63) / 127)
                        .expect("panel detent"),
                ),
            };
            self.midi[knob as usize] = Some(value);
        }
    }

    /// MIDI value nearest to the retained panel setting when a clip releases this control.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Panel detents and controller ranges are bounded."
    )]
    pub(super) fn midi_value(self, cc: u8, voice: Voice) -> Option<u8> {
        if cc == CC_WAVE {
            return Some(if voice.waveform == 0 { 0 } else { 127 });
        }
        let knob = Knob::from_cc(cc)?;
        if let Some(v) = self.midi[knob as usize] {
            return Some(v);
        }
        let detent = self.detent(knob);
        let (lo, hi) = knob.limits();
        Some(match knob {
            Knob::Cutoff => nearest(&MIDI_CUTOFF, self.cutoff_hz()),
            Knob::Attack => u8::try_from((u32::from(self.attack_us()) * 127 + 30_000) / 60_000)
                .expect("7-bit attack"),
            Knob::Tuning if detent <= 120 => {
                u8::try_from((u16::from(detent) * 64 + 60) / 120).expect("negative tuning")
            }
            Knob::Tuning => {
                64 + u8::try_from((u16::from(detent - 120) * 63 + 60) / 120)
                    .expect("positive tuning")
            }
            _ => {
                let range = u16::from(hi - lo);
                u8::try_from((u16::from(detent - lo) * 127 + range / 2) / range)
                    .expect("7-bit controller")
            }
        })
    }

    pub(super) fn restore(self, cc: u8, voice: Voice) -> Option<Message> {
        if cc == CC_WAVE {
            return Some(Message::Waveform(if voice.waveform == 0 {
                Waveform::Sawtooth
            } else {
                Waveform::Square
            }));
        }
        Some(Knob::from_cc(cc)?.message(&self))
    }

    pub(super) fn detent(self, knob: Knob) -> u8 {
        self.values[knob as usize]
    }

    pub(super) fn turn_muffler(&mut self, detents: i8) {
        let position = (self.muffler as u8).saturating_add_signed(detents);
        self.muffler = match position {
            0 => Muffler::Off,
            1 => Muffler::One,
            _ => Muffler::Two,
        };
    }

    pub(super) fn muffler_label(self) -> &'static str {
        match self.muffler {
            Muffler::Off => "OFF",
            Muffler::One => "1",
            Muffler::Two => "2",
        }
    }

    pub(super) fn turn_knobs(&mut self, detents: [i8; 4]) {
        for (&knob, delta) in Knob::direct().iter().zip(detents) {
            self.turn(knob, delta);
        }
    }

    pub(super) fn turn(&mut self, knob: Knob, detents: i8) {
        if detents == 0 {
            return;
        }
        self.midi[knob as usize] = None;
        let (min, max) = knob.limits();
        let value = &mut self.values[knob as usize];
        *value = value.saturating_add_signed(detents).clamp(min, max);
        self.last = knob;
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

    pub(super) fn readout(self, knob: Knob) -> (&'static [u8], i32, &'static [u8]) {
        let value = i32::from(self.detent(knob));
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
            Knob::Overdrive => (value.saturating_mul(100).saturating_add(18) / 36, b"PCT  "),
        };
        (knob.name(), number, units)
    }

    fn voice_params(self) -> VoiceParams {
        let mut params = VoiceParams::default();
        for knob in Self::VOICE_KNOBS {
            match knob.message(&self) {
                Message::CutoffHz(v) => params.cutoff_hz = v,
                Message::Resonance(v) => params.resonance = v,
                Message::EnvOctaves(v) => params.env_oct = v,
                Message::DecaySeconds(v) => params.decay_s = v,
                Message::TuningSemitones(v) => params.tuning_st = v,
                Message::AttackSeconds(v) => params.attack_s = v,
                Message::Accent(v) => params.accent = v,
                _ => unreachable!("voice calibration knob"),
            }
        }
        params
    }

    pub(super) fn events(self, previous: Self, voice: Voice, old_voice: Voice) -> EventBatch {
        let mut batch = EventBatch::new();
        if self.values != previous.values || self.midi != previous.midi {
            for knob in CONTROLLERS.into_iter().filter_map(Knob::from_cc) {
                let i = knob as usize;
                if self.values[i] == previous.values[i] && self.midi[i] == previous.midi[i] {
                    continue;
                }
                let message = knob.message(&self);
                batch.push(at(message)).expect("eight panel controls");
            }
        }
        if self.muffler != previous.muffler {
            batch
                .push(at(Message::Muffler(self.muffler)))
                .expect("one Muffler change");
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
                    accent: note.velocity >= 100,
                    slide: self.slide
                        || (before.is_some() && voice.gate.phrase == old_voice.gate.phrase),
                },
                (Some(old), None) => Message::NoteOff(old.number),
                (None, None) => return batch,
            };
            batch.push(at(message)).expect("one note change");
        }
        batch
    }
}

/// Translate the TB-03 voice controls at the graph boundary.
pub(super) fn midi_parameter(cc: u8, value: u8) -> Option<Message> {
    let unit = f32::from(value) / 127.0;
    Some(match cc {
        74 => Message::CutoffHz(f32::from(MIDI_CUTOFF[usize::from(value)])),
        71 => Message::Resonance(unit),
        12 => Message::EnvOctaves(unit * 6.0),
        75 => Message::DecaySeconds(0.05 + unit * 1.95),
        104 => Message::TuningSemitones(
            (f32::from(value) - 64.0) * (12.0 / if value < 64 { 64.0 } else { 63.0 }),
        ),
        73 => Message::AttackSeconds(unit * 0.060),
        16 => Message::Accent(unit),
        17 => Message::PedalDrive(unit),
        CC_WAVE => Message::Waveform(if value < 64 {
            Waveform::Sawtooth
        } else {
            Waveform::Square
        }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_uses_the_same_cutoff_scale_as_the_encoder() {
        for (detent, hz) in CUTOFF_HZ.into_iter().enumerate() {
            assert_eq!(usize::from(cutoff_detent(hz)), detent);
        }
        assert_eq!(cutoff_detent(0), 0);
        assert_eq!(cutoff_detent(u16::MAX), 42);
        assert_eq!(
            cutoff_detent(316),
            Controls::DEEP_HOUSE.detent(Knob::Cutoff)
        );
    }
}
