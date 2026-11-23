//! Persistent instruments feed a shared effect send/return graph.

use crate::{
    bass::Controls,
    envelope::EnvelopeControls,
    fx::{Fx, FxPage},
    organ::{Controls as OrganControls, OrganKey},
    sequencer::{DrumLamp, DrumMessage, PatternPlayer, Position, Track, Transport},
};
use heapless::Vec;
use wavegraph::{
    Event, Port, Process, ProcessContext,
    bytecode::{Error, Patch, Program, VERSION},
    dsp::Software,
};
use wavegraph_303::{Message as BassMessage, Tb303};
use wavegraph_909::Voice as Drum;
use wavegraph_fx::{ChannelMixer, Message as FxMessage, MixerMessage, SidechainDucker};
use wavegraph_monosynth::{Message as MonoMessage, Monosynth, Waveform};
use wavegraph_organ::{Message as OrganMessage, Organ, POLYPHONY, Voicing};
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
    Organ,
    Bass,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Held {
    key: u8,
    part: Part,
    note: Note,
    strike: u32,
}

/// A physical key retains the part and pitch it had when struck.
#[derive(Debug, Default)]
pub(super) struct Keyboard<const KEYS: usize> {
    pub(super) focus: Part,
    pub(super) octave: i8,
    held: Vec<Held, KEYS>,
    gates: [Gate; 3],
    serial: u32,
}

impl<const KEYS: usize> Keyboard<KEYS> {
    pub(super) fn shift_octave(&mut self, delta: i8) {
        self.octave = self.octave.saturating_add(delta).clamp(-3, 3);
    }

    /// The unshifted panel note identifies the physical key on both edges.
    pub(super) fn press(&mut self, key: Note) {
        self.release(key.number);
        let note = Note {
            number: key
                .number
                .saturating_add_signed(self.octave.saturating_mul(12)),
            velocity: key.velocity,
        };
        let gate = &mut self.gates[self.focus as usize];
        if gate.note.is_none() {
            gate.phrase = gate.phrase.wrapping_add(1);
        }
        gate.note = Some(note);
        self.serial = self.serial.wrapping_add(1);
        self.held
            .push(Held {
                key: key.number,
                part: self.focus,
                note,
                strike: self.serial,
            })
            .expect("one entry per pressed key");
    }

    pub(super) fn release(&mut self, key: u8) {
        if let Some(i) = self.held.iter().position(|held| held.key == key) {
            let part = self.held.remove(i).part;
            self.gates[part as usize].note = self
                .held
                .iter()
                .rev()
                .find(|key| key.part == part)
                .map(|key| key.note);
        }
    }

    pub(super) fn gates(&self) -> [Gate; 3] {
        self.gates
    }

    /// Four most recently held organ keys. Older physical keys are restored
    /// when a newer key is released; their strike IDs do not change.
    pub(super) fn organ_keys(&self) -> [OrganKey; POLYPHONY] {
        let mut result = [OrganKey::EMPTY; POLYPHONY];
        let organ_keys = self
            .held
            .iter()
            .rev()
            .filter(|held| held.part == Part::Organ);
        for (slot, held) in result.iter_mut().zip(organ_keys) {
            *slot = OrganKey {
                note: Some(held.note),
                strike: held.strike,
            };
        }
        result
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
            Part::Organ | Part::Bass => position % 2,
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
            Part::Organ if self.waveform == 0 => b"HOUSE   ",
            Part::Organ => b"ANALOG  ",
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

    pub(super) fn note_name(self) -> [u8; 4] {
        const NAMES: [[u8; 2]; 12] = [
            *b"C ", *b"C#", *b"D ", *b"D#", *b"E ", *b"F ", *b"F#", *b"G ", *b"G#", *b"A ", *b"A#",
            *b"B ",
        ];
        let Some(Note { number: note, .. }) = self.sounding() else {
            return *b"--  ";
        };
        let [letter, accidental] = NAMES[usize::from(note % 12)];
        let [octave, tail] = match (note / 12).checked_sub(1) {
            Some(octave) => [b'0'.saturating_add(octave), b' '],
            None => *b"-1",
        };
        if accidental == b'#' {
            [letter, accidental, octave, tail]
        } else {
            [letter, octave, tail, b' ']
        }
    }
}

/// Strike generations for the first six physical white keys, F3 through D4.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct DrumPads([u8; 6]);

impl DrumPads {
    pub(super) fn press(&mut self, key_index: u8) {
        if let Some(i) = [0, 2, 4, 6, 7, 9].iter().position(|&key| key == key_index) {
            self.0[i] = self.0[i].wrapping_add(1);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Parts {
    pub(super) pads: DrumPads,
    pub(super) voices: [Voice; 3],
    pub(super) bass: Controls,
    pub(super) organ: OrganControls,
    pub(super) organ_keys: [OrganKey; POLYPHONY],
    pub(super) mono_envelope: EnvelopeControls,
}

impl Parts {
    pub(super) const INITIAL: Self = Self {
        pads: DrumPads([0; 6]),
        voices: [Voice::INITIAL; 3],
        bass: Controls::INITIAL,
        organ: OrganControls::INITIAL,
        organ_keys: [OrganKey::EMPTY; POLYPHONY],
        mono_envelope: EnvelopeControls::INITIAL,
    };

    pub(super) const DEEP_HOUSE: Self = Self {
        bass: Controls::DEEP_HOUSE,
        organ: OrganControls::DEEP_HOUSE,
        ..Self::INITIAL
    };

    pub(super) fn turn_select(&mut self, focus: Part, detents: i8) {
        match focus {
            Part::Mono | Part::Organ => {
                let voice = &mut self.voices[focus as usize];
                voice.transpose = voice.transpose.saturating_add(detents).clamp(-12, 12);
            }
            Part::Bass => self.bass.turn_select(detents),
        }
    }

    pub(super) fn turn_knobs(&mut self, focus: Part, detents: [i8; 4]) {
        match focus {
            Part::Mono => self.voices[Part::Mono as usize].turn_cutoff(detents[0]),
            Part::Organ => self.organ.turn_knobs(detents),
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

    fn organ_events(self, previous: Option<Self>) -> EventBatch {
        let mut events = self.organ.events(previous.map(|parts| parts.organ));
        let old_keys = previous.map_or([OrganKey::EMPTY; POLYPHONY], |p| p.organ_keys);
        let old_transpose = previous.map_or(0, |p| p.voices[Part::Organ as usize].transpose);
        let now_transpose = self.voices[Part::Organ as usize].transpose;
        let sounding = |key: OrganKey, transpose: i8| {
            key.note.map(|mut note| {
                note.number = note.number.saturating_add_signed(transpose);
                note
            })
        };
        let same = |left: OrganKey, right: OrganKey| {
            left.strike == right.strike
                && sounding(left, old_transpose) == sounding(right, now_transpose)
        };
        // Two physical keys can hold the same pitch across an octave change.
        // Release it only when its last key goes up.
        for &old in &old_keys {
            if let Some(note) = sounding(old, old_transpose)
                && !self.organ_keys.iter().any(|&new| {
                    sounding(new, now_transpose).is_some_and(|held| held.number == note.number)
                })
            {
                events
                    .push(at(OrganMessage::NoteOff(note.number)))
                    .expect("at most four organ note-offs");
            }
        }
        // Apply simultaneous strikes oldest first so the newest velocity wins.
        for &new in self.organ_keys.iter().rev() {
            if let Some(note) = sounding(new, now_transpose)
                && !old_keys.iter().any(|&old| same(old, new))
            {
                events
                    .push(at(OrganMessage::NoteOn {
                        note: note.number,
                        velocity: note.velocity,
                    }))
                    .expect("at most four organ note-ons");
            }
        }
        let shape = self.voices[Part::Organ as usize].waveform;
        if previous.is_none_or(|p| p.voices[Part::Organ as usize].waveform != shape) {
            events
                .push(at(OrganMessage::Voicing(if shape == 0 {
                    Voicing::House
                } else {
                    Voicing::Analog
                })))
                .expect("one voicing change");
        }
        events
    }
}

/// The global mixer page edits one parameter on all four strips at once.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum MixerBank {
    #[default]
    Volume,
    Reverb,
    Delay,
}

impl MixerBank {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A bank index plus an i8 delta fits i16"
    )]
    pub(super) fn turn(&mut self, detents: i8) {
        *self = match (i16::from(*self as u8) + i16::from(detents)).rem_euclid(3) {
            0 => Self::Volume,
            1 => Self::Reverb,
            _ => Self::Delay,
        };
    }
}

/// Four track levels and effect sends persist across instrument selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Mixer {
    pub(super) volume: [u8; 4],
    pub(super) reverb: [u8; 4],
    pub(super) delay: [u8; 4],
}

impl Mixer {
    // Unity faders and silent sends provide the dry reference mix.
    pub(super) const DRY: Self = Self {
        volume: [128; 4],
        reverb: [0; 4],
        delay: [0; 4],
    };
    pub(super) const INITIAL: Self = Self {
        volume: [128, 128, 112, 80],
        reverb: [32, 36, 24, 0],
        delay: [0, 0, 92, 0],
    };

