//! Four assignable instrument nodes feed the shared effect send/return graph.

use crate::sampler::Program as SampleProgram;
use crate::wavetable::Program as WaveProgram;
use crate::{
    arp::{self, ChannelArp},
    bass::{Controls, Knob},
    bumper::Cue,
    envelope::{EnvelopeControls, EnvelopeTarget},
    fx::{Device, Fx, INSERT_SLOTS, Insert},
    keys::HitLog,
    keys::PolyKey,
    lfo::{KnobOffset, Route},
    mixer::Mixer,
    organ::Controls as OrganControls,
    panel::{Assignment, CHANNELS, Channel, View},
    sequencer::{
        BankPosition, DrumKits, DrumLamp, DrumMachine, DrumMessage, PatternPlayer, Position,
        Sequencer, Transport,
        pattern::{Automation, Slot},
    },
};
#[cfg(test)]
use crate::{fx::BassFx, panel::Navigation};
use heapless::Vec;
use wavegraph_303::{Message as BassMessage, Tb303};
use wavegraph_909::Voice as Drum;
use wavegraph_dsp::{
    Event, Port, Process, ProcessContext,
    bytecode::{Error, Patch, Program, VERSION},
    dsp::Software,
    midi::MidiMessage,
};
use wavegraph_fm::{Fm, Message as FmMessage};
use wavegraph_fx::{ChannelMixer, MasterBus, MasterControls, Message as FxMessage, MixerMessage};
use wavegraph_monosynth::{Message as MonoMessage, Monosynth, Tuning, Waveform};
use wavegraph_organ::{Message as OrganMessage, Organ, POLYPHONY, Voicing};
use wavegraph_sample::I24;
use wavegraph_sampler::{PitchedMessage as SampleMessage, PitchedSampler};
use wavegraph_supersaw::{Message as SawMessage, Supersaw};
use wavegraph_wavetable::{Message as WaveMessage, Scan, Wavetable};

const _: () = assert!(wavegraph_wavetable::POLYPHONY == POLYPHONY);
const _: () = assert!(wavegraph_fm::POLYPHONY == POLYPHONY);
const _: () = assert!(wavegraph_sampler::POLYPHONY == POLYPHONY);

const MONO_WAVEFORMS: [(Waveform, &[u8]); 4] = [
    (Waveform::Sine, b"SINE    "),
    (Waveform::Triangle, b"TRIANGLE"),
    (Waveform::Sawtooth, b"SAWTOOTH"),
    (Waveform::Square, b"SQUARE  "),
];

pub(super) const MAX_CUTOFF_STEP: u8 = 66;
// Two semitones per detent, rounded to the nearest Hz from 20 kHz down.
pub(super) const CUTOFF_HZ: [u16; MAX_CUTOFF_STEP as usize + 1] = [
    10, 11, 12, 14, 16, 17, 20, 22, 25, 28, 31, 35, 39, 44, 49, 55, 62, 70, 78, 88, 98, 110, 124,
    139, 156, 175, 197, 221, 248, 278, 313, 351, 394, 442, 496, 557, 625, 702, 787, 884, 992, 1114,
    1250, 1403, 1575, 1768, 1984, 2227, 2500, 2806, 3150, 3536, 3969, 4454, 5000, 5612, 6300, 7071,
    7937, 8909, 10000, 11225, 12599, 14142, 15874, 17818, 20000,
];

/// Keep these discriminants stable for sequencer banks and recorded edits.
/// GLO uses `Part::ALL` for its alphabetical display order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Part {
    #[default]
    Mono,
    Organ,
    Bass,
    Tr909,
    Tr808,
    Supersaw,
    Wavetable,
    Fm,
    Sampler,
}

impl Part {
    /// Keep the GLO instrument list in alphabetical order by its on-screen name.
    pub(super) const ALL: [Self; 9] = [
        Self::Fm,
        Self::Organ,
        Self::Mono,
        Self::Sampler,
        Self::Supersaw,
        Self::Bass,
        Self::Tr808,
        Self::Tr909,
        Self::Wavetable,
    ];

    pub(super) const fn is_kit(self) -> bool {
        matches!(self, Self::Tr808 | Self::Tr909)
    }

    pub(super) const fn is_polyphonic(self) -> bool {
        self.has_voice_modes() || matches!(self, Self::Sampler)
    }

    pub(super) const fn has_voice_modes(self) -> bool {
        matches!(
            self,
            Self::Organ | Self::Supersaw | Self::Wavetable | Self::Fm
        )
    }

    pub(super) const fn uses_house_patterns(self, waveform: u8) -> bool {
        matches!(self, Self::Sampler) && waveform == crate::assets::HOUSE_M7
    }

    pub(super) fn note_on(
        self,
        Note {
            number: note,
            velocity,
        }: Note,
        slide: bool,
    ) -> Message {
        match self {
            Self::Sampler => Message::Sampler(SampleMessage::NoteOn(Note {
                number: note,
                velocity,
            })),
            Self::Mono => MonoMessage::NoteOn { note, velocity }.into(),
            Self::Organ => OrganMessage::NoteOn { note, velocity }.into(),
            Self::Wavetable => WaveMessage::NoteOn { note, velocity }.into(),
            Self::Fm => FmMessage::NoteOn { note, velocity }.into(),
            Self::Supersaw => SawMessage::NoteOn { note, velocity }.into(),
            Self::Bass => BassMessage::NoteOn {
                note,
                accent: velocity >= 100,
                slide,
            }
            .into(),
            Self::Tr808 | Self::Tr909 => unreachable!("drum kits take triggers"),
        }
    }

    pub(super) fn note_off(self, note: u8) -> Message {
        match self {
            Self::Sampler => Message::Sampler(SampleMessage::NoteOff(note)),
            Self::Mono => MonoMessage::NoteOff(note).into(),
            Self::Organ => OrganMessage::NoteOff(note).into(),
            Self::Wavetable => WaveMessage::NoteOff(note).into(),
            Self::Fm => FmMessage::NoteOff(note).into(),
            Self::Supersaw => SawMessage::NoteOff(note).into(),
            Self::Bass => BassMessage::NoteOff(note).into(),
            Self::Tr808 | Self::Tr909 => unreachable!("drum kits take triggers"),
        }
    }

    pub(super) fn drum(self, note: u8) -> Option<Drum> {
        match self {
            Self::Tr808 => {
                wavegraph_808::Voice::from_midi_note(note).map(|v| self.pads()[v as usize])
            }
            Self::Tr909 => Drum::from_midi_note(note),
            _ => None,
        }
    }

    pub(super) const fn pads(self) -> &'static [Drum] {
        match self {
            Self::Tr808 => &[Drum::Kick, Drum::Snare, Drum::ClosedHat, Drum::OpenHat],
            Self::Tr909 => &Drum::ALL,
            _ => &[],
        }
    }

    fn picker_index(self) -> i8 {
        let row = Self::ALL
            .iter()
            .position(|&part| part == self)
            .expect("instrument in GLO picker");
        i8::try_from(row).expect("small instrument catalog")
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A part index plus an i8 fits i16"
    )]
    pub(super) fn turn(&mut self, detents: i8) {
        let index = (i16::from(self.picker_index()) + i16::from(detents))
            .rem_euclid(i16::try_from(Self::ALL.len()).expect("small catalog"));
        *self = Self::ALL[usize::try_from(index).expect("wrapped instrument index")];
    }

    #[cfg(any(target_os = "none", test))]
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Mono => "MONOSYNTH",
            Self::Organ => "M1 ORGAN",
            Self::Bass => "TB-303",
            Self::Tr909 => "TR-909",
            Self::Tr808 => "TR-808",
            Self::Supersaw => "SUPER SAW",
            Self::Wavetable => "WAVETABLE",
            Self::Fm => "FM SYNTH",
            Self::Sampler => "SAMPLER",
        }
    }
}

