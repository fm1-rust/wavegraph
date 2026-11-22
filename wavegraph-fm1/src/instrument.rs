//! Two persistent instrument nodes feed a common mix and switchable gain stage.

use crate::{bass::Controls, envelope::EnvelopeControls};
use heapless::Vec;
use wavegraph::{
    Event, Port, Process, ProcessContext,
    bytecode::{Error, Patch, Program, VERSION},
    dsp::Software,
};
use wavegraph_303::{Message as BassMessage, Tb303};
use wavegraph_monosynth::{Message as MonoMessage, Monosynth, Waveform};
use wavegraph_sample::I24;

const MONO_WAVEFORMS: [(Waveform, &[u8]); 4] = [
    (Waveform::Sine, b"SINE    "),
    (Waveform::Triangle, b"TRIANGLE"),
    (Waveform::Sawtooth, b"SAWTOOTH"),
    (Waveform::Square, b"SQUARE  "),
];

const MAX_CUTOFF_STEP: u8 = 66;
// Two semitones per detent, rounded to the nearest Hz from 20 kHz down.
const CUTOFF_HZ: [u16; MAX_CUTOFF_STEP as usize + 1] = [
    10, 11, 12, 14, 16, 17, 20, 22, 25, 28, 31, 35, 39, 44, 49, 55, 62, 70, 78, 88, 98, 110, 124,
    139, 156, 175, 197, 221, 248, 278, 313, 351, 394, 442, 496, 557, 625, 702, 787, 884, 992, 1114,
    1250, 1403, 1575, 1768, 1984, 2227, 2500, 2806, 3150, 3536, 3969, 4454, 5000, 5612, 6300, 7071,
    7937, 8909, 10000, 11225, 12599, 14142, 15874, 17818, 20000,
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Part {
    #[default]
    Mono,
    Bass,
}

impl Part {
    pub(super) fn turn_preset(&mut self, detents: i8) {
        if detents % 2 != 0 {
            *self = match self {
                Self::Mono => Self::Bass,
                Self::Bass => Self::Mono,
            };
        }
    }

    pub(super) const fn tap(self) -> usize {
        match self {
            Self::Mono => 0,
            Self::Bass => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Note {
    pub(super) number: u8,
    pub(super) velocity: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Gate {
    pub(super) note: Option<Note>,
    // Distinguish a fresh strike from legato even when a short gap fits inside one DMA block.
    pub(super) phrase: u32,
}

/// Each key keeps the instrument selected at note-on until it is released.
#[derive(Debug, Default)]
pub(super) struct Keyboard<const KEYS: usize> {
    pub(super) focus: Part,
    held: Vec<(Part, Note), KEYS>,
    gates: [Gate; 2],
}

impl<const KEYS: usize> Keyboard<KEYS> {
    pub(super) fn press(&mut self, note: Note) {
        self.release(note.number);
        let gate = &mut self.gates[self.focus as usize];
        if gate.note.is_none() {
            gate.phrase = gate.phrase.wrapping_add(1);
        }
        gate.note = Some(note);
        self.held
            .push((self.focus, note))
            .expect("one entry per pressed key");
    }

    pub(super) fn release(&mut self, number: u8) {
        if let Some(i) = self.held.iter().position(|(_, note)| note.number == number) {
            let (part, _) = self.held.remove(i);
            self.gates[part as usize].note = self
                .held
                .iter()
                .rev()
                .find(|(owner, _)| *owner == part)
                .map(|(_, note)| *note);
        }
    }

    pub(super) fn gates(&self) -> [Gate; 2] {
        self.gates
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Voice {
    pub(super) waveform: u8,
    cutoff_step: u8,
    pub(super) gate: Gate,
    pub(super) transpose: i8,
}

impl Voice {
    pub(super) const INITIAL: Self = Self {
        waveform: 0,
        cutoff_step: MAX_CUTOFF_STEP - 12, // 5 kHz

        gate: Gate {
            note: None,
            phrase: 0,
        },
        transpose: 0,
    };

    pub(super) fn turn_waveform(&mut self, part: Part, detents: i8) {
        let position = self.waveform.wrapping_add_signed(detents);
        self.waveform = match part {
            Part::Mono => position % 4,
            Part::Bass => position % 2,
        };
    }

    pub(super) fn turn_cutoff(&mut self, detents: i8) {
        self.cutoff_step = self
            .cutoff_step
            .saturating_add_signed(detents)
            .min(MAX_CUTOFF_STEP);
    }

    pub(super) fn cutoff_hz(self) -> u16 {
        CUTOFF_HZ[usize::from(self.cutoff_step)]
    }

    pub(super) fn label(self, part: Part) -> &'static [u8] {
        match part {
            Part::Mono => MONO_WAVEFORMS[usize::from(self.waveform)].1,
            Part::Bass if self.waveform == 0 => b"SAW     ",
            Part::Bass => b"SQUARE  ",
        }
    }

    pub(super) fn sounding(self) -> Option<Note> {
        self.gate.note.map(|mut note| {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Parts {
    pub(super) voices: [Voice; 2],
    pub(super) bass: Controls,
    pub(super) mono_envelope: EnvelopeControls,
}

impl Parts {
    pub(super) const INITIAL: Self = Self {
        voices: [Voice::INITIAL; 2],
        bass: Controls::INITIAL,
        mono_envelope: EnvelopeControls::INITIAL,
    };

    pub(super) fn turn_select(&mut self, focus: Part, detents: i8) {
        match focus {
            Part::Mono => {
                let voice = &mut self.voices[Part::Mono as usize];
                voice.transpose = voice.transpose.saturating_add(detents).clamp(-12, 12);
            }
            Part::Bass => self.bass.turn_select(detents),
        }
    }

    pub(super) fn turn_knobs(&mut self, focus: Part, detents: [i8; 4]) {
        match focus {
            Part::Mono => self.voices[Part::Mono as usize].turn_cutoff(detents[0]),
            Part::Bass => self.bass.turn_knobs(detents),
        }
    }

    fn mono_events(self, previous: Option<Self>) -> EventBatch {
        let mono = self.voices[Part::Mono as usize];
        let old = previous.map(|parts| parts.voices[Part::Mono as usize]);
        let mut events = EventBatch::new();
        if old.is_none_or(|v| v.waveform != mono.waveform) {
            events
                .push(at(MonoMessage::Waveform(
                    MONO_WAVEFORMS[usize::from(mono.waveform)].0,
                )))
                .expect("one waveform change");
        }
        if old.is_none_or(|v| v.cutoff_step != mono.cutoff_step) {
            events
                .push(at(MonoMessage::CutoffHz(mono.cutoff_hz())))
                .expect("one cutoff change");
        }
        if previous.is_none_or(|p| p.mono_envelope != self.mono_envelope) {
            events
                .push(at(MonoMessage::VcaEnvelope(self.mono_envelope.adsr())))
                .expect("one envelope change");
        }
        let before = old.and_then(Voice::sounding);
        let next = mono.sounding();
        let fresh = old.is_some_and(|v| v.gate.phrase != mono.gate.phrase);
        // A release and new strike can both fall between two audio blocks.
        // Deliver the gate edges in order; overlapping keys only change pitch.
        if let Some(last) = before.filter(|_| next.is_none() || fresh) {
            events
                .push(at(MonoMessage::NoteOff(last.number)))
                .expect("one note-off");
        }
        if let Some(note) = next.filter(|_| next != before || fresh) {
            events
                .push(at(MonoMessage::NoteOn {
                    note: note.number,
                    velocity: note.velocity,
                }))
                .expect("one note-on");
        }
        events
    }
}

// Both native instruments share a graph and keep their own message vocabulary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Message {
    Mono(MonoMessage),
    Bass(BassMessage),
}

impl From<MonoMessage> for Message {
    fn from(value: MonoMessage) -> Self {
        Self::Mono(value)
    }
}
impl From<BassMessage> for Message {
    fn from(value: BassMessage) -> Self {
        Self::Bass(value)
    }
}

pub(super) fn at(message: impl Into<Message>) -> Event<Message> {
    Event {
        frame: 0,
        port: 0,
        message: message.into(),
    }
}

// One waveform, eight knob assignments and one note transition at the block boundary.
const MAX_EVENTS: usize = 10;
pub(super) type EventBatch = Vec<Event<Message>, MAX_EVENTS>;

pub(super) enum Node<'a> {
    Mono(Monosynth<'a>),
    Bass(Tb303),
}

impl Process<Message, I24> for Node<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        match self {
            Self::Mono(node) => dispatch(node, cx, |m| match m {
                Message::Mono(m) => Some(m),
                _ => None,
            }),
            Self::Bass(node) => dispatch(node, cx, |m| match m {
                Message::Bass(m) => Some(m),
                _ => None,
            }),
        }
    }
}

fn dispatch<M>(
    node: &mut impl Process<M, I24>,
    cx: &mut ProcessContext<'_, '_, Message, I24>,
    unpack: impl Fn(Message) -> Option<M>,
) {
    let events: Vec<_, MAX_EVENTS> = cx
        .events
        .iter()
        .filter_map(|e| {
            unpack(e.message).map(|message| Event {
                frame: e.frame,
                port: e.port,
                message,
            })
        })
        .collect();
    node.process(&mut ProcessContext {
        frames: cx.frames,
        inputs: cx.inputs,
        outputs: cx.outputs,
        events: &events,
        emit: &mut |e| Err(e), // Both native voices have only a signal outlet.
    });
}

pub(super) const BUS_COUNT: usize = 5;
type Plan = Program<'static, 5, 1>;
pub(super) struct Programs([Plan; 2]);

impl Programs {
    pub(super) fn decode() -> Result<Self, Error> {
        // CALL mono -> 0, CALL bass -> 1, GAIN bass 32/128 -> 2, MIX 0+2 -> 3.
        // The mono voice peaks at 1/16 full scale; trimming the bass leaves room for both.
        const ROUTE: &[u8] = &[VERSION, 1, 0, 0, 1, 1, 1, 0, 1, 2, 1, 32, 3, 0, 2, 2, 3, 64];
        // The last instruction applies half gain to the mixed bus -> 4.
        const DRY_LEN: usize = ROUTE.len() - 3;
        Ok(Self([
            Plan::decode(&ROUTE[..DRY_LEN])?,
            Plan::decode(ROUTE)?,
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
    previous: Option<Parts>,
}

impl Instrument<'_> {
    pub(super) fn set_half_gain(&mut self, enabled: bool) -> Result<(), Error> {
        self.patch
            .replace(&self.programs.0[usize::from(enabled)], BUS_COUNT)?;
        self.half_gain = enabled;
        Ok(())
    }

    pub(super) fn render<'b, const N: usize>(
        &mut self,
        parts: Parts,
        buses: &'b mut [[I24; N]; BUS_COUNT],
    ) -> Result<&'b [I24; N], Error> {
        let bass = parts.voices[Part::Bass as usize];
        let mono_events = parts.mono_events(self.previous);
        let old = self.previous.unwrap_or(Parts::INITIAL);
        let bass_events = parts
            .bass
            .events(old.bass, bass, old.voices[Part::Bass as usize]);
        self.patch.render(
            &mut Software,
            buses,
            &[&mono_events, &bass_events],
            |_, e| Err(e),
        )?;
        self.previous = Some(parts);
        Ok(&buses[if self.half_gain { 4 } else { 3 }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wavegraph_monosynth::Tuning;

    fn note(number: u8, velocity: u8) -> Note {
        Note { number, velocity }
    }

    fn with_instrument(
        mono_rate: u32,
        test: impl FnOnce(&mut Instrument<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let programs = Programs::decode()?;
        let tuning = Tuning::new(mono_rate, 1).expect("audio sample clock");
        let mut mono = Node::Mono(Monosynth::new(&tuning));
        let mut bass = Node::Bass(Tb303::new(24_000_000.0 / 544.0));
        let mut nodes: [&mut dyn Process<Message, I24>; 2] = [&mut mono, &mut bass];
        test(&mut programs.bind(&mut nodes)?)
    }

    #[test]
    fn keys_keep_their_owner_and_recall_each_parts_last_strike() {
        let mut keys = Keyboard::<3>::default();
        let (a, c, e) = (note(69, 35), note(72, 113), note(76, 64));
        keys.press(a);
        keys.press(c);
        let [mono, _] = keys.gates();
        assert_eq!(mono.note, Some(c));
        keys.focus.turn_preset(-1);
        keys.press(e);
        keys.release(c.number);
        assert_eq!(keys.gates().map(|g| g.note), [Some(a), Some(e)]);
        keys.focus.turn_preset(i8::MIN);
        assert_eq!(keys.focus, Part::Bass);
        keys.focus.turn_preset(i8::MAX);
        assert_eq!(keys.focus, Part::Mono);
        keys.release(e.number);
        assert_eq!(keys.gates().map(|g| g.note), [Some(a), None]);
        keys.release(a.number);
        keys.press(a);
        assert_ne!(keys.gates()[0].phrase, mono.phrase);
        keys.release(a.number);
        assert_eq!(keys.gates().map(|g| g.note), [None, None]);
    }

    #[test]
    fn knob_one_preserves_both_cutoffs_and_sends_the_initial_monosynth_cutoff() {
        let mut parts = Parts::INITIAL;
        let initial = parts;
        assert_eq!(initial.voices[0].cutoff_hz(), 5000);
        assert_eq!(
            initial.mono_events(None).as_slice(),
            &[
                at(MonoMessage::Waveform(Waveform::Sine)),
                at(MonoMessage::CutoffHz(5000)),
                at(MonoMessage::VcaEnvelope(parts.mono_envelope.adsr())),
            ]
        );
        parts.turn_knobs(Part::Mono, [-6, 0, 0, 0]);
        assert_eq!(parts.bass, Controls::INITIAL);
        parts.turn_knobs(Part::Bass, [6, 0, 0, 0]);
        assert_eq!(parts.bass.readout().1, 1000);
        parts.mono_envelope.turn([1, -1, 2, -2]);
        let mono = &mut parts.voices[Part::Mono as usize];
        assert_eq!(mono.cutoff_hz(), 2500);
        mono.turn_waveform(Part::Mono, 2);
        mono.gate.note = Some(note(69, 80));
        assert_eq!(
            parts.mono_events(Some(initial)).as_slice(),
            &[
                at(MonoMessage::Waveform(Waveform::Sawtooth)),
                at(MonoMessage::CutoffHz(2500)),
                at(MonoMessage::VcaEnvelope(parts.mono_envelope.adsr())),
                at(MonoMessage::NoteOn {
                    note: 69,
                    velocity: 80
                }),
            ]
        );
        let held = parts;
        parts.voices[0].turn_cutoff(i8::MIN);
        assert_eq!(
            parts.mono_events(Some(held)).as_slice(),
            &[at(MonoMessage::CutoffHz(10))]
        );
        parts.voices[0].turn_cutoff(i8::MAX);
        assert_eq!(parts.voices[0].cutoff_hz(), 20000);
        let open = parts;
        parts.voices[0].turn_cutoff(1);
        assert!(parts.mono_events(Some(open)).is_empty());
        assert_eq!(parts.bass.readout().1, 1000);
    }

    #[test]
    fn mono_retriggers_after_a_short_gap_and_plays_overlapping_keys_legato() {
        let mut keys = Keyboard::<2>::default();
        let mut parts = Parts::INITIAL;
        keys.press(note(60, 80));
        parts.voices[0].gate = keys.gates()[0];
        let first = parts;
        let play = |number| {
            at(MonoMessage::NoteOn {
                note: number,
                velocity: 80,
            })
        };

        // Both edges fit between DMA blocks, even at the same pitch and velocity.
        keys.release(60);
        keys.press(note(60, 80));
        parts.voices[0].gate = keys.gates()[0];
        assert_eq!(
            parts.mono_events(Some(first)).as_slice(),
            &[at(MonoMessage::NoteOff(60)), play(60)]
        );

        let repeated = parts;
        keys.press(note(67, 80));
        parts.voices[0].gate = keys.gates()[0];
        assert_eq!(parts.mono_events(Some(repeated)).as_slice(), &[play(67)]);
        let legato = parts;
        keys.release(67);
        parts.voices[0].gate = keys.gates()[0];
        assert_eq!(parts.mono_events(Some(legato)).as_slice(), &[play(60)]);
    }

    #[test]
    fn plays_at_strike_velocity_and_releases_the_transposed_note() -> Result<(), Error> {
        // At this rate the 5 kHz default is above Nyquist, so the square keeps its exact levels.
        with_instrument(8_000, |instrument| {
            let mut buses = [[I24::default(); 32]; BUS_COUNT];
            let mut parts = Parts::INITIAL;
            parts
                .mono_envelope
                .turn([i8::MIN, i8::MIN, i8::MAX, i8::MIN]);
            assert_eq!(parts.voices[0].note_name(), *b"-- ");
            assert_eq!(
                instrument.render(parts, &mut buses)?.map(I24::inner),
                [0; 32]
            );
            parts.voices[0].turn_waveform(Part::Mono, -1);
            assert_eq!(parts.voices[0].label(Part::Mono), b"SQUARE  ");
            for velocity in [32, 96, 32] {
                let mono = &mut parts.voices[0];
                mono.gate.note = Some(note(69, velocity));
                mono.transpose = 12;
                assert_eq!(mono.note_name(), *b"A5 ");
                let block = instrument.render(parts, &mut buses)?.map(I24::inner);
                let level = (32767 * i32::from(velocity)) >> 3;
                assert!(block[12..].iter().all(|sample| sample.abs() == level));
            }
            parts.voices[0].gate.note = None;
            parts.voices[0].transpose = 0;
            let release = instrument.render(parts, &mut buses)?.map(I24::inner);
            assert!(release[0].abs() > release[14].abs());
            assert_ne!(release[14], 0);
            assert_eq!(release[15..], [0; 17]);
            Ok(())
        })
    }

    #[test]
    fn changing_routes_preserves_the_sounding_oscillators_phase() -> Result<(), Error> {
        with_instrument(1760, |instrument| {
            let mut buses = [[I24::default(); 3]; BUS_COUNT];
            let mut parts = Parts::INITIAL;
            parts.voices[0].gate.note = Some(note(69, 80));
            assert_eq!(parts.voices[0].note_name(), *b"A4 ");
            let dry = instrument.render(parts, &mut buses)?.map(I24::inner);
            assert_eq!(dry[0], 0);
            assert!(dry[1] > 0);
            instrument.set_half_gain(true)?;
            let half = instrument.render(parts, &mut buses)?.map(I24::inner);
            assert!(half[0] < 0);
            assert_eq!(half, buses[0].map(|sample| sample.inner() >> 1));
            Ok(())
        })
    }

    #[test]
    fn both_voices_reach_the_mix_and_releasing_one_preserves_the_other() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut parts = Parts::INITIAL;
            let mut keys = Keyboard::<2>::default();
            let sync = |parts: &mut Parts, keys: &Keyboard<2>| {
                for (voice, gate) in parts.voices.iter_mut().zip(keys.gates()) {
                    voice.gate = gate;
                }
            };
            keys.press(note(69, 84));
            sync(&mut parts, &keys);
            instrument.render(parts, &mut buses)?;
            assert_eq!(buses[Part::Bass.tap()].map(I24::inner), [0; 128]);
            keys.focus.turn_preset(1);
            keys.press(note(53, 110));
            sync(&mut parts, &keys);
            for half in [false, true] {
                instrument.set_half_gain(half)?;
                let mix = instrument.render(parts, &mut buses)?.map(I24::inner);
                let mono = &buses[Part::Mono.tap()];
                let bass = &buses[Part::Bass.tap()];
                assert!(mono.iter().any(|s| s.inner() != 0));
                assert!(bass.iter().any(|s| s.inner() != 0));
                for i in 0..mix.len() {
                    assert_eq!(
                        mix[i],
                        (mono[i].inner() + bass[i].inner()) >> u8::from(half)
                    );
                }
            }
            keys.release(69);
            sync(&mut parts, &keys);
            instrument.render(parts, &mut buses)?;
            assert!(buses[Part::Mono.tap()].iter().any(|s| s.inner() != 0));
            // The mono's 125 ms release continues while the 303 key is held.
            for _ in 0..44 {
                instrument.render(parts, &mut buses)?;
            }
            assert_eq!(buses[Part::Mono.tap()].map(I24::inner), [0; 128]);
            assert!(buses[Part::Bass.tap()].iter().any(|s| s.inner() != 0));
            keys.focus.turn_preset(-1);
            keys.release(53);
            sync(&mut parts, &keys);
            instrument.render(parts, &mut buses)?;
            assert!(buses[Part::Bass.tap()].iter().any(|s| s.inner() != 0));
            for _ in 0..60 {
                instrument.render(parts, &mut buses)?;
            }
            assert!(buses[Part::Bass.tap()].iter().all(|s| s.inner().abs() <= 1));
            Ok(())
        })
    }

    #[test]
    fn select_switches_the_legend_without_changing_the_sound_and_recalls_each_bank() {
        let mut parts = Parts::INITIAL;
        let voice = Voice {
            gate: Gate {
                note: Some(note(53, 100)),
                phrase: 1,
            },
            ..Voice::INITIAL
        };
        parts.voices = [voice; 2];
        parts.turn_knobs(Part::Bass, [0, 0, 0, -10]);
        let filter = parts.bass;
        assert_eq!(
            filter.readout(),
            (b"DECAY    ".as_slice(), 500, b"MS   ".as_slice())
        );
        parts.turn_select(Part::Bass, -1);
        assert_eq!(
            parts.bass.bank.knobs().map(|k| k.name()),
            [b"TUNING   ", b"ATTACK   ", b"ACCENT   ", b"OVERDRIVE"]
        );
        assert_eq!(parts.bass.last().name(), b"TUNING   ");
        assert!(parts.bass.events(filter, parts.voices[1], voice).is_empty());
        parts.turn_knobs(Part::Bass, [-7, 1, 5, 12]);
        let edited = parts.bass;
        assert_eq!(edited.readout().1, 12);
        assert_eq!(
            &*edited.events(filter, voice, voice),
            &[
                at(BassMessage::TuningSemitones(-0.7)),
                at(BassMessage::AttackSeconds(0.002)),
                at(BassMessage::Accent(0.55)),
                at(BassMessage::OverdriveDb(12.0)),
            ]
        );
        parts.turn_select(Part::Bass, i8::MIN);
        assert_eq!(parts.bass, edited);
        parts.turn_select(Part::Bass, i8::MAX);
        assert_eq!(
            parts.bass.bank.knobs().map(|k| k.name()),
            [b"CUTOFF   ", b"RESONANCE", b"ENVMOD   ", b"DECAY    "]
        );
        assert_eq!(parts.bass.readout(), filter.readout());
        parts.turn_select(Part::Mono, -4);
        assert_eq!(parts.voices[0].sounding(), Some(note(49, 100)));
        assert_eq!(parts.voices[1], voice);
        parts.turn_select(Part::Bass, 1);
        assert_eq!(parts.bass, edited);
    }

    #[test]
    fn bass_legato_retrigger_and_panel_changes() {
        let mut controls = Controls::INITIAL;
        let mut keys = Keyboard::<2>::default();
        keys.focus.turn_preset(1);
        let mut bass = Voice::INITIAL;
        let mut old = bass;
        keys.press(note(53, 95));
        bass.gate = keys.gates()[1];
        assert_eq!(
            &*controls.events(controls, bass, old),
            &[at(BassMessage::NoteOn {
                note: 53,
                accent: false,
                slide: false,
            })]
        );
        old = bass;
        keys.press(note(60, 96));
        bass.gate = keys.gates()[1];
        assert_eq!(
            &*controls.events(controls, bass, old),
            &[at(BassMessage::NoteOn {
                note: 60,
                accent: true,
                slide: true,
            })]
        );
        old = bass;
        keys.release(60);
        keys.release(53);
        keys.press(note(60, 96));
        bass.gate = keys.gates()[1];
        bass.turn_waveform(Part::Bass, -1);
        controls.turn_knobs([i8::MAX; 4]);
        assert_eq!(controls.readout().1, 1870);
        controls.turn_select(1);
        controls.turn_knobs([i8::MAX; 4]);
        assert_eq!(
            &*controls.events(Controls::INITIAL, bass, old),
            &[
                at(BassMessage::CutoffHz(8000.0)),
                at(BassMessage::Resonance(1.0)),
                at(BassMessage::EnvOctaves(6.0)),
                at(BassMessage::DecaySeconds(1.87)),
                at(BassMessage::TuningSemitones(12.0)),
                at(BassMessage::AttackSeconds(0.06)),
                at(BassMessage::Accent(1.0)),
                at(BassMessage::OverdriveDb(36.0)),
                at(BassMessage::Waveform(wavegraph_303::Waveform::Square)),
                at(BassMessage::NoteOn {
                    note: 60,
                    accent: true,
                    slide: false
                }),
            ]
        );
        assert!(controls.events(controls, bass, bass).is_empty());
    }
}