    pub(super) fn turn_knobs(&mut self, bank: MixerBank, detents: [i8; 4]) {
        let values = match bank {
            MixerBank::Volume => &mut self.volume,
            MixerBank::Reverb => &mut self.reverb,
            MixerBank::Delay => &mut self.delay,
        };
        for (value, delta) in values.iter_mut().zip(detents) {
            *value = value.saturating_add_signed(delta).min(128);
        }
    }

    fn events(self, previous: Option<Self>) -> EventBatch {
        let mut events = EventBatch::new();
        for track in 0..4 {
            let id = u8::try_from(track).expect("four mixer tracks");
            for (changed, message) in [
                (
                    previous.is_none_or(|old| old.volume[track] != self.volume[track]),
                    MixerMessage::Level {
                        track: id,
                        level: self.volume[track],
                    },
                ),
                (
                    previous.is_none_or(|old| old.reverb[track] != self.reverb[track]),
                    MixerMessage::Send {
                        track: id,
                        level: self.reverb[track],
                    },
                ),
                (
                    previous.is_none_or(|old| old.delay[track] != self.delay[track]),
                    MixerMessage::SendTo {
                        send: 1,
                        track: id,
                        level: self.delay[track],
                    },
                ),
            ] {
                if changed {
                    events
                        .push(at(message))
                        .expect("at most twelve mixer changes");
                }
            }
        }
        events
    }
}

// Instruments, mixer and global effects share one message vocabulary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Message {
    Mono(MonoMessage),
    Organ(OrganMessage),
    Bass(BassMessage),
    Mixer(MixerMessage),
    Fx(FxMessage),
    Drum(DrumMessage),
    DrumLamps([DrumLamp; 6]),
}

impl From<MonoMessage> for Message {
    fn from(value: MonoMessage) -> Self {
        Self::Mono(value)
    }
}
impl From<OrganMessage> for Message {
    fn from(value: OrganMessage) -> Self {
        Self::Organ(value)
    }
}
impl From<BassMessage> for Message {
    fn from(value: BassMessage) -> Self {
        Self::Bass(value)
    }
}
impl From<MixerMessage> for Message {
    fn from(value: MixerMessage) -> Self {
        Self::Mixer(value)
    }
}
impl From<FxMessage> for Message {
    fn from(value: FxMessage) -> Self {
        Self::Fx(value)
    }
}

pub(super) fn at(message: impl Into<Message>) -> Event<Message> {
    Event {
        frame: 0,
        port: 0,
        message: message.into(),
    }
}

// Four organ note-offs, four note-ons, four panel values and a voicing event.
const MAX_EVENTS: usize = 16;
pub(super) type EventBatch = Vec<Event<Message>, MAX_EVENTS>;
// At most one note/gate transition and two automation updates in a 128-frame
// DMA block, plus sixteen panel messages. Keep this storage off the IRQ stack.
const MAX_ROUTED_EVENTS: usize = 32;
type RoutedEvents = Vec<Event<Message>, MAX_ROUTED_EVENTS>;

pub(super) enum Node<'a> {
    Mono(Monosynth<'a>),
    Organ(&'a mut Organ),
    Bass(Tb303),
}

impl Node<'_> {
    pub(super) fn bass(rate: f32) -> Self {
        // Leave room for resonant filter peaks before Q23 conversion. Applying
        // the graph's former quarter-gain after conversion was already too late.
        Self::Bass(
            Tb303::new(rate)
                .with_output_gain(0.0625)
                .with_idle_suspension(),
        )
    }
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
            Self::Organ(node) => dispatch(*node, cx, |m| match m {
                Message::Organ(m) => Some(m),
                _ => None,
            }),
            Self::Bass(node) => dispatch(node, cx, |m| match m {
                Message::Bass(m) => Some(m),
                _ => None,
            }),
        }
    }
}