pub(super) use wavegraph_dsp::midi::Note;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Gate {
    pub(super) note: Option<Note>,
    // Distinguish a fresh strike from legato even when a short gap fits inside one DMA block.
    pub(super) phrase: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Voice {
    pub(super) waveform: u8,
    pub(super) cutoff_step: u8,
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

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A u8 slot plus an i8 detent fits i16; the bank is nonempty."
    )]
    pub(super) fn turn_waveform(&mut self, part: Part, detents: i8) {
        let position = self.waveform.wrapping_add_signed(detents);
        self.waveform = match part {
            Part::Mono => position % 4,
            Part::Sampler => {
                let slots =
                    i16::try_from(crate::assets::SAMPLE_NAMES.len()).expect("small catalog");
                let index = (i16::from(self.waveform) + i16::from(detents)).rem_euclid(slots);
                u8::try_from(index).expect("wrapped sample index")
            }
            Part::Organ | Part::Bass | Part::Wavetable => position % 2,
            Part::Tr808 | Part::Tr909 | Part::Supersaw | Part::Fm => 0,
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
            Part::Supersaw => b"SUPERSAW",
            Part::Wavetable => b"WAVETABLE",
            Part::Fm => b"FM SYNTH",
            Part::Sampler => &crate::assets::SAMPLE_NAMES[usize::from(self.waveform)],
        }
    }

    pub(super) fn sounding(self) -> Option<Note> {
        self.gate.note.map(|mut note| {
            note.number = note.number.saturating_add_signed(self.transpose).min(127);
            note
        })
    }

    pub(super) fn note_name(self) -> [u8; 4] {
        self.sounding()
            .map_or(*b"--  ", |note| Self::pitch_name(note.number))
    }

    pub(super) fn pitch_name(note: u8) -> [u8; 4] {
        const NAMES: [[u8; 2]; 12] = [
            *b"C ", *b"C#", *b"D ", *b"D#", *b"E ", *b"F ", *b"F#", *b"G ", *b"G#", *b"A ", *b"A#",
            *b"B ",
        ];
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
pub(super) struct SawControls {
    pub(super) resonance: u8,
    pub(super) detune: u8,
    pub(super) mix: u8,
    pub(super) filter_envelope: EnvelopeControls,
}

// Only the assigned instrument uses this storage. Every channel snapshot carries a copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Bank {
    None,
    Kit(DrumHits),
    Wave([WaveProgram; 2]),
    Sampler(
        [SampleProgram; crate::assets::SAMPLE_NAMES.len()],
        crate::sampler::Pads,
    ),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DrumHits {
    pub(super) panel: HitLog<8>,
    pub(super) midi: HitLog<32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Track {
    pub(super) lfo: crate::lfo::Controls,
    pub(super) arp: arp::Controls,
    pub(super) arp_revision: u32,
    pub(super) assignment: Assignment,
    pub(super) voice: Voice,
    bank: Bank,
    pub(super) bass: Controls,
    pub(super) inserts: [Insert; INSERT_SLOTS],
    pub(super) polyphony: crate::panel::Polyphony,
    pub(super) organ: OrganControls,
    pub(super) fm: crate::fm::Controls,
    pub(super) saw: SawControls,
    pub(super) poly_keys: [PolyKey; POLYPHONY],
    pub(super) voice_envelope: EnvelopeControls,
    pub(super) envelope_target: EnvelopeTarget,
    pub(super) voice_tap: bool,
    pub(super) drums808: wavegraph_808::Controls,
    pub(super) decay909: [u8; 3],
}

impl Track {
    pub(super) const fn new(assignment: Assignment) -> Self {
        Self {
            lfo: crate::lfo::Controls::INITIAL,
            arp: arp::Controls::INITIAL,
            arp_revision: 0,
            assignment,
            voice: Voice::INITIAL,
            bass: Controls::INITIAL,
            inserts: Insert::factory(assignment.part),
            polyphony: crate::panel::Polyphony::INITIAL,
            organ: OrganControls::INITIAL,
            fm: crate::fm::Controls::INITIAL,
            poly_keys: [PolyKey::EMPTY; POLYPHONY],
            voice_envelope: match assignment.part {
                Part::Supersaw => EnvelopeControls::SUPER_SAW_AMP,
                Part::Wavetable => EnvelopeControls::WAVETABLE,
                _ => EnvelopeControls::INITIAL,
            },
            bank: match assignment.part {
                Part::Tr808 | Part::Tr909 => Bank::Kit(DrumHits {
                    panel: HitLog::EMPTY,
                    midi: HitLog::EMPTY,
                }),
                Part::Wavetable => Bank::Wave([
                    WaveProgram::new(Scan::RESONANT),
                    WaveProgram::new(Scan::VOCAL),
                ]),
                Part::Sampler => Bank::Sampler(SampleProgram::BANK, crate::sampler::Pads::INITIAL),
                _ => Bank::None,
            },
            envelope_target: EnvelopeTarget::Amp,
            saw: SawControls {
                resonance: 0,
                detune: wavegraph_supersaw::DEFAULT_DETUNE,
                mix: wavegraph_supersaw::DEFAULT_MIX,
                filter_envelope: EnvelopeControls::SUPER_SAW_FILTER,
            },
            voice_tap: matches!(
                assignment.part,
                Part::Bass
                    | Part::Tr808
                    | Part::Tr909
                    | Part::Supersaw
                    | Part::Wavetable
                    | Part::Fm
                    | Part::Sampler
            ),
            drums808: wavegraph_808::Controls::INITIAL,
            decay909: [crate::sequencer::DECAY_909; 3],
        }
    }

    #[cfg(test)]
    #[expect(
        clippy::panic,
        reason = "Test helper checks the factory insert invariant"
    )]
    fn bass_fx(&mut self) -> &mut BassFx {
        let Insert::Compressor(fx) = &mut self.inserts[0] else {
            panic!("factory bass insert");
        };
        fx
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
        if self.amen_slices() {
            return;
        }
        match self.assignment.part {
            Part::Mono
            | Part::Organ
            | Part::Supersaw
            | Part::Wavetable
            | Part::Fm
            | Part::Sampler => {
                self.voice.transpose = self.voice.transpose.saturating_add(detents).clamp(-12, 12);
            }
            Part::Bass | Part::Tr808 | Part::Tr909 => (),
        }
    }

    pub(super) fn turn_algorithm(&mut self, view: View, bass_knob: Option<Knob>, detents: i8) {
        if let Some(knob) = bass_knob {
            self.bass.turn(knob, detents);
        } else if self.assignment.part == Part::Fm {
            if view == View::Scope {
                self.fm.select(detents);
            } else if self.envelope_target == EnvelopeTarget::Pitch {
                self.fm.turn_depth(detents);
            }
        } else if self.assignment.part != Part::Sampler || view != View::Envelope {
            self.voice.turn_waveform(self.assignment.part, detents);
        }
    }

    pub(super) fn turn_knobs(&mut self, detents: [i8; 4]) {
        match self.assignment.part {
            Part::Mono => self.voice.turn_cutoff(detents[0]),
            Part::Supersaw => {
                let [cutoff, resonance, detune, mix] = detents;
                self.voice.turn_cutoff(cutoff);
                self.saw.resonance = self.saw.resonance.saturating_add_signed(resonance).min(127);
                self.saw.detune = self.saw.detune.saturating_add_signed(detune).min(127);
                self.saw.mix = self.saw.mix.saturating_add_signed(mix).min(127);
            }
            Part::Organ => self.organ.turn_knobs(detents),
            Part::Fm => self.fm.turn(detents),
            Part::Wavetable => self.wave_program_mut().turn(detents),
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
            Part::Sampler => self.sample_program_mut().turn(detents),
            Part::Tr909 => {
                for (value, delta) in self.decay909.iter_mut().zip(detents) {
                    *value = value.saturating_add_signed(delta).min(100);
                }
            }
        }
    }

    pub(super) fn drum_hits_mut(&mut self) -> Option<&mut DrumHits> {
        if let Bank::Kit(hits) = &mut self.bank {
            Some(hits)
        } else {
            None
        }
    }

    pub(super) fn amen_slices(&self) -> bool {
        self.assignment.part == Part::Sampler
            && self.voice.waveform == 0
            && self.sample_program().playback.mode == wavegraph_sampler::PlaybackMode::Slice
    }

    pub(super) fn sample_pads(&self) -> crate::sampler::Pads {
        match &self.bank {
            Bank::Sampler(_, pads) => *pads,
            _ => crate::sampler::Pads::INITIAL,
        }
    }

    pub(super) fn select_slice(&mut self, note: u8) {
        if self.amen_slices()
            && let Bank::Sampler(_, pads) = &mut self.bank
        {
            pads.select(note);
        }
    }

    pub(super) fn key_transpose(&self) -> i8 {
        if self.amen_slices() {
            0
        } else {
            self.voice.transpose
        }
    }

    pub(super) fn sample_program(&self) -> SampleProgram {
        match &self.bank {
            Bank::Sampler(bank, _) => bank[usize::from(self.voice.waveform)],
            _ => SampleProgram::DRY,
        }
    }

    fn sample_program_mut(&mut self) -> &mut SampleProgram {
        let Bank::Sampler(bank, _) = &mut self.bank else {
            unreachable!("sampler assignment")
        };
        &mut bank[usize::from(self.voice.waveform)]
    }

    fn wave_program_mut(&mut self) -> &mut WaveProgram {
        let Bank::Wave(bank) = &mut self.bank else {
            unreachable!("wavetable assignment")
        };
        &mut bank[usize::from(self.voice.waveform)]
    }

    pub(super) fn edit_panel(&mut self, view: View, select: i8, knobs: [i8; 4], sel: bool) {
        match (view, self.assignment.part) {
            (View::SampleEdit, Part::Sampler) => {
                self.turn_select(select);
                let Bank::Sampler(bank, pads) = &mut self.bank else {
                    unreachable!("sampler editor")
                };
                let slot = self.voice.waveform;
                bank[usize::from(slot)].edit(knobs, (slot == 0).then_some(pads));
            }
            (View::Envelope, Part::Sampler) => {
                self.envelope_target = self
                    .envelope_target
                    .turn(i16::from(select).saturating_add(i16::from(sel)), false);
                let target = self.envelope_target;
                self.sample_program_mut().shape(target).turn(knobs);
                return;
            }
            (View::Envelope, part @ (Part::Supersaw | Part::Wavetable | Part::Fm)) => {
                if part == Part::Fm {
                    self.envelope_target = self
                        .envelope_target
                        .turn(i16::from(select).saturating_add(i16::from(sel)), true);
                    self.fm.turn_envelope(self.envelope_target, knobs);
                } else {
                    if select % 2 != 0 || sel {
                        self.envelope_target = self.envelope_target.turn(1, false);
                    }
                    if self.envelope_target == EnvelopeTarget::Amp {
                        self.voice_envelope.turn(knobs);
                    } else if part == Part::Supersaw {
                        self.saw.filter_envelope.turn(knobs);
                    } else {
                        self.wave_program_mut().position_envelope.turn(knobs);
                    }
                }
                return;
            }
            (View::Envelope, Part::Tr808 | Part::Tr909) => {
                self.turn_knobs(knobs);
                return;
            }
            (View::Envelope, Part::Mono) => self.voice_envelope.turn(knobs),
            (View::Envelope, Part::Organ) => self.organ.envelope.turn_organ(knobs),
            _ => {
                self.turn_select(select);
                self.turn_knobs(knobs);
            }
        }
        if sel {
            self.voice_tap = !self.voice_tap;
        }
    }

    pub(super) fn arp_chord_changed(&self, previous: Option<&Self>) -> bool {
        self.arp.enabled
            && previous.is_none_or(|old| {
                !old.arp.enabled
                    || old.assignment != self.assignment
                    || old.arp_revision != self.arp_revision
            })
    }

    // The direct path releases its gates when the arpeggiator takes ownership.
    fn direct(mut self, offset: KnobOffset) -> Self {
        offset.apply(&mut self);
        if self.arp.enabled {
            self.voice.gate = Gate::default();
            self.poly_keys = [PolyKey::EMPTY; POLYPHONY];
        }
        self
    }

    fn events(self, old: Option<Self>) -> Result<EventBatch, Error> {
        let mut events = match self.assignment.part {
            Part::Mono => self.monophonic_events(old),
            Part::Sampler => self.sampler_events(old),
            Part::Supersaw => {
                let mut events = self.saw_controls(old);
                self.poly_events(old, &mut events);
                events
            }
            Part::Fm => {
                let mut events = EventBatch::new();
                if old.is_none_or(|p| p.fm.patch != self.fm.patch) {
                    events
                        .push(at(FmMessage::Patch(self.fm.patch)))
                        .map_err(|_| Error::Capacity)?;
                }
                self.poly_events(old, &mut events);
                events
            }
            Part::Organ => self.organ_events(old),
            Part::Wavetable => self.wave_events(old),
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
                if self.assignment.part == Part::Tr909
                    && old.map_or([crate::sequencer::DECAY_909; 3], |t| t.decay909) != self.decay909
                {
                    events
                        .push(at(Message::Drum(DrumMessage::Decay909(self.decay909))))
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

    fn mono_controls(self, previous: Option<Self>) -> EventBatch {
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
        if previous.is_none_or(|p| p.voice_envelope != self.voice_envelope) {
            events
                .push(at(MonoMessage::VcaEnvelope(self.voice_envelope.adsr())))
                .expect("one envelope change");
        }
        events
    }

    fn saw_controls(self, previous: Option<Self>) -> EventBatch {
        let mut events = EventBatch::new();
        let mut changed = |differs: bool, message: SawMessage| {
            if differs {
                events.push(at(message)).expect("at most six saw controls");
            }
        };
        changed(
            previous.is_none_or(|p| p.voice.cutoff_step != self.voice.cutoff_step),
            SawMessage::CutoffHz(self.voice.cutoff_hz()),
        );
        changed(
            previous.is_none_or(|p| p.saw.resonance != self.saw.resonance),
            SawMessage::Resonance(self.saw.resonance),
        );
        changed(
            previous.is_none_or(|p| p.saw.detune != self.saw.detune),
            SawMessage::Detune(self.saw.detune),
        );
        changed(
            previous.is_none_or(|p| p.saw.mix != self.saw.mix),
            SawMessage::Mix(self.saw.mix),
        );
        changed(
            previous.is_none_or(|p| p.voice_envelope != self.voice_envelope),
            SawMessage::AmpEnvelope(self.voice_envelope.adsr()),
        );
        changed(
            previous.is_none_or(|p| p.saw.filter_envelope != self.saw.filter_envelope),
            SawMessage::FilterEnvelope(self.saw.filter_envelope.adsr()),
        );
        events
    }

    fn sampler_events(self, previous: Option<Self>) -> EventBatch {
        let mut events = EventBatch::new();
        if previous.is_none_or(|p| p.voice.waveform != self.voice.waveform) {
            events
                .push(at(Message::Sampler(SampleMessage::Select(
                    self.voice.waveform,
                ))))
                .expect("one sample selection");
        }
        let program = self.sample_program();
        let old = previous.map(|p| p.sample_program());
        if old.is_none_or(|p| p.playback != program.playback) {
            events
                .push(at(Message::Sampler(SampleMessage::Playback(
                    program.playback,
                ))))
                .expect("one playback setup");
        }
        if self.amen_slices() {
            let trims = self.sample_pads().trims;
            if previous.is_none_or(|p| !p.amen_slices() || p.sample_pads().trims != trims) {
                events
                    .push(at(Message::Sampler(SampleMessage::SliceTrims(trims))))
                    .expect("one slice setup");
            }
        }
        // A selection can return to a still-ringing clip. Refresh its live tone as well.
        if previous.is_none_or(|p| p.voice.waveform != self.voice.waveform)
            || old.is_none_or(|p| p.timbre != program.timbre)
        {
            events
                .push(at(Message::Sampler(SampleMessage::Timbre(program.timbre))))
                .expect("one sampler tone");
        }
        self.poly_key_events(previous.as_ref(), &mut events);
        events
    }

    fn monophonic_events(self, previous: Option<Self>) -> EventBatch {
        let part = Part::Mono;
        let mut events = self.mono_controls(previous);
        let before = previous.and_then(|p| p.voice.sounding());
        let next = self.voice.sounding();
        let fresh = previous.is_some_and(|p| p.voice.gate.phrase != self.voice.gate.phrase);
        // A release and new strike can both fall between two audio blocks.
        // Deliver the gate edges in order; overlapping keys only change pitch.
        if let Some(last) = before.filter(|_| next.is_none() || fresh) {
            events
                .push(at(part.note_off(last.number)))
                .expect("one note-off");
        }
        if let Some(note) = next.filter(|_| fresh || next != before) {
            events
                .push(at(part.note_on(note, false)))
                .expect("one note-on");
        }
        events
    }

    fn organ_events(self, previous: Option<Self>) -> EventBatch {
        let mut events = self.organ.events(previous.map(|parts| parts.organ));
        self.poly_events(previous, &mut events);
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
    pub(super) fn wave_program(&self) -> WaveProgram {
        match &self.bank {
            Bank::Wave(bank) => bank[usize::from(self.voice.waveform)],
            _ => WaveProgram::new(Scan::RESONANT),
        }
    }

    fn wave_events(self, previous: Option<Self>) -> EventBatch {
        let mut events = EventBatch::new();
        let bank = self.voice.waveform;
        let program = self.wave_program();
        let old_program = previous.map(|p| p.wave_program());
        if previous.is_none_or(|p| p.voice.waveform != bank) {
            events.push(at(WaveMessage::Bank(bank))).expect("one bank");
        }
        if old_program.is_none_or(|p| p.scan != program.scan) {
            events
                .push(at(WaveMessage::Scan(program.scan)))
                .expect("one scan");
        }
        if old_program.is_none_or(|p| p.position_envelope != program.position_envelope) {
            events
                .push(at(WaveMessage::PositionEnvelope(
                    program.position_envelope.adsr(),
                )))
                .expect("one position envelope");
        }
        if previous.is_none_or(|p| p.voice_envelope != self.voice_envelope) {
            events
                .push(at(WaveMessage::Envelope(self.voice_envelope.adsr())))
                .expect("one envelope");
        }
        self.poly_events(previous, &mut events);
        events
    }

    fn poly_events(self, previous: Option<Self>, events: &mut EventBatch) {
        let mode = self.polyphony.mode;
        if previous.map_or(wavegraph_organ::VoiceMode::Poly, |p| p.polyphony.mode) != mode {
            let msg = match self.assignment.part {
                Part::Fm => Message::Fm(FmMessage::VoiceMode(mode)),
                Part::Organ => Message::Organ(OrganMessage::VoiceMode(mode)),
                Part::Wavetable => Message::Wavetable(WaveMessage::VoiceMode(mode)),
                Part::Supersaw => Message::Supersaw(SawMessage::VoiceMode(mode)),
                _ => unreachable!("polyphonic instrument"),
            };
            events.push(at(msg)).expect("one voice mode");
        }
        self.poly_key_events(previous.as_ref(), events);
    }

    fn poly_key_events(&self, previous: Option<&Self>, events: &mut EventBatch) {
        let old_keys = previous.map_or([PolyKey::EMPTY; POLYPHONY], |p| p.poly_keys);
        let old_transpose = previous.map_or(0, Self::key_transpose);
        let now_transpose = self.key_transpose();
        let sounding = |key: PolyKey, transpose: i8| {
            key.note.map(|mut note| {
                note.number = note.number.saturating_add_signed(transpose).min(127);
                note
            })
        };
        let same = |left: PolyKey, right: PolyKey| {
            left.strike == right.strike
                && sounding(left, old_transpose) == sounding(right, now_transpose)
        };
        // Two physical keys can hold the same pitch across an octave change.
        // Release it only when its last key goes up.
        for &old in &old_keys {
            if let Some(note) = sounding(old, old_transpose)
                && !self.poly_keys.iter().any(|&new| {
                    sounding(new, now_transpose).is_some_and(|held| held.number == note.number)
                })
            {
                events
                    .push(at(self.assignment.part.note_off(note.number)))
                    .expect("at most four polyphonic note-offs");
            }
        }
        // A fifth pad can hide an older held one-shot. Uncovering it isn't a new hit.
        let last_strike = previous
            .filter(|p| {
                self.assignment.part == Part::Sampler
                    && matches!(
                        self.sample_program().playback.mode,
                        wavegraph_sampler::PlaybackMode::OneShot
                            | wavegraph_sampler::PlaybackMode::Slice
                    )
                    && p.voice.gate.note.is_some()
            })
            .map(|p| p.voice.gate.phrase);
        // Apply simultaneous strikes oldest first so the newest velocity wins.
        for &new in self.poly_keys.iter().rev() {
            if let Some(note) = sounding(new, now_transpose)
                && last_strike
                    .is_none_or(|last| (1..(1 << 31)).contains(&new.strike.wrapping_sub(last)))
                && !old_keys.iter().any(|&old| same(old, new))
            {
                events
                    .push(at(self.assignment.part.note_on(note, false)))
                    .expect("at most four polyphonic note-ons");
            }
        }
    }
}

// Instruments, mixer and global effects share one message vocabulary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Message {
    Mono(MonoMessage),
    Supersaw(SawMessage),
    Fm(FmMessage),
    Wavetable(WaveMessage),
    Sampler(SampleMessage),
    Recording(Option<crate::clipboard::Take>),
    WaveFrame(u8),
    Organ(OrganMessage),
    Bass(BassMessage),
    BassCycle,
    Mixer(MixerMessage),
    Fx(FxMessage),
    Assign(Assignment),
    Drum(DrumMessage),
    DrumLamps([DrumLamp; 6]),
    Insert(Channel, Part),
    ChannelInserts(Channel, [Insert; INSERT_SLOTS]),
    Master(MasterControls),
    GainReduction(u8),
    Cue(Cue),
}

impl From<MonoMessage> for Message {
    fn from(value: MonoMessage) -> Self {
        Self::Mono(value)
    }
}
impl From<WaveMessage> for Message {
    fn from(value: WaveMessage) -> Self {
        Self::Wavetable(value)
    }
}
impl From<FmMessage> for Message {
    fn from(value: FmMessage) -> Self {
        Self::Fm(value)
    }
}
impl From<SawMessage> for Message {
    fn from(value: SawMessage) -> Self {
        Self::Supersaw(value)
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

// Up to eight polyphonic gate edges, six Super Saw controls and one voice mode.
const MAX_EVENTS: usize = 16;
pub(super) type EventBatch = Vec<Event<Message>, MAX_EVENTS>;
// The 909 accepts nine MIDI pitches across six voices. Clip strikes on alternate
// pitches can coincide with overdubs on the other voices, a full live burst,
// and frame-zero assignment or release events. Storage is off the IRQ stack.
const MAX_ROUTED_EVENTS: usize = 32 + 8 + 9 + 2 + 1; // Optional recording replacement.
pub(super) type RoutedEvents = Vec<Event<Message>, MAX_ROUTED_EVENTS>;

// Only the assigned engine occupies mutable storage on each channel.
#[allow(
    clippy::large_enum_variant,
    reason = "All engines use inline storage on the FM-1"
)]
enum Player<'a> {
    Empty,
    Mono(Monosynth<'a>),
    Supersaw(Supersaw<'a>),
    Fm(Fm<'a>),
    Wavetable(Wavetable<'a>),
    Sampler(PitchedSampler<'a>),
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
                Part::Wavetable => Player::Wavetable(
                    Wavetable::new(self.tuning, &crate::assets::WAVETABLES)
                        .expect("factory bank catalog"),
                ),
                Part::Fm => Player::Fm(Fm::new(self.tuning)),
                Part::Supersaw => Player::Supersaw(Supersaw::new(self.tuning)),
                Part::Sampler => Player::Sampler(
                    PitchedSampler::at_sample_rate(&crate::assets::RAVE_BANK, self.sample_rate)
                        .expect("factory sample bank and audio clock"),
                ),
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
            Player::Fm(node) => dispatch(node, cx, |m| match m {
                Message::Fm(m) => Some(m),
                _ => None,
            }),
            Player::Sampler(node) => {
                for event in cx.events {
                    if let Message::Recording(take) = event.message {
                        node.replace_recording(take.map(crate::clipboard::Take::clip));
                    }
                }
                dispatch(node, cx, |m| match m {
                    Message::Sampler(message) => Some(message),
                    _ => None,
                });
            }
            Player::Supersaw(node) => dispatch(node, cx, |m| match m {
                Message::Supersaw(m) => Some(m),
                _ => None,
            }),
            Player::Wavetable(node) => {
                dispatch(node, cx, |m| match m {
                    Message::Wavetable(m) => Some(m),
                    _ => None,
                });
                if let Some(last) = cx.frames.checked_sub(1) {
                    let _ = (cx.emit)(Event {
                        frame: last,
                        port: 2,
                        message: Message::WaveFrame(node.frame()),
                    });
                }
            }
            Player::Organ(node) => dispatch(node, cx, |m| match m {
                Message::Organ(m) => Some(m),
                _ => None,
            }),
            Player::Bass(node) => {
                dispatch(node, cx, |m| match m {
                    Message::Bass(m) => Some(m),
                    _ => None,
                });
                if let Some(frame) = node.last_cycle_start() {
                    let _ = (cx.emit)(Event {
                        frame,
                        port: 2,
                        message: Message::BassCycle,
                    });
                }
            }
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

/// Restore dry headroom and process both wet returns in the final mix.
pub(super) struct Master(pub(super) MasterBus);

impl Process<Message, I24> for Master {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal; 3]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal, Port::Message]
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Dry, two returns and the cue sum to at most eleven Q23 rails"
    )]
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        // The firmware publishes all rack edits and the splash cue at frame zero.
        let mut cue = Cue::EMPTY;
        for event in cx.events {
            match event.message {
                Message::Master(control) => self.0.edit(control),
                Message::Cue(samples) => cue = samples,
                _ => (),
            }
        }
        if cx.frames == 0 {
            return;
        }
        let mut mix = cx.inputs[0].iter().zip(cx.inputs[1]).zip(cx.inputs[2]).map(
            |((dry, reverb), delay)| (dry.inner() << DRY_HEADROOM) + reverb.inner() + delay.inner(),
        );
        let output = &mut *cx.outputs[0];
        let (intro, rest) = output.split_at_mut(cue.len.min(output.len()));
        for ((out, sum), sample) in intro.iter_mut().zip(&mut mix).zip(cue.samples()) {
            *out = self.0.process_sample(sum + (i32::from(sample) << 8));
        }
        for (out, sum) in rest.iter_mut().zip(mix) {
            *out = self.0.process_sample(sum);
        }
        // One Q7 meter reading per block; the LCD converts it to dB.
        let reduction = ((1.0 - self.0.dynamics_gain()) * 128.0 + 0.5) as u8;
        if let Some(frame) = cx.frames.checked_sub(1) {
            let _ = (cx.emit)(Event {
                frame,
                port: 1,
                message: Message::GainReduction(reduction),
            });
        }
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
pub(super) const DRY_HEADROOM: u8 = 3;
const MASTER_BUS: usize = 18;
pub(super) const BUS_COUNT: usize = MASTER_BUS + 2;
impl Channel {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Three times signed Q23 fits in i32."
    )]
    pub(super) fn copy_scope<const N: usize>(
        self,
        part: Part,
        view: View,
        buses: &[[I24; N]; BUS_COUNT],
        out: &mut [i32; N],
    ) {
        // Give drum transients and the detuned saws headroom on the display.
        // Apply this gain only to the selected strip's scope tap.
        let bus = if view == View::Fx {
            STRIP_BUS + self.index()
        } else {
            self.source_bus()
        };
        let scale = |s: I24| match part {
            Part::Tr909 => s.inner() >> 4,
            Part::Tr808 => s.inner() >> 3,
            Part::Sampler => s.inner() >> 2,
            Part::Bass | Part::Wavetable if view == View::Fx => s.inner() >> 3,
            Part::Supersaw => (s.inner() * 3) >> 3,
            _ => s.inner(),
        };
        for (dst, &sample) in out.iter_mut().zip(&buses[bus]) {
            *dst = scale(sample);
        }
    }

    #[cfg(test)]
    fn scope_samples<const N: usize>(
        self,
        part: Part,
        view: View,
        buses: &[[I24; N]; BUS_COUNT],
    ) -> [i32; N] {
        let mut samples = [0; N];
        self.copy_scope(part, view, buses, &mut samples);
        samples
    }
}

type Plan = Program<'static, 11, 8>;
pub(super) type AudioProcess<'a> = dyn Process<Message, I24> + Send + 'a;
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
            1, 8, 3, 1, 13, 16, 17, // CALL master EQ and dynamics on dry + returns -> 18
            2, 18, 64, // GAIN optional output trim -> 19
        ];
        const FULL_LEN: usize = ROUTE.len() - 3;
        Ok(Self([
            Plan::decode(&ROUTE[..FULL_LEN])?,
            Plan::decode(ROUTE)?,
        ]))
    }

    pub(super) fn bind<'a>(
        &'a self,
        nodes: &'a mut [&'a mut AudioProcess<'a>],
        events: &'a mut [RoutedEvents; CHANNELS],
        rate: (u32, u32),
    ) -> Result<Instrument<'a>, Error> {
        #[allow(
            clippy::cast_precision_loss,
            reason = "Single precision is sufficient for the modulation clock."
        )]
        let lfo =
            wavegraph_lfo::Lfo::new(rate.0 as f32 / rate.1 as f32).expect("audio sample clock");
        Ok(Instrument {
            lfos: [lfo; CHANNELS],
            offsets: [KnobOffset::default(); CHANNELS],
            recorder: crate::clipboard::Recorder::new(rate),
            recording_request: crate::clipboard::Request::INITIAL,
            previous_take: None,
            programs: self,
            patch: self.0[0].bind_nodes(nodes)?,
            half_gain: false,
            mixer: Mixer::DRY,
            previous_mixer: None,
            previous_bpm: 0,
            fx: Fx::INITIAL,
            cue: Cue::EMPTY,
            previous_fx: None,
            transport: Transport::INITIAL,
            sequences: Sequencer::INITIAL.enabled,
            player: PatternPlayer::new(rate),
            arps: core::array::from_fn(|_| ChannelArp::new(rate)),
            events,
            position: None,
            drums: [[DrumLamp::Off; 6]; CHANNELS],
            wave_frames: [0; CHANNELS],
            bass_cycles: [None; CHANNELS],
            reduction: 0,
            previous: None,
            gap_events: false,
        })
    }
}

