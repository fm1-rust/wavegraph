//! The two signal paths share one oscillator and its phase accumulator.

use heapless::Vec;
use wavegraph::{
    Event, Process,
    bytecode::{Error, Patch, Program, VERSION},
    dsp::Software,
};
use wavegraph_monosynth::{Message, Waveform};
use wavegraph_sample::I24;

const PRESETS: [(Waveform, &[u8]); 4] = [
    (Waveform::Sine, b"SINE    "),
    (Waveform::Triangle, b"TRIANGLE"),
    (Waveform::Sawtooth, b"SAWTOOTH"),
    (Waveform::Square, b"SQUARE  "),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Note {
    pub(super) number: u8,
    pub(super) velocity: u8,
}

/// Press order and strike velocities of the physical keys that are still down.
#[derive(Debug, Default)]
pub(super) struct HeldNotes<const KEYS: usize>(Vec<Note, KEYS>);

impl<const KEYS: usize> HeldNotes<KEYS> {
    pub(super) fn press(&mut self, note: Note) {
        self.release(note.number);
        self.0.push(note).expect("one slot per physical key");
    }

    pub(super) fn release(&mut self, number: u8) {
        self.0.retain(|note| note.number != number);
    }

    pub(super) fn current(&self) -> Option<Note> {
        self.0.last().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Voice {
    preset: u8,
    pub(super) note: Option<Note>,
    pub(super) transpose: i8,
}

impl Voice {
    pub(super) const INITIAL: Self = Self {
        preset: 0,
        note: None,
        transpose: 0,
    };

    pub(super) fn turn_preset(&mut self, detents: i8) {
        self.preset = self.preset.wrapping_add_signed(detents) % 4;
    }

    pub(super) fn label(self) -> &'static [u8] {
        PRESETS[usize::from(self.preset)].1
    }

    fn sounding(self) -> Option<Note> {
        self.note.map(|mut note| {
            note.number = note.number.saturating_add_signed(self.transpose);
            note
        })
    }

    pub(super) fn note_name(self) -> [u8; 3] {
        const NAMES: [[u8; 2]; 12] = [
            *b"C ", *b"C#", *b"D ", *b"D#", *b"E ", *b"F ", *b"F#", *b"G ", *b"G#", *b"A ", *b"A#",
            *b"B ",
        ];
        let Some(Note { number: note, .. }) = self.sounding() else {
            return *b"-- ";
        };
        let [letter, accidental] = NAMES[usize::from(note % 12)];
        let octave = b'0'.saturating_add((note / 12).saturating_sub(1));
        if accidental == b'#' {
            [letter, accidental, octave]
        } else {
            [letter, octave, b' ']
        }
    }

    fn events(self, previous: Option<Self>) -> Vec<Event<Message>, 2> {
        let mut events = Vec::new();
        let mut send = |message| {
            events
                .push(Event {
                    frame: 0,
                    port: 0,
                    message,
                })
                .expect("at most one waveform change and one note change");
        };
        if previous.is_none_or(|old| old.preset != self.preset) {
            send(Message::Waveform(PRESETS[usize::from(self.preset)].0));
        }
        let before = previous.and_then(Self::sounding);
        let next = self.sounding();
        if before != next {
            if let Some(Note { number, velocity }) = next {
                send(Message::NoteOn {
                    note: number,
                    velocity,
                });
            } else if let Some(last) = before {
                send(Message::NoteOff(last.number));
            }
        }
        events
    }
}

type Plan = Program<'static, 2, 1>;
pub(super) struct Programs([Plan; 2]);

impl Programs {
    pub(super) fn decode() -> Result<Self, Error> {
        // CALL object 0, zero signal inputs, one output (bus 0).
        // The second program appends GAIN bus 0, 64/128 (bus 1).
        Ok(Self([
            Plan::decode(&[VERSION, 1, 0, 0, 1])?,
            Plan::decode(&[VERSION, 1, 0, 0, 1, 2, 0, 64])?,
        ]))
    }

    pub(super) fn bind<'a>(
        &'a self,
        nodes: &'a mut [&'a mut dyn Process<Message, I24>],
    ) -> Result<Instrument<'a>, Error> {
        Ok(Instrument {
            programs: self,
            patch: self.0[0].bind(nodes)?,
            half_gain: false,
            previous: None,
        })
    }
}