/// One dry sum and two independent, post-fader sends.
pub(super) struct FxMixer(pub(super) ChannelMixer<4, 3>);

impl Process<Message, I24> for FxMixer {
    fn inlets(&self) -> &[Port] {
        self.0.inlets()
    }

    fn outlets(&self) -> &[Port] {
        self.0.outlets()
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        dispatch(&mut self.0, cx, |m| match m {
            Message::Mixer(m) => Some(m),
            _ => None,
        });
    }
}

/// Adapt an effect processor to the firmware's message type.
pub(super) struct FxNode<T>(pub(super) T);

impl<T: Process<FxMessage, I24>> Process<Message, I24> for FxNode<T> {
    fn inlets(&self) -> &[Port] {
        self.0.inlets()
    }

    fn outlets(&self) -> &[Port] {
        self.0.outlets()
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        dispatch(&mut self.0, cx, |m| match m {
            Message::Fx(m) => Some(m),
            _ => None,
        });
    }
}

fn dispatch<M>(
    node: &mut impl Process<M, I24>,
    cx: &mut ProcessContext<'_, '_, Message, I24>,
    unpack: impl Fn(Message) -> Option<M>,
) {
    let events: Vec<_, MAX_ROUTED_EVENTS> = cx
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
        emit: &mut |e| Err(e), // The instrument nodes have only a signal outlet.
    });
}

pub(super) fn bass_ducker(rate_hz: u32) -> FxNode<SidechainDucker<5>> {
    FxNode(SidechainDucker::new(rate_hz, 32, 176).expect("valid 303 insert settings"))
}

const MASTER_BUS: usize = 12;
pub(super) const BUS_COUNT: usize = MASTER_BUS + 2;
impl Track {
    /// The four source calls produce buses in track order, before faders or sends.
    pub(super) fn scope_samples<const N: usize>(self, buses: &[[I24; N]; BUS_COUNT]) -> [i32; N] {
        let samples = &buses[self as usize];
        if self == Self::Drums {
            // A full-scale Q23 kit should fit the scope, including the accented kick.
            samples.map(|sample| sample.inner() >> 4)
        } else {
            samples.map(I24::inner)
        }
    }
}

/// Display telemetry from the same submitted audio block.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Playback {
    pub(super) position: Option<Position>,
    pub(super) drums: [DrumLamp; 6],
}

type Plan = Program<'static, 11, 5>;
pub(super) struct Programs([Plan; 2]);

impl Programs {
    pub(super) fn decode() -> Result<Self, Error> {
        // Four persistent sources feed a shared post-fader dry/send mixer.
        const ROUTE: &[u8] = &[
            VERSION, 1, 0, 0, 1, // CALL mono -> 0
            1, 1, 0, 1, // CALL organ -> 1
            1, 2, 0, 1, // CALL bass -> 2
            1, 5, 0, 2, // CALL sequenced 909 -> kit 3, isolated kick 4
            1, 7, 5, 1, 2, 4, 3, 1, 0, // CALL 303 insert(self, kick, kit, organ, mono) -> 5
            1, 3, 4, 3, 0, 1, 5, 3, // CALL mixer -> dry 6, reverb send 7, delay send 8
            1, 4, 1, 1, 7, // CALL reverb(7) -> 9
            1, 6, 1, 1, 8, // CALL delay(8) -> 10
            3, 6, 9, // MIX dry + reverb -> 11
            3, 11, 10, // MIX in delay -> master 12
            2, 12, 64, // GAIN optional headroom -> 13
        ];
        const FULL_LEN: usize = ROUTE.len() - 3;
        Ok(Self([
            Plan::decode(&ROUTE[..FULL_LEN])?,
            Plan::decode(ROUTE)?,
        ]))
    }

    pub(super) fn bind<'a>(
        &'a self,
        nodes: &'a mut [&'a mut dyn Process<Message, I24>],
        rate: (u32, u32),
    ) -> Result<Instrument<'a>, Error> {
        Ok(Instrument {
            programs: self,
            patch: self.0[0].bind(nodes)?,
            half_gain: false,
            mixer: Mixer::DRY,
            previous_mixer: None,
            fx: Fx::INITIAL,
            previous_fx: None,
            transport: Transport::INITIAL,
            tracks: crate::sequencer::Sequencer::INITIAL.enabled,
            player: PatternPlayer::new(rate),
            events: core::array::from_fn(|_| RoutedEvents::new()),
            automated: false,
            playback: Playback::default(),
            previous: None,
        })
    }
}

pub(super) struct Instrument<'a> {
    programs: &'a Programs,
    patch: Patch<'a, 'a, Message, I24, 5>,
    pub(super) half_gain: bool,
    mixer: Mixer,
    previous_mixer: Option<Mixer>,
    fx: Fx,
    previous_fx: Option<Fx>,
    pub(super) transport: Transport,
    pub(super) tracks: [bool; 4],
    player: PatternPlayer,
    events: [RoutedEvents; 4],
    automated: bool,
    pub(super) playback: Playback,
    previous: Option<Parts>,
}