pub(super) struct Instrument<'a> {
    pub(super) recorder: crate::clipboard::Recorder,
    pub(super) recording_request: crate::clipboard::Request,
    previous_take: Option<crate::clipboard::Take>,
    programs: &'a Programs,
    patch: Patch<'a, 'a, Message, I24, 8, AudioProcess<'a>>,
    pub(super) half_gain: bool,
    mixer: Mixer,
    previous_mixer: Option<Mixer>,
    previous_bpm: u16,
    fx: Fx,
    pub(super) cue: Cue,
    previous_fx: Option<Fx>,
    pub(super) transport: Transport,
    pub(super) sequences: [bool; CHANNELS],
    player: PatternPlayer,
    lfos: [wavegraph_lfo::Lfo; CHANNELS],
    offsets: [KnobOffset; CHANNELS],
    arps: [ChannelArp; CHANNELS],
    events: &'a mut [RoutedEvents; CHANNELS],
    pub(super) position: Option<Position>,
    pub(super) drums: [[DrumLamp; 6]; CHANNELS],
    pub(super) wave_frames: [u8; CHANNELS],
    pub(super) bass_cycles: [Option<usize>; CHANNELS],
    pub(super) reduction: u8,
    previous: Option<[Track; CHANNELS]>,
    gap_events: bool,
}

