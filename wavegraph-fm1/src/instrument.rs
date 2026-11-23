//! Four assignable instrument nodes feed the shared effect send/return graph.

#[cfg(test)]
use crate::panel::Navigation;
use crate::{
    bass::Controls,
    envelope::EnvelopeControls,
    fx::{Fx, FxPage},
    keys::HitLog,
    mixer::Mixer,
    organ::{Controls as OrganControls, OrganKey},
    panel::{Assignment, CHANNELS, Channel},
    sequencer::{
        DrumKits, DrumLamp, DrumMachine, DrumMessage, PatternPlayer, Position, Sequencer, Transport,
    },
};
use heapless::Vec;
use wavegraph::{
    Event, Port, Process, ProcessContext,
    bytecode::{Error, Patch, Program, VERSION},
    dsp::Software,
};
use wavegraph_303::{Message as BassMessage, Tb303};
use wavegraph_909::Voice as Drum;
use wavegraph_fx::{ChannelMixer, Message as FxMessage, MixerMessage};
use wavegraph_monosynth::{Message as MonoMessage, Monosynth, Tuning, Waveform};
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
    Tr909,
    Tr808,
}

impl Part {
    pub(super) const ALL: [Self; 5] = [
        Self::Mono,
        Self::Organ,
        Self::Bass,
        Self::Tr909,
        Self::Tr808,
    ];

    pub(super) const fn is_kit(self) -> bool {
        matches!(self, Self::Tr808 | Self::Tr909)
    }