impl Instrument<'_> {
    pub(super) fn set_mixer(&mut self, mixer: Mixer) {
        self.mixer = mixer;
    }

    pub(super) fn set_fx(&mut self, fx: Fx) {
        self.fx = fx;
    }

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
        let organ_events = parts.organ_events(self.previous);
        let old = self.previous.unwrap_or(Parts::INITIAL);
        let mut bass_events = parts
            .bass
            .events(old.bass, bass, old.voices[Part::Bass as usize]);
        let mut enabled = self.tracks;
        // Physical keys temporarily own their instrument. Release sequence gates
        // before live strikes; the next sequence strike resumes after key-up.
        for (track, held) in [
            (
                Track::Mono,
                parts.voices[Part::Mono as usize].gate.note.is_some(),
            ),
            (
                Track::Organ,
                parts.organ_keys.iter().any(|key| key.note.is_some()),
            ),
            (Track::Bass, bass.gate.note.is_some()),
        ] {
            enabled[track as usize] &= !held;
        }
        let automated = self.transport.playing() && enabled[Track::Bass as usize];
        if automated {
            bass_events.retain(|event| !is_automation(event.message));
        } else if self.automated {
            for message in parts.bass.automation_reset() {
                bass_events.push(at(message)).map_err(|_| Error::Capacity)?;
            }
        }
        for events in &mut self.events {
            events.clear();
        }
        for (track, panel) in [
            (Track::Mono, &mono_events),
            (Track::Organ, &organ_events),
            (Track::Bass, &bass_events),
        ] {
            self.events[track as usize]
                .extend_from_slice(panel)
                .map_err(|_| Error::Capacity)?;
        }
        if parts.pads != old.pads {
            for (i, voice) in Drum::ALL.into_iter().enumerate() {
                if parts.pads.0[i] != old.pads.0[i] {
                    self.events[Track::Drums as usize]
                        .push(at(Message::Drum(DrumMessage::Trigger(voice))))
                        .map_err(|_| Error::Capacity)?;
                }
            }
        }
        self.player
            .render(N, self.transport, enabled, |track, event| {
                self.events[track as usize]
                    .push(event)
                    .map_err(|_| Error::Capacity)
            })?;
        for events in &mut self.events {
            // Stable, bounded insertion sort. Same-frame key releases precede
            // sequence strikes, and sequence releases precede live key strikes.
            for i in 1..events.len() {
                let mut j = i;
                while let Some(previous) = j.checked_sub(1) {
                    if event_order(&events[j]) >= event_order(&events[previous]) {
                        break;
                    }
                    events.swap(j, previous);
                    j = previous;
                }
            }
        }
        let mixer_events = self.mixer.events(self.previous_mixer);
        let reverb = self.fx.events(self.previous_fx, FxPage::Reverb);
        let delay = self.fx.events(self.previous_fx, FxPage::Delay);
        let comp = self.fx.events(self.previous_fx, FxPage::BassComp);
        self.patch.render(
            &mut Software,
            buses,
            &[
                &self.events[Track::Mono as usize],
                &self.events[Track::Organ as usize],
                &self.events[Track::Bass as usize],
                &mixer_events,
                &reverb,
                &self.events[Track::Drums as usize],
                &delay,
                &comp,
            ],
            |_, event| match event.message {
                Message::DrumLamps(lamps) => {
                    self.playback.drums = lamps;
                    Ok(())
                }
                _ => Err(event),
            },
        )?;
        self.previous = Some(parts);
        self.previous_mixer = Some(self.mixer);
        self.previous_fx = Some(self.fx);
        self.automated = automated;
        self.playback.position = self.player.position();
        Ok(&buses[if self.half_gain {
            MASTER_BUS + 1
        } else {
            MASTER_BUS
        }])
    }
}

fn is_automation(message: Message) -> bool {
    matches!(
        message,
        Message::Bass(
            BassMessage::CutoffHz(_) | BassMessage::Resonance(_) | BassMessage::OverdriveDb(_)
        )
    )
}