impl Instrument<'_> {
    pub(super) fn sync_chord(
        &mut self,
        channel: Channel,
        track: &Track,
        chord: &wavegraph_arp::Chord,
    ) {
        let i = channel.index();
        if track.arp_chord_changed(self.previous.as_ref().map(|last| &last[i])) {
            self.arps[i].chord.clone_from(chord);
        }
    }

    pub(super) fn arp_note(&self, channel: Channel) -> Option<u8> {
        let track = &self.previous.as_ref()?[channel.index()];
        self.arps[channel.index()]
            .clock
            .sounding()
            .map(|n| n.saturating_add_signed(track.key_transpose()).min(127))
    }

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

    pub(super) fn bass_automation(&self, channel: Channel) -> Option<(Channel, Automation)> {
        self.player
            .automation(channel)
            .map(|controls| (channel, controls))
    }

    pub(super) fn banks(&self) -> [BankPosition; CHANNELS] {
        self.player.banks()
    }

    pub(super) fn select_patterns(&mut self, slots: [Slot; CHANNELS]) {
        self.player.requested = slots;
    }

    /// Advance the clip and arp clocks without rendering inaudible DSP blocks.
    /// Parking the clip lanes releases their old gates; re-enabling them in
    /// the next render chases the controllers at the new playhead.
    pub(super) fn skip_frames(
        &mut self,
        frames: usize,
        mut remember_cc: impl FnMut(Channel, u8, u8),
    ) -> Result<(), Error> {
        let Some(tracks) = &self.previous else {
            return Ok(());
        };
        if frames == 0 {
            return Ok(());
        }
        self.recorder.interrupt();
        for events in self.events.iter_mut() {
            events.clear();
        }
        self.player.render(
            frames,
            self.transport,
            tracks,
            [false; CHANNELS],
            |c, event| {
                // A held synth gate needs a release. Drum one-shots and FX tails
                // can resume from their current state when the DAC catches up.
                if !matches!(event.message, Message::Drum(DrumMessage::Stop)) {
                    self.events[c.index()]
                        .push(event)
                        .map_err(|_| Error::Capacity)?;
                }
                Ok(())
            },
            |c, event| {
                // Parking a lane releases slide and restores its panel controls.
                // Preserve those values even if STOP or mute prevents a later chase.
                if let MidiMessage::Control(cc, value) = event.message {
                    remember_cc(c, cc, value);
                }
            },
        )?;
        for (i, arp) in self.arps.iter_mut().enumerate() {
            let track = &tracks[i];
            if track.lfo.enabled {
                self.lfos[i].advance(frames);
            }
            if track.arp.enabled
                && !arp.chord.is_empty()
                && let Some(note) =
                    arp.clock
                        .skip(&arp.chord, track.arp.config, self.transport.bpm, frames)
            {
                arp::route(
                    track,
                    Event {
                        frame: 0,
                        port: 0,
                        message: MidiMessage::Off(note),
                    },
                    &mut self.events[i],
                    |_| {},
                )?;
            }
        }
        self.gap_events = true;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn render<'b, const N: usize>(
        &mut self,
        tracks: [Track; CHANNELS],
        buses: &'b mut [[I24; N]; BUS_COUNT],
    ) -> Result<&'b [I24; N], Error> {
        self.render_with_midi(&tracks, buses, |_, _| ())
    }

    pub(super) fn render_with_midi<'b, const N: usize>(
        &mut self,
        tracks: &[Track; CHANNELS],
        buses: &'b mut [[I24; N]; BUS_COUNT],
        mut wire: impl FnMut(Channel, Event<MidiMessage>),
    ) -> Result<&'b [I24; N], Error> {
        let downbeat = self.transport.playing() && self.player.transport_changed(self.transport);
        let mut inserts = Vec::<_, 8>::new();
        let mut enabled = self.sequences;
        let mut live_counts = [0; CHANNELS];
        let mut offsets = [KnobOffset::default(); CHANNELS];
        let recovering = core::mem::take(&mut self.gap_events);
        for (i, events) in self.events.iter_mut().enumerate() {
            let track = tracks[i];
            let part = track.assignment.part;
            let previous = self.previous.as_ref().map(|tracks| tracks[i]);
            let old = previous.filter(|old| old.assignment == track.assignment);
            if old.is_none() {
                inserts
                    .push(at(Message::Insert(Channel::ALL[i], part)))
                    .map_err(|_| Error::Capacity)?;
            }
            if old.is_none_or(|t| t.inserts != track.inserts) {
                inserts
                    .push(at(Message::ChannelInserts(Channel::ALL[i], track.inserts)))
                    .map_err(|_| Error::Capacity)?;
            }
            // Each synth's live gate takes over its own sequencer lane.
            enabled[i] &= if track.arp.enabled {
                self.arps[i].chord.is_empty()
            } else {
                part.is_kit() || track.voice.gate.note.is_none()
            };
            let c = track.lfo;
            let was_enabled = old.is_some_and(|t| t.lfo.enabled);
            let amount = c.tick(
                &mut self.lfos[i],
                self.transport.bpm,
                !was_enabled || (c.sync && downbeat),
                N,
            );
            let mut offset = KnobOffset::default();
            let mut bass_mod = None;
            if let Some(amount) =
                amount.or_else(|| (was_enabled && !c.enabled && part == Part::Bass).then_some(0.0))
            {
                match c.target(part).route {
                    Route::Bass(target) => bass_mod = Some(BassMessage::Modulate(target, amount)),
                    Route::Knob { index, span } => offset = KnobOffset::new(index, span, amount),
                }
            }
            let mut panel = track
                .direct(offset)
                .events(old.map(|t| t.direct(self.offsets[i])))?;
            offsets[i] = offset;
            if part == Part::Sampler && (old.is_none() || self.previous_bpm != self.transport.bpm) {
                panel
                    .push(at(Message::Sampler(SampleMessage::Tempo(
                        self.transport.bpm,
                    ))))
                    .map_err(|_| Error::Capacity)?;
            }
            if enabled[i]
                && old.is_some()
                && let Some(owned) = self.player.automation(Channel::ALL[i])
            {
                panel.retain(|event| match event.message {
                    Message::Bass(message) => {
                        Knob::from_message(message).is_none_or(|cc| owned.get(cc).is_none())
                    }
                    _ => true,
                });
            }
            if !recovering || old.is_none() {
                events.clear();
            }
            events
                .extend_from_slice(&panel)
                .map_err(|_| Error::Capacity)?;
            if part == Part::Sampler
                && (old.is_none() || self.recorder.status.take != self.previous_take)
            {
                events
                    .push(at(Message::Recording(self.recorder.status.take)))
                    .map_err(|_| Error::Capacity)?;
            }
            if let Some(message) = bass_mod {
                events.push(at(message)).map_err(|_| Error::Capacity)?;
            }
            if let Some(last) = previous.filter(|t| t.arp.enabled)
                && (!track.arp.enabled
                    || old.is_none()
                    || self.arps[i].chord.is_empty()
                    || last.voice.transpose != track.voice.transpose)
            {
                self.arps[i].clock.stop(|n| {
                    arp::route(
                        &last,
                        Event {
                            frame: 0,
                            port: 0,
                            message: MidiMessage::Off(n),
                        },
                        events,
                        |e| wire(Channel::ALL[i], e),
                    )
                })?;
            }
            if let Bank::Kit(hits) = &track.bank
                && !track.arp.enabled
            {
                let old_hits = old.as_ref().and_then(|t| match &t.bank {
                    Bank::Kit(hits) => Some(hits),
                    _ => None,
                });
                hits.panel
                    .append_since(old_hits.map_or(0, |h| h.panel.total()), events)?;
                hits.midi
                    .append_since(old_hits.map_or(0, |h| h.midi.total()), events)?;
            }
            live_counts[i] = events.len();
        }
        let mut player = self.player;
        player.render(
            N,
            self.transport,
            tracks,
            enabled,
            |channel, event| {
                self.events[channel.index()]
                    .push(event)
                    .map_err(|_| Error::Capacity)
            },
            &mut wire,
        )?;
        for (i, (events, live)) in self.events.iter_mut().zip(live_counts).enumerate() {
            merge_live_events(events, live);
            let track = &tracks[i];
            let arp = &mut self.arps[i];
            // A held or latched chord owns this lane, so the only clip events
            // in this buffer are its frame-zero release and controller restore.
            if track.arp.enabled && !arp.chord.is_empty() {
                arp.clock
                    .render(&arp.chord, track.arp.config, self.transport.bpm, N, |e| {
                        arp::route(track, e, events, |e| wire(Channel::ALL[i], e))
                    })?;
            }
        }
        let mixer_events = self.mixer.events(self.previous_mixer);
        let reverb = self.fx.return_events(self.previous_fx, Device::Reverb);
        let delay = self.fx.return_events(self.previous_fx, Device::Delay);
        let mut master = self.fx.master_events(self.previous_fx);
        if self.cue.len != 0 {
            master
                .push(at(Message::Cue(self.cue)))
                .map_err(|_| Error::Capacity)?;
        }
        let mut drums = [[DrumLamp::Off; 6]; CHANNELS];
        let mut wave_frames = [0; CHANNELS];
        let mut bass_cycles = [None; CHANNELS];
        let mut reduction = 0;
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
                &master,
            ],
            |object, event| match event.message {
                Message::GainReduction(value) => {
                    reduction = value;
                    Ok(())
                }
                Message::WaveFrame(index) => {
                    wave_frames[usize::from(object)] = index;
                    Ok(())
                }
                Message::BassCycle => {
                    bass_cycles[usize::from(object)] = Some(event.frame);
                    Ok(())
                }
                Message::DrumLamps(lamps) => {
                    drums[usize::from(object)] = lamps;
                    Ok(())
                }
                _ => Err(event),
            },
        )?;
        self.previous_take = self.recorder.status.take;
        self.recorder.process(
            self.recording_request,
            tracks,
            self.transport,
            player.downbeat,
            buses,
        );
        self.player = player;
        self.position = player.position();
        self.drums = drums;
        self.wave_frames = wave_frames;
        self.bass_cycles = bass_cycles;
        self.reduction = reduction;
        self.offsets = offsets;
        self.previous = Some(*tracks);
        self.previous_mixer = Some(self.mixer);
        self.previous_bpm = self.transport.bpm;
        self.previous_fx = Some(self.fx);
        self.cue = Cue::EMPTY;
        Ok(&buses[if self.half_gain {
            MASTER_BUS + 1
        } else {
            MASTER_BUS
        }])
    }
}

/// Merge panel changes at frame zero with the already time-ordered clip stream.
fn merge_live_events(events: &mut [Event<Message>], live: usize) {
    // On a takeover, release the old gate and apply controls before a live strike.
    // Only sort the live prefix. Same-tick MIDI events retain their file order.
    for i in 1..live {
        let mut j = i;
        while let Some(prev) = j.checked_sub(1) {
            if event_order(&events[j]) >= event_order(&events[prev]) {
                break;
            }
            events.swap(j, prev);
            j = prev;
        }
    }
    let mut left = 0;
    let mut right = live;
    while left < right && right < events.len() && events[right].frame == 0 {
        if event_order(&events[right]) < event_order(&events[left]) {
            events[left..=right].rotate_right(1);
            right = right.saturating_add(1);
        }
        left = left.saturating_add(1);
    }
}