    pub(super) const fn pads(self) -> &'static [Drum] {
        match self {
            Self::Tr808 => &[Drum::Kick, Drum::Snare, Drum::ClosedHat, Drum::OpenHat],
            Self::Tr909 => &Drum::ALL,
            _ => &[],
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A part index plus an i8 fits i16"
    )]
    pub(super) fn turn(&mut self, detents: i8) {
        let index = (i16::from(*self as u8) + i16::from(detents))
            .rem_euclid(i16::try_from(Self::ALL.len()).expect("small catalog"));
        *self = Self::ALL[usize::try_from(index).expect("wrapped instrument index")];
    }

    #[cfg(target_os = "none")]
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Mono => "MONOSYNTH",
            Self::Organ => "M1 ORGAN",
            Self::Bass => "TB-303",
            Self::Tr909 => "TR-909",
            Self::Tr808 => "TR-808",
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
            Part::Tr808 | Part::Tr909 => 0,
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
            Part::Tr909 => b"TR-909  ",
            Part::Tr808 => b"TR-808  ",
        }
    }

    pub(super) fn sounding(self) -> Option<Note> {
        self.gate.note.map(|mut note| {
            note.number = note.number.saturating_add_signed(self.transpose).min(127);
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Track {
    pub(super) assignment: Assignment,
    pub(super) voice: Voice,
    pub(super) bass: Controls,
    pub(super) organ: OrganControls,
    pub(super) organ_keys: [OrganKey; POLYPHONY],
    pub(super) mono_envelope: EnvelopeControls,
    pub(super) voice_tap: bool,
    pub(super) drums808: wavegraph_808::Controls,
    pub(super) panel_hits: HitLog<8>,
    pub(super) midi_hits: HitLog<32>,
}

impl Track {
    pub(super) const fn new(assignment: Assignment) -> Self {
        Self {
            assignment,
            voice: Voice::INITIAL,
            bass: Controls::INITIAL,
            organ: OrganControls::INITIAL,
            organ_keys: [OrganKey::EMPTY; POLYPHONY],
            mono_envelope: EnvelopeControls::INITIAL,
            voice_tap: assignment.part.is_kit(),
            drums808: wavegraph_808::Controls::INITIAL,
            panel_hits: HitLog::EMPTY,
            midi_hits: HitLog::EMPTY,
        }
    }

    #[cfg(test)]
    pub(super) const TEST_TRACKS: [Self; CHANNELS] = {
        let a = Navigation::SYNTH_RACK.assignments;
        [
            Self::new(a[0]),
            Self::new(a[1]),
            Self::new(a[2]),
            Self::new(a[3]),
        ]
    };

    pub(super) const fn rack(a: [Assignment; CHANNELS]) -> [Self; CHANNELS] {
        [
            Self::factory(Channel::One, a[0]),
            Self::factory(Channel::Two, a[1]),
            Self::factory(Channel::Three, a[2]),
            Self::factory(Channel::Four, a[3]),
        ]
    }

    pub(super) const fn factory(channel: Channel, assignment: Assignment) -> Self {
        let harmony = matches!((channel, assignment.part), (Channel::Two, Part::Bass));
        Self {
            bass: if harmony {
                Controls::HARMONY
            } else {
                Controls::DEEP_HOUSE
            },
            voice: Voice {
                waveform: harmony as u8,
                ..Voice::INITIAL
            },
            organ: OrganControls::DEEP_HOUSE,
            ..Self::new(assignment)
        }
    }

    pub(super) fn turn_select(&mut self, detents: i8) {
        match self.assignment.part {
            Part::Mono | Part::Organ => {
                self.voice.transpose = self.voice.transpose.saturating_add(detents).clamp(-12, 12);
            }
            Part::Bass => self.bass.turn_select(detents),
            Part::Tr808 | Part::Tr909 => (),
        }
    }

    pub(super) fn turn_knobs(&mut self, detents: [i8; 4]) {
        match self.assignment.part {
            Part::Mono => self.voice.turn_cutoff(detents[0]),
            Part::Organ => self.organ.turn_knobs(detents),
            Part::Bass => self.bass.turn_knobs(detents),
            Part::Tr808 => {
                let c = &mut self.drums808;
                for (value, delta) in [
                    &mut c.kick_tone,
                    &mut c.kick_decay,
                    &mut c.snare_tone,
                    &mut c.snappy,
                ]
                .into_iter()
                .zip(detents)
                {
                    *value = value.saturating_add_signed(delta).min(127);
                }
            }
            Part::Tr909 => (),
        }
    }

    fn events(self, old: Option<Self>) -> Result<EventBatch, Error> {
        let mut events = match self.assignment.part {
            Part::Mono => self.mono_events(old),
            Part::Organ => self.organ_events(old),
            Part::Bass => {
                let old = old.unwrap_or_else(|| Self::new(self.assignment));
                self.bass.events(old.bass, self.voice, old.voice)
            }
            Part::Tr808 | Part::Tr909 => {
                let mut events = EventBatch::new();
                if self.assignment.part == Part::Tr808
                    && old.is_none_or(|t| t.drums808 != self.drums808)
                {
                    events
                        .push(at(Message::Drum(DrumMessage::Controls808(self.drums808))))
                        .map_err(|_| Error::Capacity)?;
                }
                events
            }
        };
        if old.is_none() {
            events
                .insert(0, at(Message::Assign(self.assignment)))
                .map_err(|_| Error::Capacity)?;
        }
        Ok(events)
    }

    fn mono_events(self, previous: Option<Self>) -> EventBatch {
        let mono = self.voice;
        let old = previous.map(|track| track.voice);
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
        let old_transpose = previous.map_or(0, |p| p.voice.transpose);
        let now_transpose = self.voice.transpose;
        let sounding = |key: OrganKey, transpose: i8| {
            key.note.map(|mut note| {
                note.number = note.number.saturating_add_signed(transpose).min(127);
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
        let shape = self.voice.waveform;
        if previous.is_none_or(|p| p.voice.waveform != shape) {
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

// Instruments, mixer and global effects share one message vocabulary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Message {
    Mono(MonoMessage),
    Organ(OrganMessage),
    Bass(BassMessage),
    Mixer(MixerMessage),
    Fx(FxMessage),
    Assign(Assignment),
    Drum(DrumMessage),
    DrumLamps([DrumLamp; 6]),
    Insert(Channel, Part),
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
// A drum channel can receive 32 USB strikes and 8 panel strikes along with
// controls and a sequence step. The four routed queues live off the IRQ stack.
const MAX_ROUTED_EVENTS: usize = 48;
pub(super) type RoutedEvents = Vec<Event<Message>, MAX_ROUTED_EVENTS>;

// Only the assigned engine occupies mutable storage on each channel.
#[allow(
    clippy::large_enum_variant,
    reason = "All engines use inline storage on the FM-1"
)]
enum Player<'a> {
    Empty,
    Mono(Monosynth<'a>),
    Organ(Organ),
    Bass(Tb303),
    Drums(DrumMachine),
}

pub(super) struct Node<'a> {
    tuning: &'a Tuning,
    organ: &'a Organ,
    drums: &'a DrumKits,
    sample_rate: f32,
    player: Player<'a>,
}

impl<'a> Node<'a> {
    pub(super) const fn new(
        tuning: &'a Tuning,
        organ: &'a Organ,
        drums: &'a DrumKits,
        sample_rate: f32,
    ) -> Self {
        Self {
            tuning,
            organ,
            drums,
            sample_rate,
            player: Player::Empty,
        }
    }
}

impl Process<Message, I24> for Node<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal, Port::Signal, Port::Message]
    }
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        if let Some(Event {
            message: Message::Assign(assignment),
            ..
        }) = cx.events.first()
        {
            self.player = match assignment.part {
                Part::Mono => Player::Mono(Monosynth::new(self.tuning).with_idle_suspension()),
                Part::Organ => Player::Organ(self.organ.clone()),
                // Reserve headroom before the resonant signal is converted to Q23.
                Part::Bass => Player::Bass(
                    Tb303::new(self.sample_rate)
                        .with_output_gain(0.0625)
                        .with_idle_suspension(),
                ),
                part @ (Part::Tr909 | Part::Tr808) => Player::Drums(self.drums.assign(part)),
            };
        }
        if let Player::Drums(node) = &mut self.player {
            return node.process(cx);
        }
        let (signal, kick) = cx.outputs.split_at_mut(1);
        kick[0].fill(I24::default());
        let cx = &mut ProcessContext {
            frames: cx.frames,
            inputs: cx.inputs,
            outputs: signal,
            events: cx.events,
            emit: cx.emit,
        };
        match &mut self.player {
            Player::Empty => cx.outputs[0].fill(I24::default()),
            Player::Mono(node) => dispatch(node, cx, |m| match m {
                Message::Mono(m) => Some(m),
                _ => None,
            }),
            Player::Organ(node) => dispatch(node, cx, |m| match m {
                Message::Organ(m) => Some(m),
                _ => None,
            }),
            Player::Bass(node) => dispatch(node, cx, |m| match m {
                Message::Bass(m) => Some(m),
                _ => None,
            }),
            Player::Drums(_) => unreachable!("kit rendered both outlets"),
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
    // Only pitched synths and effects use this adapter.
    let events: Vec<_, 32> = cx
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

/// The contiguous, post-insert signals immediately upstream of the faders.
pub(super) const STRIP_BUS: usize = 8;
const MASTER_BUS: usize = 19;
pub(super) const BUS_COUNT: usize = MASTER_BUS + 2;
impl Channel {
    pub(super) fn scope_samples<const N: usize>(
        self,
        part: Part,
        buses: &[[I24; N]; BUS_COUNT],
    ) -> [i32; N] {
        // Leave room on the scope for the full-scale accented kick.
        let shift = match part {
            Part::Tr909 => 4,
            Part::Tr808 => 3,
            _ => 0,
        };
        buses[self.source_bus()].map(|sample| sample.inner() >> shift)
    }
}

type Plan = Program<'static, 11, 8>;
pub(super) struct Programs([Plan; 2]);

impl Programs {
    pub(super) fn decode() -> Result<Self, Error> {
        // Each channel provides a pre-fader scope tap and an isolated drum kick.
        const ROUTE: &[u8] = &[
            VERSION, 1, 0, 0, 2, // CALL channel 1 -> 0, 1
            1, 1, 0, 2, // CALL channel 2 -> 2, 3
            1, 2, 0, 2, // CALL channel 3 -> 4, 5
            1, 3, 0, 2, // CALL channel 4 -> 6, 7
            1, 7, 8, 5, 0, 1, 2, 3, 4, 5, 6, 7, // CALL inserts -> strips 8..11, key 12
            1, 4, 4, 3, 8, 9, 10, 11, // CALL mixer -> dry 13, reverb send 14, delay send 15
            1, 5, 1, 1, 14, // CALL reverb -> 16
            1, 6, 1, 1, 15, // CALL delay -> 17
            3, 13, 16, // MIX dry + reverb -> 18
            3, 18, 17, // MIX in delay -> master 19
            2, 19, 64, // GAIN optional headroom -> 20
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
        events: &'a mut [RoutedEvents; CHANNELS],
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
            sequences: Sequencer::INITIAL.enabled,
            player: PatternPlayer::new(rate),
            events,
            automated: [false; CHANNELS],
            position: None,
            drums: [[DrumLamp::Off; 6]; CHANNELS],
            previous: None,
        })
    }
}

pub(super) struct Instrument<'a> {
    programs: &'a Programs,
    patch: Patch<'a, 'a, Message, I24, 8>,
    pub(super) half_gain: bool,
    mixer: Mixer,
    previous_mixer: Option<Mixer>,
    fx: Fx,
    previous_fx: Option<Fx>,
    pub(super) transport: Transport,
    pub(super) sequences: [bool; CHANNELS],
    player: PatternPlayer,
    events: &'a mut [RoutedEvents; CHANNELS],
    automated: [bool; CHANNELS],
    pub(super) position: Option<Position>,
    pub(super) drums: [[DrumLamp; 6]; CHANNELS],
    previous: Option<[Track; CHANNELS]>,
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
        tracks: [Track; CHANNELS],
        buses: &'b mut [[I24; N]; BUS_COUNT],
    ) -> Result<&'b [I24; N], Error> {
        let assignments = tracks.map(|track| track.assignment);
        let mut inserts = Vec::<_, 8>::new();
        let mut enabled = self.sequences;
        let mut automated = [false; CHANNELS];
        for (i, events) in self.events.iter_mut().enumerate() {
            let track = tracks[i];
            let part = track.assignment.part;
            let old = self
                .previous
                .as_ref()
                .map(|previous| previous[i])
                .filter(|old| old.assignment == track.assignment);
            if old.is_none() {
                inserts
                    .push(at(Message::Insert(Channel::ALL[i], part)))
                    .map_err(|_| Error::Capacity)?;
            }
            // Each synth's live gate takes over its own sequencer lane.
            enabled[i] &= part.is_kit() || track.voice.gate.note.is_none();
            automated[i] = self.transport.playing() && enabled[i] && part == Part::Bass;
            let mut panel = track.events(old)?;
            if automated[i] {
                panel.retain(|event| !is_automation(event.message));
            } else if old.is_some() && self.automated[i] {
                for message in track.bass.automation_reset() {
                    panel.push(at(message)).map_err(|_| Error::Capacity)?;
                }
            }
            events.clear();
            events
                .extend_from_slice(&panel)
                .map_err(|_| Error::Capacity)?;
            if part.is_kit() {
                track
                    .panel_hits
                    .append_since(old.map_or(0, |t| t.panel_hits.total()), events)?;
                track
                    .midi_hits
                    .append_since(old.map_or(0, |t| t.midi_hits.total()), events)?;
            }
        }
        let mut player = self.player;
        player.render(N, self.transport, assignments, enabled, |channel, event| {
            self.events[channel.index()]
                .push(event)
                .map_err(|_| Error::Capacity)
        })?;
        for events in self.events.iter_mut() {
            // Stable insertion sort. Replace the processor before any of its messages,
            // then release sequence notes before any same-frame live strike.
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
        let previous_fx = self.previous_fx.filter(|_| inserts.is_empty());
        inserts
            .extend_from_slice(&self.fx.events(previous_fx, FxPage::BassComp))
            .map_err(|_| Error::Capacity)?;
        let mut drums = [[DrumLamp::Off; 6]; CHANNELS];
        self.patch.render(
            &mut Software,
            buses,
            &[
                &self.events[0],
                &self.events[1],
                &self.events[2],
                &self.events[3],
                &mixer_events,
                &reverb,
                &delay,
                &inserts,
            ],
            |object, event| match event.message {
                Message::DrumLamps(lamps) => {
                    drums[usize::from(object)] = lamps;
                    Ok(())
                }
                _ => Err(event),
            },
        )?;
        self.player = player;
        self.position = player.position();
        self.automated = automated;
        self.drums = drums;
        self.previous = Some(tracks);
        self.previous_mixer = Some(self.mixer);
        self.previous_fx = Some(self.fx);
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
        Message::Assign(_) => 0,
        Message::Mono(MonoMessage::NoteOff(_))
        | Message::Organ(OrganMessage::NoteOff(_))
        | Message::Bass(BassMessage::NoteOff(_) | BassMessage::Reset)
        | Message::Drum(DrumMessage::Stop) => 1,
        Message::Mono(MonoMessage::NoteOn { .. })
        | Message::Organ(OrganMessage::NoteOn { .. })
        | Message::Bass(BassMessage::NoteOn { .. })
        | Message::Drum(DrumMessage::Trigger(_, _)) => 3,
        _ => 2,
    };
    (event.frame, order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        keys::{Keys, MidiAction, Piano, decode_midi},
        panel::Channel,
    };
    use wavegraph_fx::{Delay, Reverb, SidechainDucker};

    const DRY_BUS: usize = 13;
    const REVERB_SEND: usize = 14;
    const DELAY_SEND: usize = 15;
    const REVERB_RETURN: usize = 16;
    const DELAY_RETURN: usize = 17;
    const DRY_AND_REVERB: usize = 18;
    const BASS_INSERT: usize = STRIP_BUS + Channel::Three.index();

    fn note(number: u8, velocity: u8) -> Note {
        Note { number, velocity }
    }

    struct Keyboard<const N: usize> {
        piano: Piano<N>,
        keys: Keys<N>,
        nav: Navigation,
        tracks: [Track; CHANNELS],
    }

    impl<const N: usize> Keyboard<N> {
        fn new() -> Self {
            Self {
                piano: Piano::new(),
                keys: Keys::new(),
                nav: Navigation::SYNTH_RACK,
                tracks: Track::TEST_TRACKS,
            }
        }

        fn shift_octave(&mut self, delta: i8) {
            self.piano.shift_octave(delta);
        }

        fn press(&mut self, note: Note) {
            let before = self.piano.held.clone();
            self.piano.press(
                self.nav.focus,
                self.nav.assignments[self.nav.focus.index()],
                note,
            );
            self.keys.panel(&before, &self.piano.held, &self.tracks);
            self.snapshot();
        }

        fn release(&mut self, note: u8) {
            let before = self.piano.held.clone();
            self.piano.release(note);
            self.keys.panel(&before, &self.piano.held, &self.tracks);
            self.snapshot();
        }

        fn snapshot(&mut self) {
            for channel in Channel::ALL {
                self.keys
                    .snapshot(channel, &mut self.tracks[channel.index()]);
            }
        }

        fn gates(&self) -> [Gate; 3] {
            core::array::from_fn(|i| self.tracks[i].voice.gate)
        }

        fn organ_keys(&self) -> [OrganKey; POLYPHONY] {
            self.tracks[1].organ_keys
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "The audio sample rates in these tests fit in an f32 exactly"
    )]
    fn with_instrument(
        rate_hz: u32,
        test: impl FnOnce(&mut Instrument<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let rate = (rate_hz, 1);
        let programs = Programs::decode()?;
        let tuning = Tuning::new(rate.0, rate.1).expect("audio sample clock");
        let organ = crate::organ::instrument(rate.0, rate.1);
        let drums = DrumKits::new(rate);
        let [one, two, three, four] =
            &mut core::array::from_fn(|_| Node::new(&tuning, &organ, &drums, rate_hz as f32));
        let mut mixer = FxMixer(ChannelMixer::new().with_boost());
        let mut storage = [0_i32; wavegraph_fx::REVERB_SAMPLES];
        let mut reverb = FxNode(Reverb::new(&mut storage));
        let mut line = [0; 44_118];
        let mut delay = FxNode(Delay::new(&mut line, rate_hz).expect("delay sample clock"));
        let mut inserts = crate::inserts::BassInserts::new(rate_hz);
        let mut nodes: [&mut dyn Process<Message, I24>; 8] = [
            one,
            two,
            three,
            four,
            &mut mixer,
            &mut reverb,
            &mut delay,
            &mut inserts,
        ];
        let mut events = [const { RoutedEvents::new() }; CHANNELS];
        test(&mut programs.bind(&mut nodes, &mut events, rate)?)
    }

    #[test]
    fn keys_keep_their_owner_with_three_parts_and_polyphonic_organ() {
        let mut keys = Keyboard::<5>::new();
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
        keys.nav.focus = Channel::Two;
        keys.press(e);
        keys.press(g);
        assert_eq!(keys.organ_keys()[0].note, Some(g));
        assert_eq!(keys.organ_keys()[1].note, Some(e));
        keys.nav.focus = Channel::Three;
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
        keys.nav.focus = Channel::One;
        keys.press(a);
        assert_ne!(keys.gates()[0].phrase, mono.phrase);
        keys.release(a.number);
    }

    #[test]
    fn octave_follows_the_keyboard_and_held_keys_keep_their_pitch_and_part() {
        let mut keys = Keyboard::<4>::new();
        keys.shift_octave(1);
        keys.press(note(53, 80)); // F3 plays F4 on the monosynth.
        keys.nav.turn(0, 1);
        keys.press(note(57, 90)); // A3 plays A4 on the organ.
        keys.shift_octave(-1);
        keys.press(note(65, 95)); // F4 on the organ.
        keys.nav.turn(0, 1);
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
            let mut keys = Keyboard::<1>::new();
            for _ in 0..5 {
                keys.shift_octave(step);
            }
            keys.press(note(key, 100));
            let mut parts = Track::TEST_TRACKS;
            parts[0].turn_select(transpose);
            let voice = &mut parts[0].voice;
            voice.gate = keys.gates()[Part::Mono as usize];
            assert_eq!(voice.sounding(), Some(note(pitch, 100)));
            assert_eq!(voice.note_name(), name);
        }
    }

    #[test]
    fn either_organ_key_can_sustain_a_shared_pitch() {
        for (first, last) in [(65, 53), (53, 65)] {
            let mut keys = Keyboard::<2>::new();
            keys.nav.turn(0, 1);
            keys.press(note(65, 60));
            keys.shift_octave(1);
            keys.press(note(53, 100));
            let mut parts = Track::TEST_TRACKS;
            parts[1].organ_keys = keys.organ_keys();
            let strike = |velocity| at(OrganMessage::NoteOn { note: 65, velocity });
            assert_eq!(
                parts[1]
                    .organ_events(Some(Track::TEST_TRACKS[1]))
                    .as_slice(),
                &[strike(60), strike(100)]
            );
            let both = parts;
            keys.release(first);
            parts[1].organ_keys = keys.organ_keys();
            assert!(parts[1].organ_events(Some(both[1])).is_empty());
            let remaining = parts;
            keys.release(last);
            parts[1].organ_keys = keys.organ_keys();
            assert_eq!(
                parts[1].organ_events(Some(remaining[1])).as_slice(),
                &[at(OrganMessage::NoteOff(65))]
            );
        }
    }

    #[test]
    fn knob_one_preserves_both_cutoffs_and_sends_the_initial_monosynth_cutoff() {
        let mut parts = Track::TEST_TRACKS;
        let initial = parts;
        assert_eq!(initial[0].voice.cutoff_hz(), 5000);
        assert_eq!(
            initial[0].mono_events(None).as_slice(),
            &[
                at(MonoMessage::Waveform(Waveform::Sine)),
                at(MonoMessage::CutoffHz(5000)),
                at(MonoMessage::VcaEnvelope(parts[0].mono_envelope.adsr())),
            ]
        );
        parts[0].turn_knobs([-6, 0, 0, 0]);
        assert_eq!(parts[2].bass, Controls::INITIAL);
        parts[2].turn_knobs([6, 0, 0, 0]);
        assert_eq!(parts[2].bass.readout().1, 1000);
        parts[0].mono_envelope.turn([1, -1, 2, -2]);
        let mono = &mut parts[0].voice;
        assert_eq!(mono.cutoff_hz(), 2500);
        mono.turn_waveform(Part::Mono, 2);
        mono.gate.note = Some(note(69, 80));
        assert_eq!(
            parts[0].mono_events(Some(initial[0])).as_slice(),
            &[
                at(MonoMessage::Waveform(Waveform::Sawtooth)),
                at(MonoMessage::CutoffHz(2500)),
                at(MonoMessage::VcaEnvelope(parts[0].mono_envelope.adsr())),
                at(MonoMessage::NoteOn {
                    note: 69,
                    velocity: 80
                }),
            ]
        );
        let held = parts;
        parts[0].voice.turn_cutoff(i8::MIN);
        assert_eq!(
            parts[0].mono_events(Some(held[0])).as_slice(),
            &[at(MonoMessage::CutoffHz(10))]
        );
        parts[0].voice.turn_cutoff(i8::MAX);
        assert_eq!(parts[0].voice.cutoff_hz(), 20000);
        let open = parts;
        parts[0].voice.turn_cutoff(1);
        assert!(parts[0].mono_events(Some(open[0])).is_empty());
        assert_eq!(parts[2].bass.readout().1, 1000);
    }

    #[test]
    fn mono_retriggers_after_a_short_gap_and_plays_overlapping_keys_legato() {
        let mut keys = Keyboard::<2>::new();
        let mut parts = Track::TEST_TRACKS;
        keys.press(note(60, 80));
        parts[0].voice.gate = keys.gates()[0];
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
        parts[0].voice.gate = keys.gates()[0];
        assert_eq!(
            parts[0].mono_events(Some(first[0])).as_slice(),
            &[at(MonoMessage::NoteOff(60)), play(60)]
        );

        let repeated = parts;
        keys.press(note(67, 80));
        parts[0].voice.gate = keys.gates()[0];
        assert_eq!(
            parts[0].mono_events(Some(repeated[0])).as_slice(),
            &[play(67)]
        );
        let legato = parts;
        keys.release(67);
        parts[0].voice.gate = keys.gates()[0];
        assert_eq!(
            parts[0].mono_events(Some(legato[0])).as_slice(),
            &[play(60)]
        );
    }

    #[test]
    fn plays_at_strike_velocity_and_releases_the_transposed_note() -> Result<(), Error> {
        // At this rate the 5 kHz default is above Nyquist, so the square keeps its exact levels.
        with_instrument(8_000, |instrument| {
            let mut buses = [[I24::default(); 32]; BUS_COUNT];
            let mut parts = Track::TEST_TRACKS;
            parts[0]
                .mono_envelope
                .turn([i8::MIN, i8::MIN, i8::MAX, i8::MIN]);
            assert_eq!(parts[0].voice.note_name(), *b"--  ");
            assert_eq!(
                instrument.render(parts, &mut buses)?.map(I24::inner),
                [0; 32]
            );
            parts[0].voice.turn_waveform(Part::Mono, -1);
            assert_eq!(parts[0].voice.label(Part::Mono), b"SQUARE  ");
            for velocity in [32, 96, 32] {
                let mono = &mut parts[0].voice;
                mono.gate.note = Some(note(69, velocity));
                mono.transpose = 12;
                assert_eq!(mono.note_name(), *b"A5  ");
                let block = instrument.render(parts, &mut buses)?.map(I24::inner);
                let level = (32767 * i32::from(velocity)) >> 3;
                assert!(block[12..].iter().all(|sample| sample.abs() == level));
            }
            parts[0].voice.gate.note = None;
            parts[0].voice.transpose = 0;
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
            let mut parts = Track::TEST_TRACKS;
            let channel = Channel::Two;
            let i = channel.index();
            let assignment = Assignment {
                part: Part::Tr909,
                generation: 1,
            };
            parts[i] = Track::factory(channel, assignment); // A second 909 beside the factory kit.
            let mut piano = Piano::<1>::new();
            let mut tap = |key: u8, track: &mut Track| {
                piano.press(channel, assignment, note(53 + key, 127));
                piano.release(53 + key); // Both edges can precede the next panel snapshot.
                piano.pads[i].apply(track);
            };
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut mixer = Mixer::DRY;
            mixer.0[i].level = 0;
            instrument.set_mixer(mixer);
            tap(1, &mut parts[i]); // Black key, outside the six pads.
            instrument.render(parts, &mut buses)?;
            assert_eq!(instrument.drums[i], [Off; 6]);

            for key in [0, 2, 4, 6, 7, 9] {
                tap(key, &mut parts[i]);
            }
            assert_eq!(
                *instrument.render(parts, &mut buses)?,
                [I24::default(); 128]
            );
            assert!(
                channel
                    .scope_samples(Part::Tr909, &buses)
                    .iter()
                    .any(|&s| s != 0)
            );
            assert_eq!(
                instrument.drums[i],
                [Strike, Strike, Strike, Off, Strike, Strike]
            );
            assert_eq!(instrument.drums[Channel::Four.index()], [Off; 6]);
            for _ in 0..24 {
                instrument.render(parts, &mut buses)?;
            }
            assert_eq!(instrument.drums[i], [Tail, Tail, Tail, Off, Tail, Tail]);
            tap(6, &mut parts[i]); // Closed hat immediately chokes open hat.
            tap(4, &mut parts[i]); // Retrigger an already-ringing clap.
            instrument.render(parts, &mut buses)?;
            assert_eq!(instrument.drums[i], [Tail, Tail, Strike, Strike, Off, Tail]);
            for _ in 0..350 {
                instrument.render(parts, &mut buses)?;
            }
            assert_eq!(instrument.drums[i], [Off; 6]);
            assert_eq!(channel.scope_samples(Part::Tr909, &buses), [0; 128]);
            Ok(())
        })
    }

    #[test]
    fn transport_plays_the_drum_bus_alongside_a_held_synth_note() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut parts = Track::TEST_TRACKS;
            instrument.sequences = [false, false, false, true];
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let silence = instrument.render(parts, &mut buses)?.map(I24::inner);
            assert_eq!(silence, [0; 128]);
            assert_eq!(
                instrument.position.map(|position| position.beat_phase),
                None
            );
            instrument.transport.toggle();
            let beat = *instrument.render(parts, &mut buses)?;
            assert_eq!(
                instrument.position.map(|position| position.beat_phase),
                Some(1)
            );
            assert_eq!(beat, buses[Channel::Four.source_bus()]);
            assert_eq!(instrument.drums[3][Drum::Kick as usize], DrumLamp::Strike);
            assert!(beat.iter().any(|s| s.inner() != 0));
            parts[0].voice.gate.note = Some(note(69, 96));
            instrument.render(parts, &mut buses)?;
            assert_ne!(buses[MASTER_BUS], buses[Channel::Four.source_bus()]);
            instrument.transport.toggle();
            let tail = *instrument.render(parts, &mut buses)?;
            assert_eq!(
                instrument.position.map(|position| position.beat_phase),
                None
            );
            assert_eq!(buses[Channel::Four.source_bus()], [I24::default(); 128]);
            assert_eq!(instrument.drums[3], [DrumLamp::Off; 6]);
            assert_eq!(tail, buses[DRY_BUS]);
            assert!(tail.iter().any(|s| s.inner() != 0));
            Ok(())
        })
    }

    #[test]
    fn live_bass_takeover_preserves_both_sends_and_stop_preserves_the_live_key() -> Result<(), Error>
    {
        with_instrument(44_100, |instrument| {
            let mut parts = Track::rack(Navigation::SYNTH_RACK.assignments);
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut mixer = Mixer::DRY;
            mixer.0[2].level = 64;
            mixer.0[2].reverb = 128;
            mixer.0[2].delay = 64;
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
            instrument.sequences = [false, false, true, false];
            instrument.transport.toggle();
            instrument.render(parts, &mut buses)?;
            assert!(instrument.automated[2]);
            check_sends(&buses);
            let pitch = instrument.events[2]
                .iter()
                .find_map(|event| match event.message {
                    Message::Bass(BassMessage::NoteOn { note, .. }) => Some(note),
                    _ => None,
                })
                .expect("the factory bass starts on the downbeat");

            parts[2].bass.turn_knobs([2, -3, 0, 0]);
            parts[2].voice.gate = Gate {
                note: Some(note(pitch, 110)),
                phrase: 1,
            };
            instrument.render(parts, &mut buses)?;
            assert!(!instrument.automated[2]);
            check_sends(&buses);
            let events = &instrument.events[2];
            assert_eq!(
                events[0].message,
                Message::Bass(BassMessage::NoteOff(pitch))
            );
            assert!(matches!(events.last().expect("live strike").message,
                Message::Bass(BassMessage::NoteOn { note, slide: false, .. }) if note == pitch));
            for message in parts[2].bass.automation_reset() {
                assert!(
                    events
                        .iter()
                        .any(|event| event.message == Message::Bass(message))
                );
            }

            instrument.transport.toggle();
            // The fader silences dry and both sends without stopping a held
            // live voice. Its earlier effect returns retain their own tails.
            mixer.0[2].level = 0;
            instrument.set_mixer(mixer);
            instrument.render(parts, &mut buses)?;
            assert!(instrument.events[2].is_empty());
            assert!(
                buses[DRY_BUS..=DELAY_SEND]
                    .iter()
                    .flatten()
                    .all(|sample| sample.inner() == 0)
            );
            assert!(
                buses[Channel::Three.source_bus()]
                    .iter()
                    .any(|sample| sample.inner() != 0)
            );
            parts[2].voice.gate.note = None;
            instrument.render(parts, &mut buses)?;
            assert_eq!(
                instrument.events[2].as_slice(),
                &[at(BassMessage::NoteOff(pitch))]
            );
            Ok(())
        })
    }

    #[test]
    fn muting_the_bass_restores_edited_panel_values_without_stopping_other_tracks()
    -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut parts = Track::rack(Navigation::SYNTH_RACK.assignments);
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            instrument.transport.toggle();
            instrument.render(parts, &mut buses)?;
            parts[2].bass.turn_knobs([4, -6, 0, 0]);
            instrument.render(parts, &mut buses)?;
            assert!(instrument.automated[2]);
            instrument.sequences[2] = false;
            instrument.render(parts, &mut buses)?;
            assert!(!instrument.automated[2]);
            assert!(instrument.position.is_some());
            for message in parts[2].bass.automation_reset() {
                assert!(
                    instrument.events[2]
                        .iter()
                        .any(|event| event.message == Message::Bass(message))
                );
            }
            assert!(
                buses[Channel::Four.source_bus()]
                    .iter()
                    .any(|sample| sample.inner() != 0)
            );
            Ok(())
        })
    }

    #[test]
    fn changing_routes_preserves_the_sounding_oscillators_phase() -> Result<(), Error> {
        with_instrument(1760, |instrument| {
            let mut buses = [[I24::default(); 3]; BUS_COUNT];
            let mut parts = Track::TEST_TRACKS;
            parts[0].voice.gate.note = Some(note(69, 80));
            assert_eq!(parts[0].voice.note_name(), *b"A4  ");
            let dry = instrument.render(parts, &mut buses)?.map(I24::inner);
            assert_eq!(dry[0], 0);
            assert!(dry[1] > 0);
            instrument.set_half_gain(true)?;
            let half = instrument.render(parts, &mut buses)?.map(I24::inner);
            assert!(half[0] < 0);
            assert_eq!(
                half,
                buses[Channel::One.source_bus()].map(|sample| sample.inner() >> 1)
            );
            Ok(())
        })
    }

    #[test]
    fn both_voices_reach_the_mix_and_releasing_one_preserves_the_other() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut parts = Track::TEST_TRACKS;
            let mut keys = Keyboard::<2>::new();
            let sync = |parts: &mut [Track; CHANNELS], keys: &Keyboard<2>| *parts = keys.tracks;
            keys.press(note(69, 84));
            sync(&mut parts, &keys);
            instrument.render(parts, &mut buses)?;
            assert_eq!(buses[Channel::Three.source_bus()].map(I24::inner), [0; 128]);
            keys.nav.focus = Channel::Three;
            keys.press(note(53, 110));
            sync(&mut parts, &keys);
            for half in [false, true] {
                instrument.set_half_gain(half)?;
                let mix = instrument.render(parts, &mut buses)?.map(I24::inner);
                let mono = &buses[Channel::One.source_bus()];
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
            assert!(
                buses[Channel::One.source_bus()]
                    .iter()
                    .any(|s| s.inner() != 0)
            );
            // The mono's 125 ms release continues while the 303 key is held.
            for _ in 0..44 {
                instrument.render(parts, &mut buses)?;
            }
            assert_eq!(buses[Channel::One.source_bus()].map(I24::inner), [0; 128]);
            assert!(
                buses[Channel::Three.source_bus()]
                    .iter()
                    .any(|s| s.inner() != 0)
            );
            keys.nav.focus = Channel::One;
            keys.release(53);
            sync(&mut parts, &keys);
            instrument.render(parts, &mut buses)?;
            assert!(
                buses[Channel::Three.source_bus()]
                    .iter()
                    .any(|s| s.inner() != 0)
            );
            for _ in 0..60 {
                instrument.render(parts, &mut buses)?;
            }
            assert!(
                buses[Channel::Three.source_bus()]
                    .iter()
                    .all(|s| s.inner().abs() <= 1)
            );
            Ok(())
        })
    }

    #[test]
    fn select_switches_the_legend_without_changing_the_sound_and_recalls_each_bank() {
        let mut parts = Track::TEST_TRACKS;
        let voice = Voice {
            gate: Gate {
                note: Some(note(53, 100)),
                phrase: 1,
            },
            ..Voice::INITIAL
        };
        parts[0].voice = voice;
        parts[2].voice = voice;
        parts[2].turn_knobs([0, 0, 0, -10]);
        let filter = parts[2].bass;
        assert_eq!(
            filter.readout(),
            (b"DECAY    ".as_slice(), 500, b"MS   ".as_slice())
        );
        parts[2].turn_select(-1);
        assert_eq!(
            parts[2].bass.bank.knobs().map(|k| k.name()),
            [b"TUNING   ", b"ATTACK   ", b"ACCENT   ", b"OVERDRIVE"]
        );
        assert_eq!(parts[2].bass.last().name(), b"TUNING   ");
        assert!(
            parts[2]
                .bass
                .events(filter, parts[2].voice, voice)
                .is_empty()
        );
        parts[2].turn_knobs([-7, 1, 5, 12]);
        let edited = parts[2].bass;
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
        parts[2].turn_select(i8::MIN);
        assert_eq!(parts[2].bass, edited);
        parts[2].turn_select(i8::MAX);
        assert_eq!(
            parts[2].bass.bank.knobs().map(|k| k.name()),
            [b"CUTOFF   ", b"RESONANCE", b"ENVMOD   ", b"DECAY    "]
        );
        assert_eq!(parts[2].bass.readout(), filter.readout());
        parts[0].turn_select(-4);
        assert_eq!(parts[0].voice.sounding(), Some(note(49, 100)));
        assert_eq!(parts[2].voice, voice);
        parts[2].turn_select(1);
        assert_eq!(parts[2].bass, edited);
    }

    #[test]
    fn shared_reverb_has_independent_sends_and_preserves_dry_audio() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut parts = Track::TEST_TRACKS;
            parts[0].voice.gate.note = Some(note(69, 110));
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
    fn compressor_panel_selects_the_named_pre_fader_signal() -> Result<(), Error> {
        use crate::fx::KeySource::{Bass, Kick, Kit, Mono, Organ as OrganKeySource};
        for (turn, key, detector_bus) in [
            (-1, Bass, Channel::Three.source_bus()),
            (0, Kick, Channel::Four.source_bus() + 1),
            (1, Kit, Channel::Four.source_bus()),
            (2, OrganKeySource, Channel::Two.source_bus()),
            (3, Mono, Channel::One.source_bus()),
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
                let mut parts = Track::rack(Navigation::SYNTH_RACK.assignments);
                parts[0].voice.gate.note = Some(note(72, 120));
                let mut mixer = Mixer::DRY;
                for channel in [Channel::One, Channel::Two, Channel::Four] {
                    mixer.0[channel.index()].level = 0;
                }
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
                        inputs: &[&buses[Channel::Three.source_bus()], &buses[detector_bus]],
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
                assert_eq!(
                    &buses[BASS_INSERT][64..],
                    &buses[Channel::Three.source_bus()][64..]
                );
                Ok(())
            })?;
        }
        Ok(())
    }

    #[test]
    fn two_reassigned_kits_key_both_bass_inserts_pre_fader() -> Result<(), Error> {
        with_instrument(44_118, |instrument| {
            let mut tracks = Track::rack(
                [Part::Tr909, Part::Bass, Part::Bass, Part::Tr909].map(Assignment::new),
            );
            let mut mixer = Mixer::DRY;
            mixer.0[0].level = 0;
            mixer.0[3].level = 0;
            instrument.set_mixer(mixer);
            tracks[1].voice.gate.note = Some(note(43, 100));
            tracks[2].voice.gate.note = Some(note(55, 100));

            let mut reference =
                [SidechainDucker::<2>::new(44_118, 32, 176).expect("valid insert"); 2];
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            for (block, kit) in [0, 3, 3].into_iter().enumerate() {
                tracks[kit].midi_hits.push(Drum::Kick, 127);
                if block == 2 {
                    // Removing a ringing kit removes it from the key immediately.
                    tracks[0] = Track::factory(Channel::One, Assignment::new(Part::Organ));
                }
                instrument.render(tracks, &mut buses)?;
                let first = &buses[Channel::One.source_bus() + 1];
                let last = &buses[Channel::Four.source_bus() + 1];
                let key: [_; 128] = core::array::from_fn(|i| {
                    I24::new_unchecked(
                        (first[i].inner() + last[i].inner()).clamp(-(1 << 23), (1 << 23) - 1),
                    )
                });
                assert!(key.iter().any(|s| s.inner() != 0));
                if block == 2 {
                    assert_eq!(first, &[I24::default(); 128]);
                }
                let controls = [
                    Event {
                        frame: 0,
                        port: 0,
                        message: FxMessage::SidechainDepth(Fx::INITIAL.depth),
                    },
                    Event {
                        frame: 0,
                        port: 0,
                        message: FxMessage::SidechainHz(Fx::INITIAL.split_hz),
                    },
                ];
                for (channel, ducker) in [Channel::Two, Channel::Three]
                    .into_iter()
                    .zip(&mut reference)
                {
                    let mut expected = [I24::default(); 128];
                    ducker.process(&mut ProcessContext {
                        frames: 128,
                        inputs: &[&buses[channel.source_bus()], &key],
                        outputs: &mut [&mut expected],
                        events: if block == 0 { &controls } else { &[] },
                        emit: &mut Err,
                    });
                    assert!(expected.iter().any(|s| s.inner() != 0));
                    assert_eq!(buses[STRIP_BUS + channel.index()], expected);
                }
            }
            Ok(())
        })
    }

    #[test]
    fn factory_rack_leaves_headroom_with_both_acid_sends_ringing() -> Result<(), Error> {
        with_instrument(44_118, |instrument| {
            let tracks = Track::rack(Navigation::INITIAL.assignments);
            instrument.set_mixer(Mixer::INITIAL);
            instrument.transport.toggle();
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut energy = [0_i64; 6];
            let mut peak = 0;
            // Two four-bar loops include tails across the loop boundary.
            for _ in 0..5425 {
                instrument.render(tracks, &mut buses)?;
                for frame in 0..128 {
                    for (i, strip) in Mixer::INITIAL.0.iter().enumerate() {
                        let value = (i64::from(buses[STRIP_BUS + i][frame].inner())
                            * i64::from(strip.level))
                            >> 15;
                        energy[i] += value * value;
                    }
                    for (meter, bus) in [(4, REVERB_RETURN), (5, DELAY_RETURN)] {
                        energy[meter] += i64::from(buses[bus][frame].inner() >> 8).pow(2);
                    }
                    peak = peak.max(buses[MASTER_BUS][frame].inner().abs());
                }
            }
            let [bass, harmony, kit808, kit909, reverb, delay] = energy;
            // Upper voice and both returns stay audible beside the dry foundation.
            assert!(harmony > bass / 8 && kit808 > kit909 / 16);
            assert!(reverb > bass / 256 && delay > bass / 8);
            assert!(peak < 8_000_000, "full-gain master peak {peak}");
            Ok(())
        })
    }

    #[test]
    fn delay_returns_after_the_drum_fader_goes_down_and_the_graph_changes() -> Result<(), Error> {
        // Five milliseconds at 32 kHz is exactly one test block.
        with_instrument(32_000, |instrument| {
            let mut buses = [[I24::default(); 160]; BUS_COUNT];
            let mut parts = Track::TEST_TRACKS;
            instrument.sequences = [false, false, false, true];
            parts[0].voice.gate.note = Some(note(69, 100));
            let mut mixer = Mixer::DRY;
            mixer.0[0].reverb = 128;
            mixer.0[3].delay = 128;
            let mut fx = Fx::INITIAL;
            let mut page = FxPage::Reverb;
            page.turn(1);
            fx.edit(page, [-74, 7, 64, 0], false);
            assert_eq!((fx.time_ms, fx.feedback, fx.delay.wet), (5, 50, 128));
            assert_eq!(fx.reverb, Fx::INITIAL.reverb);
            instrument.set_mixer(mixer);
            instrument.set_fx(fx);
            instrument.transport.toggle();
            instrument.render(parts, &mut buses)?;
            assert_eq!(
                (buses[REVERB_SEND], buses[DELAY_SEND]),
                (
                    buses[Channel::One.source_bus()],
                    buses[Channel::Four.source_bus()]
                )
            );
            assert_eq!(buses[DELAY_RETURN], [I24::default(); 160]);
            let echo = buses[Channel::Four.source_bus()].map(|s| (s.inner() / 256) << 8);
            assert!(echo.iter().any(|&s| s != 0));

            mixer.0[3].level = 0;
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
            let mut parts = Track::TEST_TRACKS;
            let mut keys = Keyboard::<6>::new();
            keys.nav.focus = Channel::Two;
            for pitch in [60, 64, 67, 71] {
                keys.press(note(pitch, 110));
            }
            parts[1].organ_keys = keys.organ_keys();
            instrument.render(parts, &mut buses)?;
            assert!(
                buses[Channel::Two.source_bus()]
                    .iter()
                    .any(|x| x.inner() != 0)
            );
            assert_eq!(buses[Channel::Three.source_bus()].map(I24::inner), [0; 128]);
            // Fifth key displaces the oldest, and releasing it brings back the oldest.
            keys.press(note(74, 95));
            parts[1].organ_keys = keys.organ_keys();
            instrument.render(parts, &mut buses)?;
            assert_eq!(parts[1].organ_keys[0].note, Some(note(74, 95)));
            keys.release(74);
            parts[1].organ_keys = keys.organ_keys();
            assert_eq!(parts[1].organ_keys[3].note, Some(note(60, 110)));
            instrument.render(parts, &mut buses)?;
            keys.nav.focus = Channel::Three;
            keys.press(note(53, 100));
            parts[2].voice.gate = keys.gates()[Part::Bass as usize];
            instrument.render(parts, &mut buses)?;
            assert!(
                buses[Channel::Two.source_bus()]
                    .iter()
                    .any(|x| x.inner() != 0)
            );
            assert!(
                buses[Channel::Three.source_bus()]
                    .iter()
                    .any(|x| x.inner() != 0)
            );
            for pitch in [60, 64, 67, 71] {
                keys.release(pitch);
            }
            parts[1].organ_keys = keys.organ_keys();
            instrument.render(parts, &mut buses)?;
            assert_eq!(parts[1].organ_keys, [OrganKey::EMPTY; POLYPHONY]);
            Ok(())
        })
    }

    #[test]
    fn bass_legato_retrigger_and_panel_changes() {
        let mut controls = Controls::INITIAL;
        let mut keys = Keyboard::<2>::new();
        keys.nav.focus = Channel::Three;
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

    #[test]
    fn identical_assignments_play_independent_midi_notes_and_swap_at_a_block_boundary()
    -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut tracks = [Track::new(Assignment::new(Part::Bass)); CHANNELS];
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut keys = Keys::<1>::new();
            for (i, number) in [41, 47, 53, 59].into_iter().enumerate() {
                let status = u8::try_from(i).expect("four channels") | 0x90;
                let (channel, action) = decode_midi([9, status, number, 106]).expect("note on");
                assert_eq!(channel.index(), i);
                keys.midi(channel, action, &mut tracks[i]);
                keys.snapshot(channel, &mut tracks[i]);
            }
            instrument.render(tracks, &mut buses)?;
            for channel in Channel::ALL {
                let bus = &buses[channel.source_bus()];
                assert!(bus.iter().any(|sample| sample.inner() != 0));
            }
            assert_ne!(
                buses[Channel::One.source_bus()],
                buses[Channel::Two.source_bus()]
            );
            assert_ne!(
                buses[Channel::Two.source_bus()],
                buses[Channel::Three.source_bus()]
            );
            assert_ne!(
                buses[Channel::Three.source_bus()],
                buses[Channel::Four.source_bus()]
            );
            keys.clear(Channel::Two);
            tracks[1] = Track::new(Assignment {
                part: Part::Tr909,
                generation: 1,
            });
            keys.midi(Channel::Two, MidiAction::Off(47), &mut tracks[1]);
            instrument.render(tracks, &mut buses)?;
            assert_eq!(buses[Channel::Two.source_bus()], [I24::default(); 128]);
            assert!(
                buses[Channel::Three.source_bus()]
                    .iter()
                    .any(|sample| sample.inner() != 0)
            );
            keys.midi(Channel::Two, MidiAction::On(note(36, 127)), &mut tracks[1]);
            instrument.render(tracks, &mut buses)?;
            assert!(
                buses[Channel::Two.source_bus()]
                    .iter()
                    .any(|sample| sample.inner() != 0)
            );
            let mut lamps = [[DrumLamp::Off; 6]; CHANNELS];
            lamps[1][Drum::Kick as usize] = DrumLamp::Strike;
            assert_eq!(instrument.drums, lamps);
            Ok(())
        })
    }

    #[test]
    fn rebirth_lineup_plays_and_each_drum_sidechain_follows_its_selected_kit() -> Result<(), Error>
    {
        use crate::fx::KeySource;
        with_instrument(44_118, |instrument| {
            let nav = Navigation::INITIAL;
            assert_eq!(
                nav.assignments.map(|a| a.part),
                [Part::Bass, Part::Bass, Part::Tr808, Part::Tr909]
            );
            let mut tracks = Track::rack(nav.assignments);
            assert_eq!(instrument.sequences, [true; 4]);
            tracks[1].voice.gate.note = Some(note(65, 100));
            tracks[2].midi_hits.push(Drum::Kick, 96);
            instrument.transport.toggle();
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            for (key, source) in [
                (KeySource::Kick, 7),
                (KeySource::Kick808, 5),
                (KeySource::Kit808, 4),
                (KeySource::Kit, 6),
            ] {
                instrument.set_fx(Fx { key, ..Fx::INITIAL });
                instrument.render(tracks, &mut buses)?;
                assert_eq!(buses[12], buses[source]);
                assert_ne!(buses[4], buses[6]);
                for c in Channel::ALL {
                    assert!(buses[c.source_bus()].iter().any(|s| s.inner() != 0));
                }
            }
            // Reassign channel four mid-kick. Channel three's tail must survive
            // and the new 808 must start silent until the next programmed hit.
            tracks[3] = Track::factory(
                Channel::Four,
                Assignment {
                    part: Part::Tr808,
                    generation: 1,
                },
            );
            instrument.render(tracks, &mut buses)?;
            assert!(buses[4].iter().any(|s| s.inner() != 0));
            assert!(buses[6].iter().all(|s| s.inner() == 0));
            Ok(())
        })
    }

    #[test]
    fn an_808_in_any_slot_accepts_only_its_own_midi_and_four_panel_pads() -> Result<(), Error> {
        for channel in Channel::ALL {
            with_instrument(44_118, |instrument| {
                let mut tracks = Track::TEST_TRACKS;
                let i = channel.index();
                let assignment = Assignment {
                    part: Part::Tr808,
                    generation: 1,
                };
                tracks[i] = Track::factory(channel, assignment);
                let mut keys = Keys::<2>::new();
                let mut piano = Piano::<2>::new();
                let mut buses = [[I24::default(); 128]; BUS_COUNT];
                let mut reference = wavegraph_808::Tr808::new(44_118, 1).expect("audio clock");
                // A fresh assignment plus a completed panel tap and two MIDI
                // strikes (with different velocities) all precede its first render.
                for pad in [53, 55, 57] {
                    piano.press(channel, assignment, note(pad, 64));
                    piano.release(pad);
                }
                piano.pads[i].apply(&mut tracks[i]);
                for n in [note(38, 31), note(38, 105), note(39, 127)] {
                    let packet = [9, 0x90 | channel as u8, n.number, n.velocity];
                    let (dest, action) = decode_midi(packet).expect("USB MIDI note");
                    keys.midi(dest, action, &mut tracks[dest.index()]);
                }
                for (voice, velocity) in [
                    (wavegraph_808::Voice::Kick, 64),
                    (wavegraph_808::Voice::Snare, 64),
                    (wavegraph_808::Voice::ClosedHat, 64),
                    (wavegraph_808::Voice::Snare, 31),
                    (wavegraph_808::Voice::Snare, 105),
                ] {
                    reference.trigger(voice, velocity);
                }
                for _ in 0..2 {
                    instrument.render(tracks, &mut buses)?;
                    let mut mix = [I24::default(); 128];
                    let mut kick = mix;
                    reference.render_mix_kick(&mut mix, &mut kick);
                    assert_eq!(buses[channel.source_bus()], mix);
                    assert_eq!(buses[channel.source_bus() + 1], kick);
                    for other in Channel::ALL.into_iter().filter(|&other| other != channel) {
                        assert!(buses[other.source_bus()].iter().all(|s| s.inner() == 0));
                    }
                }
                assert_eq!(
                    instrument.drums[i],
                    [
                        DrumLamp::Strike,
                        DrumLamp::Strike,
                        DrumLamp::Off,
                        DrumLamp::Strike,
                        DrumLamp::Off,
                        DrumLamp::Off
                    ]
                );
                Ok(())
            })?;
        }
        Ok(())
    }

    #[test]
    fn overlapping_303_assignments_take_over_and_reassign_independently() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut tracks = Track::rack(Navigation::SYNTH_RACK.assignments);
            tracks[0] = Track::factory(Channel::One, Assignment::new(Part::Bass));
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            instrument.sequences = [true, false, true, false];
            instrument.transport.toggle();
            instrument.render(tracks, &mut buses)?;
            assert_eq!(
                buses[Channel::One.source_bus()],
                buses[Channel::Three.source_bus()]
            );
            assert_eq!(instrument.automated, [true, false, true, false]);

            tracks[0].voice.gate.note = Some(note(55, 110));
            instrument.render(tracks, &mut buses)?;
            assert_eq!(instrument.automated, [false, false, true, false]);
            assert_ne!(
                buses[Channel::One.source_bus()],
                buses[Channel::Three.source_bus()]
            );

            tracks[0] = Track::factory(
                Channel::One,
                Assignment {
                    part: Part::Tr909,
                    generation: 1,
                },
            );
            tracks[0].midi_hits.push(Drum::Clap, 127);
            let previous_phase = instrument.position;
            instrument.render(tracks, &mut buses)?;
            assert_ne!(instrument.position, previous_phase);
            assert_eq!(instrument.drums[0][Drum::Clap as usize], DrumLamp::Strike);
            assert!(
                buses[Channel::One.source_bus()]
                    .iter()
                    .any(|sample| sample.inner() != 0)
            );
            assert!(
                buses[Channel::Three.source_bus()]
                    .iter()
                    .any(|sample| sample.inner() != 0)
            );
            assert!(instrument.automated[2]);
            Ok(())
        })
    }

    #[test]
    fn midi_note_off_and_disconnect_leave_the_panel_notes_alone() {
        let mut keys = Keyboard::<2>::new();
        keys.press(note(60, 80));
        for (packet, expected) in [
            ([9, 0x90, 60, 100], Some(note(60, 100))),
            ([9, 0x90, 67, 100], Some(note(67, 100))),
            ([9, 0x90, 67, 0], Some(note(60, 100))),
        ] {
            let (channel, action) = decode_midi(packet).expect("channel one message");
            keys.keys.midi(channel, action, &mut keys.tracks[0]);
            keys.snapshot();
            assert_eq!(keys.tracks[0].voice.gate.note, expected);
        }
        keys.keys.clear_midi();
        keys.snapshot();
        assert_eq!(keys.tracks[0].voice.gate.note, Some(note(60, 80)));
        assert!(decode_midi([9, 0x94, 60, 100]).is_none());
    }
}