fn event_order(event: &Event<Message>) -> (usize, u8) {
    let order = match event.message {
        Message::Mono(MonoMessage::NoteOff(_))
        | Message::Organ(OrganMessage::NoteOff(_))
        | Message::Bass(BassMessage::NoteOff(_) | BassMessage::Reset)
        | Message::Drum(DrumMessage::Stop) => 0,
        Message::Mono(MonoMessage::NoteOn { .. })
        | Message::Organ(OrganMessage::NoteOn { .. })
        | Message::Bass(BassMessage::NoteOn { .. })
        | Message::Drum(DrumMessage::Trigger(_)) => 2,
        _ => 1,
    };
    (event.frame, order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wavegraph_fx::{Delay, Reverb};
    use wavegraph_monosynth::Tuning;

    const BASS_INSERT: usize = 5;
    const DRY_BUS: usize = 6;
    const REVERB_SEND: usize = 7;
    const DELAY_SEND: usize = 8;
    const REVERB_RETURN: usize = 9;
    const DELAY_RETURN: usize = 10;
    const DRY_AND_REVERB: usize = 11;

    fn note(number: u8, velocity: u8) -> Note {
        Note { number, velocity }
    }

    fn with_instrument(
        mono_rate: u32,
        test: impl FnOnce(&mut Instrument<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let programs = Programs::decode()?;
        let tuning = Tuning::new(mono_rate, 1).expect("audio sample clock");
        let mut mono = Node::Mono(Monosynth::new(&tuning).with_idle_suspension());
        let mut organ_state = crate::organ::instrument(mono_rate, 1);
        let mut organ = Node::Organ(&mut organ_state);
        let mut bass = Node::bass(24_000_000.0 / 544.0);
        let mut mixer = FxMixer(ChannelMixer::new());
        let mut storage = [0_i32; wavegraph_fx::REVERB_SAMPLES];
        let mut reverb = FxNode(Reverb::new(&mut storage));
        let mut line = [0; 44_118];
        let mut delay = FxNode(Delay::new(&mut line, mono_rate).expect("delay sample clock"));
        let mut drums = crate::sequencer::DrumMachine::new((24_000_000, 544));
        let mut insert = bass_ducker(mono_rate);
        let mut nodes: [&mut dyn Process<Message, I24>; 8] = [
            &mut mono,
            &mut organ,
            &mut bass,
            &mut mixer,
            &mut reverb,
            &mut drums,
            &mut delay,
            &mut insert,
        ];
        test(&mut programs.bind(&mut nodes, (mono_rate, 1))?)
    }

    #[test]
    fn keys_keep_their_owner_with_three_parts_and_polyphonic_organ() {
        let mut keys = Keyboard::<5>::default();
        let (a, c, e, g, bass) = (
            note(69, 35),
            note(72, 113),
            note(76, 64),
            note(79, 100),
            note(55, 96),
        );
        keys.press(a);
        keys.press(c);
        let [mono, _, _] = keys.gates();
        assert_eq!(mono.note, Some(c));
        keys.focus = Part::Organ;
        keys.press(e);
        keys.press(g);
        assert_eq!(keys.organ_keys()[0].note, Some(g));
        assert_eq!(keys.organ_keys()[1].note, Some(e));
        keys.focus = Part::Bass;
        keys.press(bass);
        keys.release(c.number);
        assert_eq!(keys.gates().map(|g| g.note), [Some(a), Some(g), Some(bass)]);
        keys.release(g.number);
        assert_eq!(keys.organ_keys()[0].note, Some(e));
        assert_eq!(keys.gates()[1].note, Some(e));
        keys.release(e.number);
        keys.release(bass.number);
        keys.release(a.number);
        assert_eq!(keys.gates().map(|g| g.note), [None, None, None]);
        keys.focus = Part::Mono;
        keys.press(a);
        assert_ne!(keys.gates()[0].phrase, mono.phrase);
        keys.release(a.number);
    }

    #[test]
    fn octave_follows_the_keyboard_and_held_keys_keep_their_pitch_and_part() {
        let mut keys = Keyboard::<4>::default();
        keys.shift_octave(1);
        keys.press(note(53, 80)); // F3 plays F4 on the monosynth.
        keys.focus = Part::Organ;
        keys.press(note(57, 90)); // A3 plays A4 on the organ.
        keys.shift_octave(-1);
        keys.press(note(65, 95)); // F4 on the organ.
        keys.focus = Part::Bass;
        keys.shift_octave(-1);
        keys.press(note(77, 105)); // F5 plays F4 on the 303.
        assert_eq!(
            keys.gates().map(|gate| gate.note),
            [Some(note(65, 80)), Some(note(65, 95)), Some(note(65, 105))]
        );
        keys.release(65);
        assert_eq!(keys.organ_keys()[0].note, Some(note(69, 90)));
        keys.release(53);
        assert_eq!(
            keys.gates().map(|gate| gate.note),
            [None, Some(note(69, 90)), Some(note(65, 105))]
        );
        keys.release(77);
        keys.release(57);
        assert_eq!(keys.gates().map(|gate| gate.note), [None; 3]);
    }

    #[test]
    fn octave_limits_compose_with_select_transpose() {
        for (step, key, transpose, pitch, name) in
            [(-1, 54, -12, 6, *b"F#-1"), (1, 79, 12, 127, *b"G9  ")]
        {
            let mut keys = Keyboard::<1>::default();
            for _ in 0..5 {
                keys.shift_octave(step);
            }
            keys.press(note(key, 100));
            let mut parts = Parts::INITIAL;
            parts.turn_select(Part::Mono, transpose);
            let voice = &mut parts.voices[Part::Mono as usize];
            voice.gate = keys.gates()[Part::Mono as usize];
            assert_eq!(voice.sounding(), Some(note(pitch, 100)));
            assert_eq!(voice.note_name(), name);
        }
    }

    #[test]
    fn either_organ_key_can_sustain_a_shared_pitch() {
        for (first, last) in [(65, 53), (53, 65)] {
            let mut keys = Keyboard::<2> {
                focus: Part::Organ,
                ..Keyboard::default()
            };
            keys.press(note(65, 60));
            keys.shift_octave(1);
            keys.press(note(53, 100));
            let mut parts = Parts::INITIAL;
            parts.organ_keys = keys.organ_keys();
            let strike = |velocity| at(OrganMessage::NoteOn { note: 65, velocity });
            assert_eq!(
                parts.organ_events(Some(Parts::INITIAL)).as_slice(),
                &[strike(60), strike(100)]
            );
            let both = parts;
            keys.release(first);
            parts.organ_keys = keys.organ_keys();
            assert!(parts.organ_events(Some(both)).is_empty());
            let remaining = parts;
            keys.release(last);
            parts.organ_keys = keys.organ_keys();
            assert_eq!(
                parts.organ_events(Some(remaining)).as_slice(),
                &[at(OrganMessage::NoteOff(65))]
            );
        }
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
            assert_eq!(parts.voices[0].note_name(), *b"--  ");
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
                assert_eq!(mono.note_name(), *b"A5  ");
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
    fn kit_scope_and_lamps_follow_live_pads_with_transport_stopped_and_fader_down()
    -> Result<(), Error> {
        use DrumLamp::{Off, Strike, Tail};
        with_instrument(44_100, |instrument| {
            let mut parts = Parts::INITIAL;
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut mixer = Mixer::DRY;
            mixer.volume[Track::Drums as usize] = 0;
            instrument.set_mixer(mixer);
            parts.pads.press(1); // Black key, outside the six pads.
            instrument.render(parts, &mut buses)?;
            assert_eq!(instrument.playback.drums, [Off; 6]);

            for key in [0, 2, 4, 6, 7, 9] {
                parts.pads.press(key);
            }
            assert_eq!(
                *instrument.render(parts, &mut buses)?,
                [I24::default(); 128]
            );
            assert!(Track::Drums.scope_samples(&buses).iter().any(|&s| s != 0));
            assert_eq!(
                instrument.playback.drums,
                [Strike, Strike, Strike, Off, Strike, Strike]
            );
            for _ in 0..24 {
                instrument.render(parts, &mut buses)?;
            }
            assert_eq!(
                instrument.playback.drums,
                [Tail, Tail, Tail, Off, Tail, Tail]
            );
            parts.pads.press(6); // Closed hat immediately chokes open hat.
            parts.pads.press(4); // Retrigger an already-ringing clap.
            instrument.render(parts, &mut buses)?;
            assert_eq!(
                instrument.playback.drums,
                [Tail, Tail, Strike, Strike, Off, Tail]
            );
            for _ in 0..350 {
                instrument.render(parts, &mut buses)?;
            }
            assert_eq!(instrument.playback.drums, [Off; 6]);
            assert_eq!(Track::Drums.scope_samples(&buses), [0; 128]);
            Ok(())
        })
    }

    #[test]
    fn transport_plays_the_drum_bus_alongside_a_held_synth_note() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut parts = Parts::INITIAL;
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            instrument.tracks = [false, false, false, true];
            let silence = instrument.render(parts, &mut buses)?.map(I24::inner);
            assert_eq!(silence, [0; 128]);
            assert_eq!(instrument.playback.position, None);
            instrument.transport.toggle();
            let beat = *instrument.render(parts, &mut buses)?;
            assert_eq!(
                instrument
                    .playback
                    .position
                    .map(|position| position.beat_phase),
                Some(1)
            );
            assert_eq!(beat, buses[3]);
            assert!(beat.iter().any(|s| s.inner() != 0));
            use DrumLamp::{Off, Strike};
            assert_eq!(instrument.playback.drums, [Strike, Off, Off, Off, Off, Off]);
            parts.voices[0].gate.note = Some(note(69, 96));
            instrument.render(parts, &mut buses)?;
            assert_ne!(buses[MASTER_BUS], buses[3]);
            instrument.transport.toggle();
            let tail = *instrument.render(parts, &mut buses)?;
            assert_eq!(instrument.playback.position, None);
            assert_eq!(buses[3], [I24::default(); 128]);
            assert_eq!(instrument.playback.drums, [Off; 6]);
            assert_eq!(tail, buses[DRY_BUS]);
            assert!(tail.iter().any(|s| s.inner() != 0));
            Ok(())
        })
    }

    #[test]
    fn live_bass_takeover_preserves_both_sends_and_stop_preserves_the_live_key() -> Result<(), Error>
    {
        with_instrument(44_100, |instrument| {
            let mut parts = Parts::DEEP_HOUSE;
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut mixer = Mixer::DRY;
            mixer.volume[Track::Bass as usize] = 64;
            mixer.reverb[Track::Bass as usize] = 128;
            mixer.delay[Track::Bass as usize] = 64;
            instrument.set_mixer(mixer);
            let check_sends = |buses: &[[I24; 128]; BUS_COUNT]| {
                assert_eq!(
                    buses[DRY_BUS].map(I24::inner),
                    buses[BASS_INSERT].map(|sample| sample.inner() >> 1)
                );
                assert_eq!(buses[REVERB_SEND], buses[DRY_BUS]);
                assert_eq!(
                    buses[DELAY_SEND].map(I24::inner),
                    buses[DRY_BUS].map(|sample| sample.inner() >> 1)
                );
            };
            instrument.tracks = [false, false, true, false];
            instrument.transport.toggle();
            instrument.render(parts, &mut buses)?;
            assert!(instrument.automated);
            check_sends(&buses);
            let pitch = instrument.events[Track::Bass as usize]
                .iter()
                .find_map(|event| match event.message {
                    Message::Bass(BassMessage::NoteOn { note, .. }) => Some(note),
                    _ => None,
                })
                .expect("the factory bass starts on the downbeat");

            parts.bass.turn_knobs([2, -3, 0, 0]);
            parts.voices[Part::Bass as usize].gate = Gate {
                note: Some(note(pitch, 110)),
                phrase: 1,
            };
            instrument.render(parts, &mut buses)?;
            assert!(!instrument.automated);
            check_sends(&buses);
            let events = &instrument.events[Track::Bass as usize];
            assert_eq!(
                events[0].message,
                Message::Bass(BassMessage::NoteOff(pitch))
            );
            assert!(matches!(events.last().expect("live strike").message,
                Message::Bass(BassMessage::NoteOn { note, slide: false, .. }) if note == pitch));
            for message in parts.bass.automation_reset() {
                assert!(
                    events
                        .iter()
                        .any(|event| event.message == Message::Bass(message))
                );
            }

            instrument.transport.toggle();
            // The fader silences dry and both sends without stopping a held
            // live voice. Its earlier effect returns retain their own tails.
            mixer.volume[Track::Bass as usize] = 0;
            instrument.set_mixer(mixer);
            instrument.render(parts, &mut buses)?;
            assert!(instrument.events[Track::Bass as usize].is_empty());
            assert!(
                buses[DRY_BUS..=DELAY_SEND]
                    .iter()
                    .flatten()
                    .all(|sample| sample.inner() == 0)
            );
            assert!(
                buses[Track::Bass as usize]
                    .iter()
                    .any(|sample| sample.inner() != 0)
            );
            parts.voices[Part::Bass as usize].gate.note = None;
            instrument.render(parts, &mut buses)?;
            assert_eq!(
                instrument.events[Track::Bass as usize].as_slice(),
                &[at(BassMessage::NoteOff(pitch))]
            );
            Ok(())
        })
    }

    #[test]
    fn muting_the_bass_restores_edited_panel_values_without_stopping_other_tracks()
    -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut parts = Parts::DEEP_HOUSE;
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            instrument.transport.toggle();
            instrument.render(parts, &mut buses)?;
            parts.bass.turn_knobs([4, -6, 0, 0]);
            instrument.render(parts, &mut buses)?;
            assert!(instrument.automated);
            instrument.tracks[Track::Bass as usize] = false;
            instrument.render(parts, &mut buses)?;
            assert!(!instrument.automated);
            assert!(instrument.playback.position.is_some());
            for message in parts.bass.automation_reset() {
                assert!(
                    instrument.events[Track::Bass as usize]
                        .iter()
                        .any(|event| event.message == Message::Bass(message))
                );
            }
            assert!(buses[3].iter().any(|sample| sample.inner() != 0));
            Ok(())
        })
    }

    #[test]
    fn changing_routes_preserves_the_sounding_oscillators_phase() -> Result<(), Error> {
        with_instrument(1760, |instrument| {
            let mut buses = [[I24::default(); 3]; BUS_COUNT];
            let mut parts = Parts::INITIAL;
            parts.voices[0].gate.note = Some(note(69, 80));
            assert_eq!(parts.voices[0].note_name(), *b"A4  ");
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
            assert_eq!(buses[Track::Bass as usize].map(I24::inner), [0; 128]);
            keys.focus = Part::Bass;
            keys.press(note(53, 110));
            sync(&mut parts, &keys);
            for half in [false, true] {
                instrument.set_half_gain(half)?;
                let mix = instrument.render(parts, &mut buses)?.map(I24::inner);
                let mono = &buses[Track::Mono as usize];
                let bass = &buses[BASS_INSERT];
                assert!(mono.iter().any(|s| s.inner() != 0));
                assert!(bass.iter().any(|s| s.inner() != 0));
                for i in 0..mix.len() {
                    let bass_fader = bass[i].inner();
                    let dry = (mono[i].inner() + bass_fader).clamp(-(1 << 23), (1 << 23) - 1);
                    assert_eq!(mix[i], dry >> u8::from(half));
                }
            }
            keys.release(69);
            sync(&mut parts, &keys);
            instrument.render(parts, &mut buses)?;
            assert!(buses[Track::Mono as usize].iter().any(|s| s.inner() != 0));
            // The mono's 125 ms release continues while the 303 key is held.
            for _ in 0..44 {
                instrument.render(parts, &mut buses)?;
            }
            assert_eq!(buses[Track::Mono as usize].map(I24::inner), [0; 128]);
            assert!(buses[Track::Bass as usize].iter().any(|s| s.inner() != 0));
            keys.focus = Part::Mono;
            keys.release(53);
            sync(&mut parts, &keys);
            instrument.render(parts, &mut buses)?;
            assert!(buses[Track::Bass as usize].iter().any(|s| s.inner() != 0));
            for _ in 0..60 {
                instrument.render(parts, &mut buses)?;
            }
            assert!(
                buses[Track::Bass as usize]
                    .iter()
                    .all(|s| s.inner().abs() <= 1)
            );
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
        parts.voices = [voice; 3];
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
        assert!(parts.bass.events(filter, parts.voices[2], voice).is_empty());
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
        assert_eq!(parts.voices[2], voice);
        parts.turn_select(Part::Bass, 1);
        assert_eq!(parts.bass, edited);
    }

    #[test]
    fn shared_reverb_has_independent_sends_and_preserves_dry_audio() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut parts = Parts::INITIAL;
            parts.voices[0].gate.note = Some(note(69, 110));
            instrument.set_mixer(Mixer::INITIAL);
            instrument.set_fx(Fx::INITIAL);
            let mut audible_wet = false;
            for _ in 0..30 {
                instrument.render(parts, &mut buses)?;
                audible_wet |= buses[REVERB_RETURN]
                    .iter()
                    .any(|sample| sample.inner() != 0);
            }
            assert!(audible_wet, "shared send must reach the reverb tank");
            let mut fx = Fx::INITIAL;
            fx.reverb.enabled = false;
            instrument.set_fx(fx);
            instrument.render(parts, &mut buses)?;
            assert!(
                buses[REVERB_RETURN]
                    .iter()
                    .all(|sample| sample.inner() == 0)
            );
            assert_eq!(
                buses[MASTER_BUS], buses[DRY_BUS],
                "muting FX preserves dry output"
            );
            Ok(())
        })
    }

    #[test]
    fn mixer_controls_all_four_strips_and_sends_only_live_tracks() {
        let mut mixer = Mixer::INITIAL;
        let mut bank = MixerBank::Volume;
        mixer.turn_knobs(bank, [i8::MIN, 1, 8, -1]);
        assert_eq!(mixer.volume, [0, 128, 120, 79]);
        bank.turn(1);
        assert_eq!(bank, MixerBank::Reverb);
        mixer.turn_knobs(bank, [0, 12, 0, 127]);
        assert_eq!(mixer.reverb, [32, 48, 24, 127]);
        assert_eq!(mixer.events(None).len(), 12);
        let initial = mixer;
        mixer.turn_knobs(bank, [0, 0, 0, 1]);
        assert_eq!(
            mixer.events(Some(initial)).as_slice(),
            &[at(MixerMessage::Send {
                track: 3,
                level: 128
            })]
        );
        bank.turn(-1);
        assert_eq!(bank, MixerBank::Volume);
        bank.turn(-1);
        assert_eq!(bank, MixerBank::Delay);
        let original = mixer;
        mixer.turn_knobs(bank, [2, 3, 4, 5]);
        assert_eq!(mixer.delay, [2, 3, 96, 5]);
        assert_eq!(mixer.reverb, original.reverb);
        assert_eq!(mixer.volume, original.volume);
        assert_eq!(mixer.events(Some(original)).len(), 4);
        bank.turn(i8::MIN);
        bank.turn(i8::MAX);
        assert_eq!(bank, MixerBank::Reverb);
    }

    #[test]
    fn compressor_panel_selects_the_named_pre_fader_signal() -> Result<(), Error> {
        use crate::fx::KeySource::{Bass, Kick, Kit, Mono, Organ as OrganKeySource};
        for (turn, key, detector_bus) in [
            (-1, Bass, 2),
            (0, Kick, 4),
            (1, Kit, 3),
            (2, OrganKeySource, 1),
            (3, Mono, 0),
        ] {
            with_instrument(44_118, |instrument| {
                let mut fx = Fx::INITIAL;
                let mut page = FxPage::Reverb;
                page.turn(-1);
                assert_eq!(page, FxPage::BassComp);
                fx.edit(page, [turn, 25, 4, 0], false);
                assert_eq!(fx.key, key);
                assert_eq!((fx.depth, fx.split_hz), (100, 200));
                assert!(fx.enabled(page));
                instrument.set_fx(fx);
                let mut reference =
                    SidechainDucker::<2>::new(44_118, 32, 176).expect("valid insert");
                let mut buses = [[I24::default(); 128]; BUS_COUNT];
                let mut parts = Parts::DEEP_HOUSE;
                parts.voices[0].gate.note = Some(note(72, 120));
                let mut mixer = Mixer::DRY;
                mixer.volume = [0, 0, 128, 0];
                instrument.set_mixer(mixer);
                instrument.transport.toggle();
                let mut detector_sounded = false;
                for block in 0..220 {
                    instrument.render(parts, &mut buses)?;
                    detector_sounded |= buses[detector_bus].iter().any(|s| s.inner() != 0);
                    let set_corner = [Event {
                        frame: 0,
                        port: 0,
                        message: FxMessage::SidechainHz(200),
                    }];
                    let mut expected = [I24::default(); 128];
                    reference.process(&mut ProcessContext {
                        frames: 128,
                        inputs: &[&buses[2], &buses[detector_bus]],
                        outputs: &mut [&mut expected],
                        events: if block == 0 { &set_corner } else { &[] },
                        emit: &mut Err,
                    });
                    assert_eq!(buses[BASS_INSERT], expected);
                    assert_eq!(buses[DRY_BUS], expected);
                }
                assert!(detector_sounded);
                fx.edit(page, [0; 4], true);
                assert!(!fx.enabled(page));
                instrument.set_fx(fx);
                instrument.render(parts, &mut buses)?;
                assert_eq!(&buses[BASS_INSERT][64..], &buses[2][64..]);
                Ok(())
            })?;
        }
        Ok(())
    }

    #[test]
    fn factory_chords_hold_their_place_beside_the_kit_with_headroom() -> Result<(), Error> {
        with_instrument(44_118, |instrument| {
            instrument.set_mixer(Mixer::INITIAL);
            instrument.transport.toggle();
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut chords = 0_i64;
            let mut drums = 0_i64;
            let mut peak = 0;
            for _ in 0..2713 {
                // Four bars at 122 BPM, rounded up to a DMA block.
                instrument.render(Parts::DEEP_HOUSE, &mut buses)?;
                for frame in 0..128 {
                    let post_fader = |bus: usize, track: usize| {
                        i64::from(
                            (buses[bus][frame].inner() * i32::from(Mixer::INITIAL.volume[track]))
                                >> 16,
                        )
                    };
                    chords += post_fader(1, 1).pow(2);
                    drums += post_fader(3, 3).pow(2);
                    peak = peak.max(buses[MASTER_BUS][frame].inner().abs());
                }
            }
            assert!(
                drums > 0 && chords * 4 > drums,
                "chords within 6 dB RMS of the kit"
            );
            assert!(peak < 8_000_000, "headroom on the full-gain master");
            Ok(())
        })
    }

    #[test]
    fn delay_returns_after_the_drum_fader_goes_down_and_the_graph_changes() -> Result<(), Error> {
        // Five milliseconds at 32 kHz is exactly one test block.
        with_instrument(32_000, |instrument| {
            let mut buses = [[I24::default(); 160]; BUS_COUNT];
            let mut parts = Parts::INITIAL;
            parts.voices[0].gate.note = Some(note(69, 100));
            let mut mixer = Mixer::DRY;
            mixer.reverb[0] = 128;
            mixer.delay[3] = 128;
            instrument.tracks = [false, false, false, true];
            let mut fx = Fx::INITIAL;
            let mut page = FxPage::Reverb;
            page.turn(1);
            fx.edit(page, [-74, 15, 64, 0], false);
            assert_eq!((fx.time_ms, fx.feedback, fx.delay.wet), (5, 50, 128));
            assert_eq!(fx.reverb, Fx::INITIAL.reverb);
            instrument.set_mixer(mixer);
            instrument.set_fx(fx);
            instrument.transport.toggle();
            instrument.render(parts, &mut buses)?;
            assert_eq!(
                (buses[REVERB_SEND], buses[DELAY_SEND]),
                (buses[0], buses[3])
            );
            assert_eq!(buses[DELAY_RETURN], [I24::default(); 160]);
            let echo = buses[3].map(|s| (s.inner() / 256) << 8);
            assert!(echo.iter().any(|&s| s != 0));

            mixer.volume[3] = 0;
            instrument.set_mixer(mixer);
            instrument.set_half_gain(true)?;
            let master = instrument.render(parts, &mut buses)?.map(I24::inner);
            assert_eq!(buses[DELAY_SEND], [I24::default(); 160]);
            assert_eq!(buses[DELAY_RETURN].map(I24::inner), echo);
            for (i, sample) in master.into_iter().enumerate() {
                assert_eq!(
                    sample,
                    (buses[DRY_AND_REVERB][i].inner() + echo[i]).clamp(-(1 << 23), (1 << 23) - 1)
                        >> 1
                );
            }
            fx.edit(page, [0; 4], true);
            instrument.set_fx(fx);
            instrument.render(parts, &mut buses)?;
            assert!(buses[DELAY_RETURN].iter().all(|s| s.inner() == 0));
            fx.edit(page, [0; 4], true);
            instrument.set_fx(fx);
            instrument.render(parts, &mut buses)?;
            assert_eq!(
                buses[DELAY_RETURN].map(I24::inner),
                echo.map(|s| (s / 1024) << 8)
            );
            // Both returns are now audible from different tracks.
            for _ in 0..3 {
                instrument.render(parts, &mut buses)?;
            }
            assert!(buses[REVERB_RETURN].iter().any(|s| s.inner() != 0));
            assert!(buses[DELAY_RETURN].iter().any(|s| s.inner() != 0));
            Ok(())
        })
    }

    #[test]
    fn organ_chord_stays_live_while_bass_is_played() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut parts = Parts::INITIAL;
            let mut keys = Keyboard::<6> {
                focus: Part::Organ,
                ..Keyboard::default()
            };
            for pitch in [60, 64, 67, 71] {
                keys.press(note(pitch, 110));
            }
            parts.organ_keys = keys.organ_keys();
            instrument.render(parts, &mut buses)?;
            assert!(buses[Track::Organ as usize].iter().any(|x| x.inner() != 0));
            assert_eq!(buses[Track::Bass as usize].map(I24::inner), [0; 128]);
            // Fifth key displaces the oldest, and releasing it brings back the oldest.
            keys.press(note(74, 95));
            parts.organ_keys = keys.organ_keys();
            instrument.render(parts, &mut buses)?;
            assert_eq!(parts.organ_keys[0].note, Some(note(74, 95)));
            keys.release(74);
            parts.organ_keys = keys.organ_keys();
            assert_eq!(parts.organ_keys[3].note, Some(note(60, 110)));
            instrument.render(parts, &mut buses)?;
            keys.focus = Part::Bass;
            keys.press(note(53, 100));
            parts.voices[Part::Bass as usize].gate = keys.gates()[Part::Bass as usize];
            instrument.render(parts, &mut buses)?;
            assert!(buses[Track::Organ as usize].iter().any(|x| x.inner() != 0));
            assert!(buses[Track::Bass as usize].iter().any(|x| x.inner() != 0));
            for pitch in [60, 64, 67, 71] {
                keys.release(pitch);
            }
            parts.organ_keys = keys.organ_keys();
            instrument.render(parts, &mut buses)?;
            assert_eq!(parts.organ_keys, [OrganKey::EMPTY; POLYPHONY]);
            Ok(())
        })
    }

    #[test]
    fn bass_legato_retrigger_and_panel_changes() {
        let mut controls = Controls::INITIAL;
        let mut keys = Keyboard::<2> {
            focus: Part::Bass,
            ..Keyboard::default()
        };
        let mut bass = Voice::INITIAL;
        let mut old = bass;
        keys.press(note(53, 95));
        bass.gate = keys.gates()[Part::Bass as usize];
        assert_eq!(
            &*controls.events(controls, bass, old),
            &[at(BassMessage::NoteOn {
                note: 53,
                accent: false,
                slide: false,
            })]
        );
        old = bass;
        keys.shift_octave(1);
        keys.press(note(60, 96));
        bass.gate = keys.gates()[Part::Bass as usize];
        assert_eq!(
            &*controls.events(controls, bass, old),
            &[at(BassMessage::NoteOn {
                note: 72,
                accent: true,
                slide: true,
            })]
        );
        old = bass;
        keys.release(60);
        keys.release(53);
        keys.shift_octave(-1);
        keys.press(note(60, 96));
        bass.gate = keys.gates()[Part::Bass as usize];
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