fn event_order(event: &Event<Message>) -> u8 {
    match event.message {
        Message::Assign(_) => 0,
        Message::Mono(MonoMessage::NoteOff(_))
        | Message::Wavetable(WaveMessage::NoteOff(_))
        | Message::Fm(FmMessage::NoteOff(_))
        | Message::Supersaw(SawMessage::NoteOff(_))
        | Message::Organ(OrganMessage::NoteOff(_))
        | Message::Sampler(SampleMessage::NoteOff(_) | SampleMessage::Stop)
        | Message::Bass(BassMessage::NoteOff(_) | BassMessage::Reset)
        | Message::Drum(DrumMessage::Stop) => 1,
        Message::Mono(MonoMessage::NoteOn { .. })
        | Message::Wavetable(WaveMessage::NoteOn { .. })
        | Message::Fm(FmMessage::NoteOn { .. })
        | Message::Supersaw(SawMessage::NoteOn { .. })
        | Message::Organ(OrganMessage::NoteOn { .. })
        | Message::Sampler(SampleMessage::NoteOn(_))
        | Message::Bass(BassMessage::NoteOn { .. })
        | Message::Drum(DrumMessage::Trigger(_, _)) => 3,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    mod arpeggiator;
    mod modulation;
    mod sampler;
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
    const BASS_INSERT: usize = STRIP_BUS + Channel::Three.index();
    const FX_WITHOUT_DYNAMICS: Fx = Fx {
        dynamics_enabled: false,
        ..Fx::INITIAL
    };

    fn select_instrument(nav: &mut Navigation, target: Part) {
        nav.turn(
            0,
            target
                .picker_index()
                .saturating_sub(nav.part().picker_index()),
        );
    }

    #[test]
    fn edit_page_cycles_each_polysynth_and_recalls_its_chord() {
        use crate::panel::PageButton;
        use wavegraph_organ::{Chord, VoiceMode};
        for part in [Part::Organ, Part::Wavetable, Part::Supersaw, Part::Fm] {
            let mut nav = Navigation::INITIAL;
            nav.assignments[0] = Assignment::new(part);
            let mut track = Track::new(nav.assignments[0]);
            nav.press(PageButton::Edit);
            assert_eq!(nav.view, View::VoiceMode);
            let old = track;
            track.polyphony.edit(1, 2, false, 0);
            let mode = VoiceMode::Chord(Chord::MAJOR.turn(2));
            let msg = match part {
                Part::Fm => at(FmMessage::VoiceMode(mode)),
                Part::Organ => at(OrganMessage::VoiceMode(mode)),
                Part::Supersaw => at(SawMessage::VoiceMode(mode)),
                _ => at(WaveMessage::VoiceMode(mode)),
            };
            assert_eq!(
                track.events(Some(old)).expect("mode edit").as_slice(),
                &[msg]
            );
            track.polyphony.edit(0, 0, true, 0);
            assert_eq!(track.polyphony.mode, VoiceMode::Unison);
            track.polyphony.edit(0, 0, true, 0);
            assert_eq!(track.polyphony.mode, VoiceMode::Poly);
            track.polyphony.edit(0, 0, true, 0);
            assert_eq!(track.polyphony.mode, mode);
            assert_eq!(crate::panel::Polyphony::label(mode), "Maj7");
            track.polyphony.edit(-2, 9, false, 0);
            assert_eq!(track.polyphony.mode, VoiceMode::Unison);
            track.polyphony.edit(-1, 0, false, 1);
            assert_eq!(crate::panel::Polyphony::label(track.polyphony.mode), "m7");
            track.polyphony.edit(0, -1, false, 0);
            assert_eq!(track.polyphony.mode, mode);
            nav.press(PageButton::Edit);
            assert_eq!(nav.view, View::Scope);
            nav.press(PageButton::Edit);
            nav.turn(0, 1);
            assert_eq!((nav.part(), nav.view), (Part::Bass, View::Scope));
        }
    }

    #[test]
    fn lfo_page_and_mod_wheel_follow_the_assigned_bass_channel() -> Result<(), Error> {
        use crate::panel::PageButton;
        use wavegraph_303::Destination;
        let mut nav = Navigation::INITIAL;
        nav.press(PageButton::Lfo);
        assert_eq!(nav.view, View::Lfo);
        nav.turn(0, 1);
        assert_eq!((nav.view, nav.focus), (View::Lfo, Channel::Two));
        nav.press(PageButton::Lfo);
        assert_eq!(nav.view, View::Scope);
        nav.press(PageButton::Lfo);
        nav.turn(0, 1);
        nav.press(PageButton::Lfo);
        assert_eq!((nav.view, nav.focus), (View::Scope, Channel::Three));

        with_instrument(8192, |rack| {
            let mut tracks = Track::rack(nav.assignments);
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut keys = Keys::<1>::new();
            // Square -> cutoff on bass 2; the wheel must also work off-page.
            tracks[1].lfo.edit(Part::Bass, 0, [3, 0, 0, 1], true);
            for (status, value) in [(0xb1, 127), (0xb2, 0)] {
                let (ch, cc) = decode_midi([11, status, 1, value]).expect("CC1");
                keys.midi(ch, cc, &mut tracks[ch.index()]);
            }
            assert_eq!(tracks[0].lfo, crate::lfo::Controls::INITIAL);
            assert_eq!(tracks[2].lfo.depth, 0);
            assert!(!tracks[2].lfo.enabled);
            rack.render(tracks, &mut buses)?;
            assert!(rack.events[1].contains(&at(BassMessage::Modulate(Destination::Cutoff, 1.0))));
            tracks[1].lfo.edit(Part::Bass, 0, [0; 4], true);
            rack.render(tracks, &mut buses)?;
            assert!(rack.events[1].contains(&at(BassMessage::Modulate(Destination::Cutoff, 0.0))));
            Ok(())
        })
    }

    #[test]
    fn synced_lfo_counts_missed_frames_and_restarts_on_play() -> Result<(), Error> {
        with_instrument(8192, |rack| {
            let mut tracks = Track::rack(Navigation::INITIAL.assignments);
            tracks[0].lfo.edit(Part::Bass, 1, [2, 0, 127, 0], true);
            rack.transport.bpm = 120;
            rack.sequences = [false; CHANNELS];
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let pitch = |amount| {
                at(BassMessage::Modulate(
                    wavegraph_303::Destination::Pitch,
                    amount,
                ))
            };
            rack.render(tracks, &mut buses)?;
            assert!(rack.events[0].contains(&pitch(-1.0)));
            rack.skip_frames(896, |_, _, _| {})?;
            rack.render(tracks, &mut buses)?;
            assert!(rack.events[0].contains(&pitch(-0.5)));
            rack.transport.toggle();
            rack.render(tracks, &mut buses)?;
            assert!(rack.events[0].contains(&pitch(-1.0)));
            Ok(())
        })
    }

    fn note(number: u8, velocity: u8) -> Note {
        Note { number, velocity }
    }

    fn check_factory_slide_recovery(
        blocks: usize,
        change: impl Fn(&mut Instrument<'_>),
        expected: u8,
    ) -> Result<(), Error> {
        use crate::{
            bass::CC_SLIDE,
            midi_out::{Batch, OutputQueue},
        };
        use fm1_audio::OUTPUT_FRAMES;

        for gap in [false, true] {
            with_instrument(44_118, |engine| {
                let tracks = Track::rack(Navigation::INITIAL.assignments);
                let mut buses = [[I24::default(); OUTPUT_FRAMES]; BUS_COUNT];
                let mut out = OutputQueue::default();
                out.connect(true);
                let session = out.session().expect("connected");
                let mut batch = Batch::default();
                let mut slide = None;
                let mut render =
                    |engine: &mut Instrument<'_>, out: &mut OutputQueue, batch: &mut Batch| {
                        batch.begin(session);
                        engine.render_with_midi(&tracks, &mut buses, |c, e| batch.push(c, e))?;
                        out.append(batch);
                        let mut packets = [[0; 4]; 16];
                        loop {
                            let (revision, count) = out.peek(&mut packets);
                            if count == 0 {
                                break;
                            }
                            for &[_, status, cc, value] in &packets[..count] {
                                if (status, cc) == (0xb0, CC_SLIDE) {
                                    slide = Some(value);
                                }
                            }
                            out.sent(revision, count);
                        }
                        Ok::<_, Error>(slide)
                    };
                engine.transport.toggle();
                let mut receiver = None;
                for _ in 0..1000 {
                    receiver = render(engine, &mut out, &mut batch)?;
                    if receiver == Some(127) {
                        break;
                    }
                }
                assert_eq!(receiver, Some(127), "first factory slide on channel 1");

                if gap {
                    out.silence();
                    let frames = blocks.checked_mul(OUTPUT_FRAMES).expect("test frame count");
                    engine.skip_frames(frames, |c, cc, value| {
                        batch.remember_cc(c, cc, value);
                    })?;
                } else {
                    for _ in 0..blocks {
                        render(engine, &mut out, &mut batch)?;
                    }
                }
                change(engine);
                assert_eq!(
                    render(engine, &mut out, &mut batch)?,
                    Some(expected),
                    "blocks = {blocks}, gap = {gap}"
                );
                Ok(())
            })?;
        }
        Ok(())
    }

    #[test]
    fn skipped_factory_slide_off_reaches_the_midi_receiver() -> Result<(), Error> {
        // The slide is still on after one block and releases within six.
        check_factory_slide_recovery(1, |_| {}, 127)?;
        check_factory_slide_recovery(6, |_| {}, 0)
    }

    #[test]
    fn stopping_during_a_gap_leaves_slide_off() -> Result<(), Error> {
        check_factory_slide_recovery(6, |engine| engine.transport.toggle(), 0)
    }

    #[test]
    fn muting_during_a_gap_leaves_slide_off() -> Result<(), Error> {
        check_factory_slide_recovery(
            6,
            |engine| engine.sequences[Channel::One.index()] = false,
            0,
        )
    }

    #[test]
    fn dropped_time_keeps_the_next_factory_kick_and_note_on_the_dac_timeline() -> Result<(), Error>
    {
        extern crate alloc;
        use alloc::vec::Vec as TestVec;
        type Playback = (TestVec<(usize, Channel, u8)>, TestVec<Option<Position>>);
        fn run(gap: bool) -> Result<Playback, Error> {
            let mut attacks = TestVec::new();
            let mut positions = TestVec::new();
            with_instrument(44_118, |engine| {
                let tracks = Track::rack(Navigation::INITIAL.assignments);
                let mut buses = [[I24::default(); 128]; BUS_COUNT];
                engine.transport.toggle();
                engine.render(tracks, &mut buses)?;
                // This span contains a gate release and a kick. Discard their
                // output and compare the following bar to uninterrupted playback.
                const BLOCKS: usize = 350;
                if gap {
                    engine.skip_frames(BLOCKS * 128, |_, _, _| {})?;
                    assert!(
                        engine
                            .events
                            .iter()
                            .flatten()
                            .any(|e| matches!(e.message, Message::Bass(BassMessage::NoteOff(_))))
                    );
                } else {
                    let (mut off, mut kick) = (false, false);
                    for _ in 0..BLOCKS {
                        engine.render_with_midi(&tracks, &mut buses, |c, e| match e.message {
                            MidiMessage::Off(_) => off = true,
                            MidiMessage::On(n) if c == Channel::Three && n.number == 36 => {
                                kick = true
                            }
                            _ => (),
                        })?;
                    }
                    assert!(off && kick);
                }
                for block in 0..400 {
                    engine.render_with_midi(&tracks, &mut buses, |c, e| {
                        if let MidiMessage::On(n) = e.message {
                            attacks.push((block * 128 + e.frame, c, n.number));
                        }
                    })?;
                    positions.push(engine.position);
                }
                Ok(())
            })?;
            Ok((attacks, positions))
        }
        let continuous = run(false)?;
        assert!(
            continuous
                .0
                .iter()
                .any(|&(_, c, n)| c == Channel::Four && n == 36)
        );
        assert_eq!(run(true)?, continuous);
        Ok(())
    }

    #[test]
    fn merging_live_controls_preserves_same_tick_midi_order() {
        let cutoff = at(BassMessage::CutoffHz(800.0));
        let off = at(BassMessage::NoteOff(38));
        let on = at(BassMessage::NoteOn {
            note: 41,
            accent: true,
            slide: false,
        });
        let resonance = at(BassMessage::Resonance(0.75));
        let mut events = [cutoff, off, on, resonance];
        merge_live_events(&mut events, 1);
        assert_eq!(events, [off, cutoff, on, resonance]);
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
            self.piano.event(
                self.nav.focus,
                self.nav.assignments[self.nav.focus.index()],
                MidiAction::On(note),
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

        fn poly_keys(&self) -> [PolyKey; POLYPHONY] {
            self.tracks[1].poly_keys
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
        let mut mixer = FxMixer(
            ChannelMixer::new()
                .with_boost()
                .with_dry_headroom(DRY_HEADROOM),
        );
        let mut master = Master(MasterBus::new(rate_hz).expect("sample clock"));
        let mut storage = [0_i32; wavegraph_fx::REVERB_SAMPLES];
        let mut reverb = FxNode(Reverb::new(&mut storage));
        let mut line = [0; 44_118];
        let mut delay = FxNode(Delay::new(&mut line, rate_hz).expect("delay sample clock"));
        let mut inserts = crate::inserts::ChannelInserts::new(rate_hz);
        let mut nodes: [&mut AudioProcess<'_>; 9] = [
            one,
            two,
            three,
            four,
            &mut mixer,
            &mut reverb,
            &mut delay,
            &mut inserts,
            &mut master,
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
        assert_eq!(keys.poly_keys()[0].note, Some(g));
        assert_eq!(keys.poly_keys()[1].note, Some(e));
        keys.nav.focus = Channel::Three;
        keys.press(bass);
        keys.release(c.number);
        assert_eq!(keys.gates().map(|g| g.note), [Some(a), Some(g), Some(bass)]);
        keys.release(g.number);
        assert_eq!(keys.poly_keys()[0].note, Some(e));
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
        assert_eq!(keys.poly_keys()[0].note, Some(note(69, 90)));
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
            parts[1].poly_keys = keys.poly_keys();
            let strike = |velocity| at(OrganMessage::NoteOn { note: 65, velocity });
            assert_eq!(
                parts[1]
                    .organ_events(Some(Track::TEST_TRACKS[1]))
                    .as_slice(),
                &[strike(60), strike(100)]
            );
            let both = parts;
            keys.release(first);
            parts[1].poly_keys = keys.poly_keys();
            assert!(parts[1].organ_events(Some(both[1])).is_empty());
            let remaining = parts;
            keys.release(last);
            parts[1].poly_keys = keys.poly_keys();
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
            initial[0].monophonic_events(None).as_slice(),
            &[
                at(MonoMessage::Waveform(Waveform::Sine)),
                at(MonoMessage::CutoffHz(5000)),
                at(MonoMessage::VcaEnvelope(parts[0].voice_envelope.adsr())),
            ]
        );
        parts[0].turn_knobs([-6, 0, 0, 0]);
        assert_eq!(parts[2].bass, Controls::INITIAL);
        parts[2].turn_knobs([6, 0, 0, 0]);
        assert_eq!(parts[2].bass.readout(Knob::Cutoff).1, 1000);
        parts[0].voice_envelope.turn([1, -1, 2, -2]);
        let mono = &mut parts[0].voice;
        assert_eq!(mono.cutoff_hz(), 2500);
        mono.turn_waveform(Part::Mono, 2);
        mono.gate.note = Some(note(69, 80));
        assert_eq!(
            parts[0].monophonic_events(Some(initial[0])).as_slice(),
            &[
                at(MonoMessage::Waveform(Waveform::Sawtooth)),
                at(MonoMessage::CutoffHz(2500)),
                at(MonoMessage::VcaEnvelope(parts[0].voice_envelope.adsr())),
                at(MonoMessage::NoteOn {
                    note: 69,
                    velocity: 80
                }),
            ]
        );
        let held = parts;
        parts[0].voice.turn_cutoff(i8::MIN);
        assert_eq!(
            parts[0].monophonic_events(Some(held[0])).as_slice(),
            &[at(MonoMessage::CutoffHz(10))]
        );
        parts[0].voice.turn_cutoff(i8::MAX);
        assert_eq!(parts[0].voice.cutoff_hz(), 20000);
        let open = parts;
        parts[0].voice.turn_cutoff(1);
        assert!(parts[0].monophonic_events(Some(open[0])).is_empty());
        assert_eq!(parts[2].bass.readout(Knob::Cutoff).1, 1000);
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
            parts[0].monophonic_events(Some(first[0])).as_slice(),
            &[at(MonoMessage::NoteOff(60)), play(60)]
        );

        let repeated = parts;
        keys.press(note(67, 80));
        parts[0].voice.gate = keys.gates()[0];
        assert_eq!(
            parts[0].monophonic_events(Some(repeated[0])).as_slice(),
            &[play(67)]
        );
        let legato = parts;
        keys.release(67);
        parts[0].voice.gate = keys.gates()[0];
        assert_eq!(
            parts[0].monophonic_events(Some(legato[0])).as_slice(),
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
                .voice_envelope
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
                instrument.render(parts, &mut buses)?;
                let level = (32767 * i32::from(velocity)) >> 3;
                let voice = &buses[Channel::One.source_bus()];
                assert!(
                    voice[12..]
                        .iter()
                        .all(|sample| sample.inner().abs() == level)
                );
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
                    .scope_samples(Part::Tr909, View::Scope, &buses)
                    .iter()
                    .any(|&s| s != 0)
            );
            assert_eq!(
                channel.scope_samples(Part::Tr909, View::Fx, &buses),
                channel.scope_samples(Part::Tr909, View::Scope, &buses)
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
            assert_eq!(
                channel.scope_samples(Part::Tr909, View::Scope, &buses),
                [0; 128]
            );
            Ok(())
        })
    }

    #[test]
    fn transport_plays_the_drum_bus_alongside_a_held_synth_note() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            instrument.set_fx(Fx {
                dynamics_enabled: false,
                ..Fx::INITIAL
            });
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
            assert_eq!(
                beat.map(I24::inner),
                buses[Channel::Four.source_bus()]
                    .map(|s| (s.inner() >> DRY_HEADROOM) << DRY_HEADROOM)
            );
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
            assert_eq!(
                tail.map(I24::inner),
                buses[DRY_BUS].map(|s| s.inner() << DRY_HEADROOM)
            );
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
                let post = buses[BASS_INSERT].map(|s| s.inner() >> 1);
                assert_eq!(
                    buses[DRY_BUS].map(I24::inner),
                    post.map(|x| x >> DRY_HEADROOM)
                );
                assert_eq!(buses[REVERB_SEND].map(I24::inner), post);
                assert_eq!(buses[DELAY_SEND].map(I24::inner), post.map(|x| x >> 1));
            };
            instrument.sequences = [false, false, true, false];
            instrument.transport.toggle();
            instrument.render(parts, &mut buses)?;
            let (channel, controls) = instrument
                .bass_automation(Channel::Three)
                .expect("factory bass has automation on the downbeat");
            assert_eq!(channel, Channel::Three);
            assert_eq!(
                [74, 71, 17].map(|cc| controls.get(cc)),
                [Some(28), Some(97), Some(42)]
            );
            assert_eq!(instrument.bass_automation(Channel::One), None);
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
            assert_eq!(instrument.bass_automation(Channel::Three), None);
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
            assert!(instrument.bass_automation(Channel::Three).is_some());
            instrument.sequences[2] = false;
            instrument.render(parts, &mut buses)?;
            assert!(instrument.bass_automation(Channel::Three).is_none());
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
            instrument.set_fx(FX_WITHOUT_DYNAMICS);
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
                buses[Channel::One.source_bus()]
                    .map(|s| (s.inner() >> DRY_HEADROOM) << (DRY_HEADROOM - 1))
            );
            Ok(())
        })
    }

    #[test]
    fn both_voices_reach_the_mix_and_releasing_one_preserves_the_other() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            instrument.set_fx(FX_WITHOUT_DYNAMICS);
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
                    assert_eq!(
                        mix[i],
                        ((dry >> DRY_HEADROOM) << DRY_HEADROOM) >> u8::from(half)
                    );
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
    fn algorithm_edits_each_selected_303_knob_then_home_restores_the_waveform() -> Result<(), Error>
    {
        let mut nav = Navigation::INITIAL;
        let mut track = Track::new(nav.assignments[0]);
        track.voice.gate.note = Some(note(53, 100));
        for (knob, message) in [
            (Knob::Attack, BassMessage::AttackSeconds(0.002)),
            (Knob::Overdrive, BassMessage::OverdriveDb(1.0)),
            (Knob::Tuning, BassMessage::TuningSemitones(0.1)),
            (Knob::Cutoff, BassMessage::CutoffHz(561.0)),
            (Knob::Resonance, BassMessage::Resonance(0.51)),
            (Knob::EnvMod, BassMessage::EnvOctaves(25.0 / 12.0)),
            (Knob::Decay, BassMessage::DecaySeconds(0.61)),
            (Knob::Accent, BassMessage::Accent(0.51)),
        ] {
            let before = track;
            nav.turn(1, 0);
            track.edit_panel(nav.view, 1, [0; 4], false);
            assert_eq!(nav.bass_knob(), Some(knob));
            assert!(track.events(Some(before))?.is_empty());
            track.turn_algorithm(nav.view, nav.bass_knob(), 1);
            assert_eq!(track.events(Some(before))?.as_slice(), &[at(message)]);
        }
        let before = track;
        track.edit_panel(nav.view, 0, [1, 2, 3, 4], false);
        assert_eq!(
            track.events(Some(before))?.as_slice(),
            &[
                at(BassMessage::CutoffHz(630.0)),
                at(BassMessage::Resonance(0.53)),
                at(BassMessage::EnvOctaves(28.0 / 12.0)),
                at(BassMessage::DecaySeconds(0.65)),
            ]
        );
        assert_eq!(nav.bass_knob(), Some(Knob::Accent));
        track.turn_algorithm(nav.view, nav.bass_knob(), -1);
        assert_eq!(track.bass.detent(Knob::Accent), 50);
        let before = track;
        nav.press(crate::panel::PageButton::Home);
        track.turn_algorithm(nav.view, nav.bass_knob(), 1);
        assert_eq!(
            track.events(Some(before))?.as_slice(),
            &[at(BassMessage::Waveform(wavegraph_303::Waveform::Square))]
        );
        Ok(())
    }

    #[test]
    fn shared_reverb_has_independent_sends_and_preserves_dry_audio() -> Result<(), Error> {
        with_instrument(44_100, |instrument| {
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut parts = Track::TEST_TRACKS;
            parts[0].voice.gate.note = Some(note(69, 110));
            instrument.set_mixer(Mixer::INITIAL);
            let mut fx = FX_WITHOUT_DYNAMICS;
            instrument.set_fx(fx);
            let mut audible_wet = false;
            for _ in 0..30 {
                instrument.render(parts, &mut buses)?;
                audible_wet |= buses[REVERB_RETURN]
                    .iter()
                    .any(|sample| sample.inner() != 0);
            }
            assert!(audible_wet, "shared send must reach the reverb tank");
            fx.reverb.enabled = false;
            instrument.set_fx(fx);
            instrument.render(parts, &mut buses)?;
            assert!(
                buses[REVERB_RETURN]
                    .iter()
                    .all(|sample| sample.inner() == 0)
            );
            assert_eq!(
                buses[MASTER_BUS].map(I24::inner),
                buses[DRY_BUS].map(|s| s.inner() << DRY_HEADROOM),
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
                let mut parts = Track::rack(Navigation::SYNTH_RACK.assignments);
                parts[2].bass_fx().edit([turn, 25, 4, 0], false);
                let fx = *parts[2].bass_fx();
                assert_eq!(fx.key, key);
                assert_eq!((fx.depth, fx.split_hz), (100, 200));
                assert!(fx.enabled);
                let mut reference =
                    SidechainDucker::<2>::new(44_118, 32, 176).expect("valid insert");
                let mut buses = [[I24::default(); 128]; BUS_COUNT];
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
                    assert_eq!(
                        buses[DRY_BUS].map(I24::inner),
                        expected.map(|s| s.inner() >> DRY_HEADROOM)
                    );
                }
                assert!(detector_sounded);
                parts[2].bass_fx().edit([0; 4], true);
                assert!(!parts[2].bass_fx().enabled);
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
                tracks[kit]
                    .drum_hits_mut()
                    .expect("kit")
                    .midi
                    .push(Drum::Kick, 127);
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
                        message: FxMessage::SidechainDepth(BassFx::INITIAL.depth),
                    },
                    Event {
                        frame: 0,
                        port: 0,
                        message: FxMessage::SidechainHz(BassFx::INITIAL.split_hz),
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
    fn each_303_keeps_its_own_key_depth_and_bypass() -> Result<(), Error> {
        use crate::fx::KeySource;
        with_instrument(44_118, |instrument| {
            let mut tracks = Track::rack(Navigation::INITIAL.assignments);
            tracks[0].bass_fx().edit([0, 25, 0, 0], true);
            tracks[1].bass_fx().key = KeySource::Kick808;
            tracks[1].voice.gate.note = Some(note(48, 120));
            tracks[0].voice.gate.note = Some(note(36, 120));
            tracks[2]
                .drum_hits_mut()
                .expect("kit")
                .midi
                .push(Drum::Kick, 127);
            let mut reference =
                SidechainDucker::<2>::new(44_118, 32, 176).expect("insert settings");
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let fx = *tracks[1].bass_fx();
            let controls = [
                FxMessage::SidechainDepth(fx.depth),
                FxMessage::SidechainHz(fx.split_hz),
            ]
            .map(|message| Event {
                frame: 0,
                port: 0,
                message,
            });
            for block in 0..16 {
                instrument.render(tracks, &mut buses)?;
                let mut expected = [I24::default(); 128];
                reference.process(&mut ProcessContext {
                    frames: 128,
                    inputs: &[&buses[2], &buses[5]],
                    outputs: &mut [&mut expected],
                    events: if block == 0 { &controls } else { &[] },
                    emit: &mut Err,
                });
                assert_eq!(buses[STRIP_BUS + 1], expected);
                assert_eq!(
                    Channel::Two.scope_samples(Part::Bass, View::Fx, &buses),
                    expected.map(|s| s.inner() >> 3)
                );
                assert_eq!(buses[STRIP_BUS][64..], buses[0][64..]);
            }
            assert_ne!(buses[STRIP_BUS + 1], buses[2]);
            Ok(())
        })
    }

    #[test]
    fn boosted_dry_and_both_returns_cancel_before_the_master_quantizes() {
        let full = 1 << 23;
        let mut strips = FxMixer(
            ChannelMixer::new()
                .with_boost()
                .with_dry_headroom(DRY_HEADROOM),
        );
        let mut mix = [[I24::default()]; 3];
        let hot = [I24::new_unchecked(full / 2)];
        let silent = [I24::default()];
        let boosts = [0, 1].map(|track| at(MixerMessage::Level { track, level: 192 }));
        strips.process(&mut ProcessContext {
            frames: 1,
            inputs: &[&hot, &hot, &silent, &silent],
            outputs: &mut mix.each_mut().map(|bus| &mut bus[..]),
            events: &boosts,
            emit: &mut Err,
        });
        assert_eq!(mix[0][0].inner() << DRY_HEADROOM, full * 3 / 2);
        let mut master = Master(MasterBus::new(44_118).expect("audio clock"));
        let mut output = silent;
        let returns = [
            I24::new_unchecked(-full * 3 / 4),
            I24::new_unchecked(-full / 2),
        ];
        master.process(&mut ProcessContext {
            frames: 1,
            inputs: &[&mix[0], &returns[..1], &returns[1..]],
            outputs: &mut [&mut output],
            events: &[],
            emit: &mut Err,
        });
        assert_eq!(output[0].inner(), full / 4);
    }

    #[test]
    fn the_logo_cue_feeds_the_master_limiter_and_expires_after_one_block() -> Result<(), Error> {
        use wavegraph_fx::Dynamics;
        with_instrument(44_118, |instrument| {
            let tracks = Track::TEST_TRACKS;
            let mut fx = Fx::INITIAL;
            fx.dynamics = Dynamics {
                threshold_db: 0,
                ratio_x2: 2,
                makeup_db: 12,
                ceiling_db: -12,
            };
            instrument.set_fx(fx);
            instrument.cue = Cue::test_peak();
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let y = instrument.render(tracks, &mut buses)?.map(I24::inner);
            assert!((y[0] - 2_107_123).abs() <= 2 && (y[1] + 2_107_123).abs() <= 2);
            assert_eq!(y[2..], [0; 126]);
            assert!(instrument.reduction > 0);
            assert_eq!(
                instrument.render(tracks, &mut buses)?.map(I24::inner),
                [0; 128]
            );
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
            fx.dynamics_enabled = false; // Inspect the linear dry/return sum at half gain.
            let page = Device::Delay;
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
                    ((buses[DRY_BUS][i].inner() << DRY_HEADROOM)
                        + buses[REVERB_RETURN][i].inner()
                        + echo[i])
                        .clamp(-(1 << 23), (1 << 23) - 1)
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
            parts[1].poly_keys = keys.poly_keys();
            instrument.render(parts, &mut buses)?;
            assert!(
                buses[Channel::Two.source_bus()]
                    .iter()
                    .any(|x| x.inner() != 0)
            );
            assert_eq!(buses[Channel::Three.source_bus()].map(I24::inner), [0; 128]);
            // Fifth key displaces the oldest, and releasing it brings back the oldest.
            keys.press(note(74, 95));
            parts[1].poly_keys = keys.poly_keys();
            instrument.render(parts, &mut buses)?;
            assert_eq!(parts[1].poly_keys[0].note, Some(note(74, 95)));
            keys.release(74);
            parts[1].poly_keys = keys.poly_keys();
            assert_eq!(parts[1].poly_keys[3].note, Some(note(60, 110)));
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
            parts[1].poly_keys = keys.poly_keys();
            instrument.render(parts, &mut buses)?;
            assert_eq!(parts[1].poly_keys, [PolyKey::EMPTY; POLYPHONY]);
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
        keys.press(note(53, 99));
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
        keys.press(note(60, 100));
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
        keys.press(note(60, 100));
        bass.gate = keys.gates()[Part::Bass as usize];
        bass.turn_waveform(Part::Bass, -1);
        controls.turn_knobs([i8::MAX; 4]);
        assert_eq!(controls.readout(controls.last).1, 1870);
        for knob in [Knob::Tuning, Knob::Attack, Knob::Accent, Knob::Overdrive] {
            controls.turn(knob, i8::MAX);
        }
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
            tracks[2]
                .drum_hits_mut()
                .expect("kit")
                .midi
                .push(Drum::Kick, 96);
            instrument.transport.toggle();
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            for (key, source) in [
                (KeySource::Kick, 7),
                (KeySource::Kick808, 5),
                (KeySource::Kit808, 4),
                (KeySource::Kit, 6),
            ] {
                tracks[1].bass_fx().key = key;
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
    fn fm_assignment_loads_programs_and_routes_panel_notes_and_edits() -> Result<(), Error> {
        use crate::panel::PageButton;
        use wavegraph_fm::{Patch as FmPatch, Ratio};

        with_instrument(44_100, |instrument| {
            let mut keys = Keyboard::<4>::new();
            keys.nav.press(PageButton::Glo);
            keys.nav.turn(1, 0);
            select_instrument(&mut keys.nav, Part::Fm);
            assert_eq!(keys.nav.part(), Part::Fm);
            let channel = keys.nav.focus;
            let i = channel.index();
            keys.tracks[i] = Track::factory(channel, keys.nav.assignments[i]);
            keys.nav.press(PageButton::Glo);
            let tuning = Tuning::new(44_100, 1).expect("audio clock");
            let mut reference = Fm::new(&tuning);
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut check = |tracks, messages: &[FmMessage]| -> Result<(), Error> {
                let events: Vec<_, 4> = messages
                    .iter()
                    .map(|&message| Event {
                        frame: 0,
                        port: 0,
                        message,
                    })
                    .collect();
                let mut dry = [I24::default(); 128];
                reference.process(&mut ProcessContext {
                    frames: dry.len(),
                    inputs: &[],
                    outputs: &mut [&mut dry],
                    events: &events,
                    emit: &mut Err,
                });
                instrument.render(tracks, &mut buses)?;
                assert_eq!(buses[channel.source_bus()], dry);
                assert!(dry.iter().any(|s| s.inner() != 0));
                assert!(
                    buses[Channel::Two.source_bus()]
                        .iter()
                        .all(|s| s.inner() == 0)
                );
                Ok(())
            };
            keys.press(note(64, 72));
            keys.press(note(71, 110));
            check(
                keys.tracks,
                &[
                    FmMessage::NoteOn {
                        note: 64,
                        velocity: 72,
                    },
                    FmMessage::NoteOn {
                        note: 71,
                        velocity: 110,
                    },
                ],
            )?;
            keys.tracks[i].turn_algorithm(keys.nav.view, None, 1);
            assert_eq!(keys.tracks[i].fm.patch, FmPatch::BELL);
            keys.tracks[i].turn_knobs([1, -3, 1, 2]);
            assert_eq!(
                keys.tracks[i].fm.patch,
                FmPatch {
                    ratio: Ratio::Two,
                    index: 103,
                    modulator: wavegraph_fm::Shape {
                        decay: wavegraph_fm::Time::from_us(2_000_000),
                        ..FmPatch::BELL.modulator
                    },
                    feedback: 8,
                    ..FmPatch::BELL
                }
            );
            keys.nav.press(PageButton::Env);
            assert_eq!(keys.nav.view, View::Envelope);
            keys.tracks[i].edit_panel(keys.nav.view, 0, [1, 0, 0, -1], false);
            assert_eq!(keys.tracks[i].fm.patch.carrier.attack.micros(), 3000);
            keys.tracks[i].edit_panel(keys.nav.view, 1, [0, -1, 0, 0], false);
            assert_eq!(
                keys.tracks[i].fm.patch.modulator.decay,
                FmPatch::BELL.modulator.decay
            );
            keys.tracks[i].edit_panel(keys.nav.view, 0, [0; 4], true);
            keys.tracks[i].turn_algorithm(keys.nav.view, None, -12);
            assert_eq!(keys.tracks[i].fm.patch.pitch_cents, -120);
            assert_eq!(keys.tracks[i].fm.program, 1);
            assert_eq!(
                keys.tracks[i]
                    .fm
                    .envelope(keys.tracks[i].envelope_target)
                    .adsr(),
                FmPatch::BELL.pitch.adsr()
            );
            check(keys.tracks, &[FmMessage::Patch(keys.tracks[i].fm.patch)])?;
            keys.release(64);
            keys.release(71);
            check(
                keys.tracks,
                &[FmMessage::NoteOff(64), FmMessage::NoteOff(71)],
            )
        })
    }

    #[test]
    fn supersaw_swaps_into_every_strip_with_independent_notes_and_controls() -> Result<(), Error> {
        use crate::panel::{PageButton, View};
        with_instrument(44_118, |instrument| {
            let mut nav = Navigation::INITIAL;
            let mut tracks = Track::rack(nav.assignments);
            let mut keys = Keys::<2>::new();
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            instrument.render(tracks, &mut buses)?;
            nav.press(PageButton::Glo);
            let tuning = Tuning::new(44_118, 1).expect("audio clock");
            let mut voices = core::array::from_fn::<_, CHANNELS, _>(|_| Supersaw::new(&tuning));
            for channel in Channel::ALL {
                let i = channel.index();
                nav.turn(1, 0);
                assert_eq!((nav.view, nav.focus), (View::Channel, channel));
                select_instrument(&mut nav, Part::Supersaw);
                assert_eq!(nav.part(), Part::Supersaw);
                tracks[i] = Track::factory(channel, nav.assignments[i]);
                assert!(tracks[i].voice_tap);
                let knobs = [[-12, 50, 8, -20], [127; 4], [-128; 4], [6, 20, -32, 16]][i];
                tracks[i].turn_knobs(knobs);
                assert_eq!(
                    (
                        tracks[i].voice.cutoff_hz(),
                        tracks[i].saw.resonance,
                        tracks[i].saw.detune,
                        tracks[i].saw.mix
                    ),
                    [
                        (1250, 50, 88, 76),
                        (20000, 127, 127, 127),
                        (10, 0, 0, 0),
                        (10000, 20, 48, 112)
                    ][i]
                );
                tracks[i].edit_panel(View::Envelope, 0, [1, 2, -8, 4], false);
                tracks[i].edit_panel(View::Envelope, 1, [2, -2, 8, 1], false);
                let number = [55, 62, 69, 76][i];
                let (dest, on) =
                    decode_midi([9, 0x90 | channel as u8, number, 109]).expect("MIDI note");
                keys.midi(dest, on, &mut tracks[i]);
                keys.snapshot(dest, &mut tracks[i]);
                let spec = [
                    SawMessage::CutoffHz(tracks[i].voice.cutoff_hz()),
                    SawMessage::Resonance(tracks[i].saw.resonance),
                    SawMessage::Detune(tracks[i].saw.detune),
                    SawMessage::Mix(tracks[i].saw.mix),
                    SawMessage::AmpEnvelope(tracks[i].voice_envelope.adsr()),
                    SawMessage::FilterEnvelope(tracks[i].saw.filter_envelope.adsr()),
                    SawMessage::NoteOn {
                        note: number,
                        velocity: 109,
                    },
                ]
                .map(|message| Event {
                    frame: 0,
                    port: 0,
                    message,
                });
                voices[i].process(&mut ProcessContext::<_, I24> {
                    frames: 0,
                    inputs: &[],
                    outputs: &mut [&mut []],
                    events: &spec,
                    emit: &mut Err,
                });
            }
            nav.press(PageButton::Glo);
            nav.press(PageButton::Env);
            assert_eq!(nav.view, View::Envelope);
            for block in 0..3 {
                if block == 1 {
                    // Swapping away during a held note discards only this engine.
                    nav.press(PageButton::Glo);
                    nav.turn(1, 0);
                    select_instrument(&mut nav, Part::Mono);
                    assert_eq!(nav.part(), Part::Mono);
                    keys.clear(Channel::Four);
                    tracks[3] = Track::factory(Channel::Four, nav.assignments[3]);
                }
                instrument.render(tracks, &mut buses)?;
                for (i, reference) in voices.iter_mut().enumerate() {
                    let channel = Channel::ALL[i];
                    let source = &buses[channel.source_bus()];
                    if i == 3 && block != 0 {
                        assert!(source.iter().all(|s| s.inner() == 0));
                        continue;
                    }
                    let mut expected = [I24::default(); 128];
                    reference.process(&mut ProcessContext {
                        frames: 128,
                        inputs: &[],
                        outputs: &mut [&mut expected],
                        events: &[],
                        emit: &mut Err,
                    });
                    assert_eq!(*source, expected);
                    assert!(source.iter().any(|s| s.inner() != 0));
                    assert!(
                        buses[channel.source_bus() + 1]
                            .iter()
                            .all(|s| s.inner() == 0)
                    );
                    assert_eq!(
                        channel.scope_samples(Part::Supersaw, View::Scope, &buses),
                        expected.map(|s| (s.inner() * 3) >> 3)
                    );
                }
            }
            Ok(())
        })
    }

    #[test]
    fn supersaw_select_edits_only_the_visible_envelope() -> Result<(), Error> {
        let mut nav = Navigation::INITIAL;
        nav.assignments[0] = Assignment::new(Part::Supersaw);
        let mut track = Track::new(nav.assignments[0]);
        let initial = track;
        nav.press(crate::panel::PageButton::Env);
        assert_eq!(nav.view, View::Envelope);
        track.edit_panel(nav.view, -1, [1, 2, 3, 4], false);
        assert_eq!(track.envelope_target, EnvelopeTarget::Modulation);
        assert_eq!(track.voice_envelope, initial.voice_envelope);
        assert_eq!(track.voice, initial.voice);
        assert_eq!(
            track.events(Some(initial))?.as_slice(),
            &[at(SawMessage::FilterEnvelope(
                track.saw.filter_envelope.adsr()
            ))]
        );
        let filter = track;
        track.edit_panel(nav.view, 0, [2, 3, -4, 5], true);
        assert_eq!(track.envelope_target, EnvelopeTarget::Amp);
        assert_eq!(track.saw.filter_envelope, filter.saw.filter_envelope);
        assert_eq!(track.voice_tap, initial.voice_tap);
        assert_eq!(
            track.events(Some(filter))?.as_slice(),
            &[at(SawMessage::AmpEnvelope(track.voice_envelope.adsr()))]
        );
        let amp = track;
        track.edit_panel(nav.view, 1, [0; 4], false);
        assert!(track.events(Some(amp))?.is_empty());
        assert_eq!(track.saw.filter_envelope, filter.saw.filter_envelope);
        nav.press(crate::panel::PageButton::Env);
        track.edit_panel(nav.view, -1, [0; 4], true);
        assert_eq!(
            (nav.view, track.voice.transpose, track.voice_tap),
            (View::Scope, -1, false)
        );
        Ok(())
    }

    #[test]
    fn wavetable_position_edits_follow_the_selected_program_and_leave_amp_independent()
    -> Result<(), Error> {
        let mut track = Track::new(Assignment::new(Part::Wavetable));
        let initial = track;
        // One click left from STEP wraps to ENV. Knob 3 now edits its attack.
        track.edit_panel(View::Scope, 0, [0, 0, 1, -1], false);
        let quick = track.wave_program();
        assert_eq!(quick.scan.mode, wavegraph_wavetable::Mode::Envelope);
        assert_eq!(quick.scan.travel_ms, Scan::RESONANT.travel_ms);
        assert_eq!(quick.position_envelope.adsr().attack_us, 100_000);
        track.edit_panel(View::Envelope, -1, [0, 2, -4, 1], false);
        let resonant = track.wave_program();
        assert_ne!(quick.position_envelope, resonant.position_envelope);
        assert_eq!(track.voice_envelope, initial.voice_envelope);
        assert_eq!(track.voice_tap, initial.voice_tap);
        assert_eq!(track.voice.transpose, initial.voice.transpose);
        assert_eq!(
            track.events(Some(initial))?.as_slice(),
            &[
                at(WaveMessage::Scan(resonant.scan)),
                at(WaveMessage::PositionEnvelope(
                    resonant.position_envelope.adsr()
                )),
            ]
        );
        let before = track;
        track.voice.turn_waveform(Part::Wavetable, 1);
        assert_eq!(track.wave_program(), WaveProgram::new(Scan::VOCAL));
        assert!(
            track
                .events(Some(before))?
                .contains(&at(WaveMessage::PositionEnvelope(
                    EnvelopeControls::WAVETABLE_POSITION.adsr()
                )))
        );
        track.voice.turn_waveform(Part::Wavetable, -1);
        assert_eq!(track.wave_program(), resonant);
        let before = track;
        track.edit_panel(View::Envelope, 0, [2, 0, 0, 0], true);
        assert_eq!(
            track.events(Some(before))?.as_slice(),
            &[at(WaveMessage::Envelope(track.voice_envelope.adsr()))]
        );
        assert_eq!(track.wave_program(), resonant);
        Ok(())
    }

    #[test]
    fn each_strip_can_play_a_wavetable_chord_and_recall_its_two_programs() -> Result<(), Error> {
        use crate::panel::PageButton;
        use wavegraph_wavetable::{Mode, Scan};
        with_instrument(44_100, |instrument| {
            let mut nav = Navigation::INITIAL;
            let mut tracks = Track::rack(nav.assignments);
            let mut keys = Keys::<2>::new();
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            nav.press(PageButton::Glo);
            for channel in Channel::ALL {
                let i = channel.index();
                nav.turn(1, 0);
                select_instrument(&mut nav, Part::Wavetable);
                assert_eq!((nav.focus, nav.part()), (channel, Part::Wavetable));
                let track = &mut tracks[i];
                *track = Track::factory(channel, nav.assignments[i]);
                track.turn_knobs([5, -10, -2, 1]);
                assert_eq!(
                    track.wave_program().scan,
                    Scan {
                        start: 41,
                        end: 117,
                        travel_ms: 1100,
                        mode: Mode::Smooth,
                    }
                );
                track.voice.turn_waveform(nav.part(), 1);
                assert_eq!(track.wave_program().scan, Scan::VOCAL);
                track.turn_knobs([0, 0, -127, 0]); // One traversal in 50 ms.
                for key in [57, 60, 64] {
                    keys.midi(channel, MidiAction::On(note(key, 108)), track);
                }
                keys.snapshot(channel, track);
                instrument.render(tracks, &mut buses)?;
                let events = &instrument.events[i];
                for number in [57, 60, 64] {
                    assert!(events.contains(&at(WaveMessage::NoteOn {
                        note: number,
                        velocity: 108
                    })));
                }
                assert!(buses[channel.source_bus()].iter().any(|v| v.inner() != 0));
                assert!(
                    buses[channel.source_bus() + 1]
                        .iter()
                        .all(|v| v.inner() == 0)
                );
            }
            assert!(
                instrument
                    .wave_frames
                    .windows(2)
                    .all(|pair| pair[0] > pair[1])
            );
            tracks[0].voice.turn_waveform(Part::Wavetable, -1);
            keys.midi(Channel::One, MidiAction::Off(60), &mut tracks[0]);
            keys.snapshot(Channel::One, &mut tracks[0]);
            instrument.render(tracks, &mut buses)?;
            assert!(instrument.events[0].contains(&at(WaveMessage::NoteOff(60))));
            assert!(
                instrument.events[0]
                    .contains(&at(WaveMessage::Scan(tracks[0].wave_program().scan)))
            );
            assert_eq!(instrument.wave_frames[0], 20);
            assert!(
                instrument.events[1..]
                    .iter()
                    .all(|events| events.is_empty())
            );
            tracks[0] = Track::factory(Channel::One, Assignment::new(Part::Supersaw));
            instrument.render(tracks, &mut buses)?;
            assert!(buses[0].iter().all(|v| v.inner() == 0));
            assert!(
                buses[Channel::Two.source_bus()]
                    .iter()
                    .any(|v| v.inner() != 0)
            );
            Ok(())
        })
    }

    #[test]
    fn live_supersaw_key_takes_over_its_playing_lane_and_releases_after_stop() -> Result<(), Error>
    {
        with_instrument(44_118, |instrument| {
            let mut nav = Navigation::INITIAL;
            nav.assignments[0] = Assignment::new(Part::Supersaw);
            let mut tracks = Track::rack(nav.assignments);
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            let mut keys = Keys::<1>::new();
            let mut piano = Piano::<1>::new();
            instrument.transport.toggle();
            instrument.render(tracks, &mut buses)?;
            assert!(instrument.events[0].contains(&at(SawMessage::NoteOn {
                note: 69,
                velocity: 100
            })));
            let note = note(69, 123); // Same pitch as the first sequenced note.
            piano.press(nav.focus, nav.assignments[0], note);
            keys.panel(&[], &piano.held, &tracks);
            keys.snapshot(nav.focus, &mut tracks[0]);
            instrument.render(tracks, &mut buses)?;
            assert_eq!(
                instrument.events[0].as_slice(),
                &[
                    at(SawMessage::NoteOff(69)),
                    at(SawMessage::NoteOn {
                        note: 69,
                        velocity: 123
                    })
                ]
            );
            instrument.transport.toggle();
            instrument.render(tracks, &mut buses)?;
            assert!(buses[0].iter().any(|s| s.inner() != 0));
            let before = piano.held.clone();
            piano.release(note.number);
            keys.panel(&before, &piano.held, &tracks);
            keys.snapshot(nav.focus, &mut tracks[0]);
            instrument.render(tracks, &mut buses)?;
            assert!(instrument.events[0].contains(&at(SawMessage::NoteOff(69))));
            instrument.render(tracks, &mut buses)?;
            assert!(buses[0].iter().all(|s| s.inner() == 0));
            Ok(())
        })
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
            assert_eq!(
                Channel::ALL.map(|c| instrument.bass_automation(c).is_some()),
                [true, false, true, false]
            );

            tracks[0].voice.gate.note = Some(note(55, 110));
            instrument.render(tracks, &mut buses)?;
            assert_eq!(
                Channel::ALL.map(|c| instrument.bass_automation(c).is_some()),
                [false, false, true, false]
            );
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
            tracks[0]
                .drum_hits_mut()
                .expect("kit")
                .midi
                .push(Drum::Clap, 127);
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
            assert!(instrument.bass_automation(Channel::Three).is_some());
            Ok(())
        })
    }

    #[test]
    fn panel_and_usb_controls_share_a_channel_with_independent_pattern_selection()
    -> Result<(), Error> {
        with_instrument(44_118, |instrument| {
            let mut tracks = Track::rack(Navigation::INITIAL.assignments);
            let mut piano = Piano::<2>::new();
            let mut keys = Keys::<2>::new();
            piano.event(
                Channel::One,
                tracks[0].assignment,
                MidiAction::On(note(53, 99)),
            );
            keys.panel(&[], &piano.held, &tracks);
            keys.snapshot(Channel::One, &mut tracks[0]);
            assert_eq!(tracks[0].voice.gate.note, Some(note(53, 99)));
            let before = piano.held.clone();
            piano.event(Channel::Two, tracks[1].assignment, MidiAction::Off(53));
            keys.panel(&before, &piano.held, &tracks);
            keys.snapshot(Channel::One, &mut tracks[0]);
            assert!(tracks[0].voice.gate.note.is_none());
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            instrument.select_patterns([
                Slot::KEYS[3].1,
                Slot::FIRST,
                Slot::KEYS[1].1,
                Slot::FIRST,
            ]);
            instrument.render(tracks, &mut buses)?;
            assert_eq!(instrument.banks().map(|p| p.slot.number()), [4, 1, 2, 1]);
            let original = tracks[0];
            for cc in crate::bass::CONTROLLERS {
                let (c, m) = decode_midi([11, 0xb1, cc, 93]).expect("channel two CC");
                keys.midi(c, m, &mut tracks[c.index()]);
            }
            instrument.render(tracks, &mut buses)?;
            for cc in crate::bass::CONTROLLERS {
                let parameter = crate::bass::midi_parameter(cc, 93).expect("panel-mapped CC");
                assert!(
                    instrument.events[1].contains(&at(parameter)) || cc == crate::bass::CC_WAVE
                );
            }
            assert!(tracks[0] == original);
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