pub(super) struct Instrument<'a> {
    programs: &'a Programs,
    patch: Patch<'a, 'a, Message, I24, 1>,
    pub(super) half_gain: bool,
    previous: Option<Voice>,
}

impl Instrument<'_> {
    pub(super) fn set_half_gain(&mut self, enabled: bool) -> Result<(), Error> {
        self.patch
            .replace(&self.programs.0[usize::from(enabled)], 2)?;
        self.half_gain = enabled;
        Ok(())
    }

    pub(super) fn render<'b, const N: usize>(
        &mut self,
        voice: Voice,
        buses: &'b mut [[I24; N]; 2],
    ) -> Result<&'b [I24; N], Error> {
        let events = voice.events(self.previous);
        let inlet = (!events.is_empty()).then_some(events.as_slice());
        self.patch
            .render(&mut Software, buses, inlet.as_slice(), |_, event| {
                Err(event)
            })?;
        self.previous = Some(voice);
        Ok(&buses[usize::from(self.half_gain)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wavegraph_monosynth::{Monosynth, Tuning};

    #[test]
    fn last_pressed_key_wins_and_recalls_its_strike() {
        let a = Note {
            number: 69,
            velocity: 35,
        };
        let c = Note {
            number: 72,
            velocity: 113,
        };
        let e = Note {
            number: 76,
            velocity: 64,
        };
        let mut held = HeldNotes::<3>::default();
        for note in [a, c, e] {
            held.press(note);
        }
        held.release(c.number);
        assert_eq!(held.current(), Some(e));
        held.release(e.number);
        assert_eq!(held.current(), Some(a));
        held.press(c);
        held.release(a.number);
        assert_eq!(held.current(), Some(c));
        held.release(c.number);
        assert_eq!(held.current(), None);
    }

    #[test]
    fn plays_at_strike_velocity_and_releases_the_transposed_note() -> Result<(), Error> {
        let programs = Programs::decode()?;
        let tuning = Tuning::new(44_100, 1).expect("audio sample clock");
        let mut synth = Monosynth::new(&tuning);
        let mut nodes: [&mut dyn Process<Message, I24>; 1] = [&mut synth];
        let mut instrument = programs.bind(&mut nodes)?;
        let mut buses = [[I24::default(); 8]; 2];
        let mut voice = Voice::INITIAL;
        assert_eq!(voice.note_name(), *b"-- ");
        assert_eq!(
            instrument.render(voice, &mut buses)?.map(I24::inner),
            [0; 8]
        );
        voice.turn_preset(-1);
        assert_eq!(voice.label(), b"SQUARE  ");
        assert_eq!(
            instrument.render(voice, &mut buses)?.map(I24::inner),
            [0; 8]
        );
        for velocity in [32, 96, 32] {
            voice.note = Some(Note {
                number: 69,
                velocity,
            });
            voice.transpose = 12;
            assert_eq!(voice.note_name(), *b"A5 ");
            let block = instrument.render(voice, &mut buses)?.map(I24::inner);
            let level = (32767 * i32::from(velocity)) >> 3;
            assert!(block.iter().all(|sample| sample.abs() == level));
        }
        voice.note = None;
        voice.transpose = 0;
        assert_eq!(
            instrument.render(voice, &mut buses)?.map(I24::inner),
            [0; 8]
        );
        Ok(())
    }

    #[test]
    fn changing_routes_preserves_the_sounding_oscillators_phase() -> Result<(), Error> {
        let programs = Programs::decode()?;
        let tuning = Tuning::new(1760, 1).expect("four samples per A4 cycle");
        let mut synth = Monosynth::new(&tuning);
        let mut nodes: [&mut dyn Process<Message, I24>; 1] = [&mut synth];
        let mut instrument = programs.bind(&mut nodes)?;
        let mut buses = [[I24::default(); 3]; 2];
        let voice = Voice {
            note: Some(Note {
                number: 69,
                velocity: 80,
            }),
            ..Voice::INITIAL
        };
        assert_eq!(voice.note_name(), *b"A4 ");
        let dry = instrument.render(voice, &mut buses)?.map(I24::inner);
        assert_eq!(dry[0], 0);
        assert!(dry[1] > 0);
        instrument.set_half_gain(true)?;
        let half = instrument.render(voice, &mut buses)?.map(I24::inner);
        assert!(half[0] < 0);
        assert_eq!(half, buses[0].map(|sample| sample.inner() >> 1));
        Ok(())
    }
}
