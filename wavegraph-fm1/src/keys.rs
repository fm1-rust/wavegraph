//! Held-note ownership across panel keys, MIDI channels and instrument swaps.

use crate::{
    instrument::{Gate, Message, Note, Part, RoutedEvents, Track, at},
    panel::{Assignment, CHANNELS, Channel},
    sequencer::DrumMessage,
};
use heapless::Vec;
use wavegraph_909::Voice as Drum;
use wavegraph_arp::Chord;
use wavegraph_dsp::bytecode::Error;
use wavegraph_organ::POLYPHONY;

/// A polyphonic key and its strike generation, retained across dropped scans.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PolyKey {
    pub(super) note: Option<Note>,
    pub(super) strike: u32,
}

impl PolyKey {
    pub(super) const EMPTY: Self = Self {
        note: None,
        strike: 0,
    };
}

const MIDI_NOTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Press {
    key: u8,
    channel: Channel,
    generation: u32,
    note: Note,
    strike: u32,
}

/// Ordered strikes between audio snapshots. A generation change starts a fresh log.
/// Capacity covers one full producer queue. If the consumer misses more than
/// a whole history, it resumes with the newest N strikes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct HitLog<const N: usize> {
    total: u32,
    hits: [(Drum, u8); N],
}

impl<const N: usize> HitLog<N> {
    const MASK: usize = {
        assert!(N.is_power_of_two());
        N - 1
    };

    pub(super) const EMPTY: Self = Self {
        total: 0,
        hits: [(Drum::Kick, 0); N],
    };

    pub(super) fn total(&self) -> u32 {
        self.total
    }

    pub(super) fn push(&mut self, drum: Drum, velocity: u8) {
        self.hits[(self.total as usize) & Self::MASK] = (drum, velocity);
        self.total = self.total.wrapping_add(1);
    }

    pub(super) fn append_since(
        &self,
        previous: u32,
        events: &mut RoutedEvents,
    ) -> Result<(), Error> {
        let count = (self.total.wrapping_sub(previous) as usize).min(N);
        let start = (self.total as usize).wrapping_sub(count);
        for offset in 0..count {
            let (voice, velocity) = self.hits[start.wrapping_add(offset) & Self::MASK];
            events
                .push(at(Message::Drum(DrumMessage::Trigger(voice, velocity))))
                .map_err(|_| Error::Capacity)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct PanelPads {
    generation: u32,
    strikes: HitLog<8>,
}

impl PanelPads {
    pub(super) fn apply(self, track: &mut Track) {
        if self.generation == track.assignment.generation
            && let Some(hits) = track.drum_hits_mut()
        {
            hits.panel = self.strikes;
        }
    }
}

pub(super) struct Piano<const KEYS: usize> {
    pub(super) octave: i8,
    pub(super) held: Vec<Press, KEYS>,
    pub(super) pads: [PanelPads; CHANNELS],
    serial: u32,
}

impl<const KEYS: usize> Piano<KEYS> {
    pub(super) const fn new() -> Self {
        Self {
            octave: 0,
            pads: [PanelPads {
                generation: 0,
                strikes: HitLog::EMPTY,
            }; CHANNELS],
            held: Vec::new(),
            serial: 0,
        }
    }

    pub(super) fn shift_octave(&mut self, delta: i8) {
        self.octave = self.octave.saturating_add(delta).clamp(-3, 3);
    }

    pub(super) fn event(&mut self, channel: Channel, assignment: Assignment, event: MidiAction) {
        match event {
            MidiAction::On(note) => {
                self.press(channel, assignment, note);
            }
            MidiAction::Off(key) => self.release(key),
            MidiAction::Control(..) => (),
        }
    }

    pub(super) fn mapped_note(&self, part: Part, key: Note) -> Option<Note> {
        let number = if part.is_kit() {
            let pad = [53, 55, 57, 59, 60, 62]
                .iter()
                .position(|&n| n == key.number)?;
            let drum = part.pads().get(pad)?;
            [36, 38, 39, 42, 46, 51][*drum as usize]
        } else {
            key.number
                .saturating_add_signed(self.octave.saturating_mul(12))
                .min(127)
        };
        Some(Note {
            number,
            velocity: key.velocity,
        })
    }

    pub(super) fn press(
        &mut self,
        channel: Channel,
        assignment: Assignment,
        key: Note,
    ) -> Option<Note> {
        self.keyboard_note(channel, assignment, key, false, false)
    }

    pub(super) fn keyboard_note(
        &mut self,
        channel: Channel,
        assignment: Assignment,
        key: Note,
        slices: bool,
        erase: bool,
    ) -> Option<Note> {
        let note = if slices {
            if key.number != 60 && !crate::assets::PAD_NOTES.contains(&key.number) {
                return None;
            }
            key
        } else {
            self.mapped_note(assignment.part, key)?
        };
        if !erase {
            self.press_mapped(channel, assignment, key.number, note);
        }
        Some(note)
    }

    fn press_mapped(&mut self, channel: Channel, assignment: Assignment, key: u8, note: Note) {
        self.release(key);
        if let Some(drum) = assignment.part.drum(note.number) {
            let pads = &mut self.pads[channel.index()];
            if pads.generation != assignment.generation {
                *pads = PanelPads {
                    generation: assignment.generation,
                    strikes: HitLog::EMPTY,
                };
            }
            pads.strikes.push(drum, note.velocity);
        }
        self.serial = self.serial.wrapping_add(1);
        self.held
            .push(Press {
                key,
                channel,
                generation: assignment.generation,
                note,
                strike: self.serial,
            })
            .expect("one press per physical key");
    }

    pub(super) fn release(&mut self, key: u8) {
        if let Some(index) = self.held.iter().position(|press| press.key == key) {
            self.held.remove(index);
        }
    }
}

pub(super) use wavegraph_dsp::midi::MidiMessage as MidiAction;

pub(super) fn decode_midi([header, status, key, value]: [u8; 4]) -> Option<(Channel, MidiAction)> {
    if header != status >> 4 {
        return None;
    }
    let channel = *Channel::ALL.get(usize::from(status & 0x0f))?;
    Some((channel, MidiAction::decode([status, key, value])?))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Key {
    Panel(u8),
    Midi(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Held {
    key: Key,
    note: Note,
    strike: u32,
}

// A pitch may be held by both a physical key and a MIDI key on the same channel.
// Keep both in strike order, including notes currently stolen by a newer note.
pub(super) struct Keys<const PANEL: usize> {
    channels: [ChannelKeys<PANEL>; CHANNELS],
}

struct ChannelKeys<const PANEL: usize> {
    midi: Vec<Held, MIDI_NOTES>,
    panel: Vec<Held, PANEL>,
    serial: u32,
    phrase: u32,
    chord: Chord,
    latch: bool,
    chord_has_midi: bool,
}

impl<const PANEL: usize> ChannelKeys<PANEL> {
    const fn new() -> Self {
        Self {
            midi: Vec::new(),
            panel: Vec::new(),
            serial: 0,
            phrase: 0,
            chord: Chord::new(),
            latch: false,
            chord_has_midi: false,
        }
    }

    fn release(&mut self, key: Key) {
        let held = match key {
            Key::Midi(_) => &mut self.midi[..],
            Key::Panel(_) => &mut self.panel[..],
        };
        if let Some(index) = held.iter().position(|held| held.key == key) {
            let old = match key {
                Key::Midi(_) => self.midi.remove(index),
                Key::Panel(_) => self.panel.remove(index),
            };
            if !self
                .midi
                .iter()
                .chain(&self.panel)
                .any(|held| held.note.number == old.note.number)
            {
                self.chord
                    .input(MidiAction::Off(old.note.number), self.latch);
            }
        }
    }

    fn press(&mut self, key: Key, note: Note) {
        self.release(key);
        if self.midi.is_empty() && self.panel.is_empty() {
            self.phrase = self.phrase.wrapping_add(1);
            self.chord_has_midi = false;
        }
        self.serial = self.serial.wrapping_add(1);
        self.chord_has_midi |= matches!(key, Key::Midi(_));
        self.chord.input(MidiAction::On(note), self.latch);
        let held = Held {
            key,
            note,
            strike: self.serial,
        };
        match key {
            Key::Midi(_) => self.midi.push(held).expect("128 MIDI pitches"),
            Key::Panel(_) => self.panel.push(held).expect("one press per panel key"),
        }
    }

    fn event(&mut self, source: Key, event: MidiAction) {
        match event {
            MidiAction::On(note) => self.press(source, note),
            MidiAction::Off(_) => self.release(source),
            MidiAction::Control(..) => (),
        }
    }

    fn clear_midi(&mut self) {
        self.midi.clear();
        self.chord_has_midi = false;
        self.chord.clear();
        for held in &self.panel {
            self.chord.input(MidiAction::On(held.note), false);
        }
    }

    fn latest(&self) -> [PolyKey; POLYPHONY] {
        let mut midi = self.midi.iter().rev().peekable();
        let mut panel = self.panel.iter().rev().peekable();
        core::array::from_fn(|_| {
            let next = match (midi.peek(), panel.peek()) {
                (Some(m), Some(p)) if p.strike.wrapping_sub(m.strike) < (1 << 31) => panel.next(),
                (Some(_), _) => midi.next(),
                (None, _) => panel.next(),
            };
            next.map_or(PolyKey::EMPTY, |held| PolyKey {
                note: Some(held.note),
                strike: held.strike,
            })
        })
    }
}

impl<const PANEL: usize> Keys<PANEL> {
    pub(super) const fn new() -> Self {
        Self {
            channels: [const { ChannelKeys::new() }; CHANNELS],
        }
    }

    pub(super) fn clear(&mut self, channel: Channel) {
        let keys = &mut self.channels[channel.index()];
        keys.midi.clear();
        keys.panel.clear();
        keys.chord.clear();
        keys.latch = false;
        keys.chord_has_midi = false;
    }

    pub(super) fn clear_midi(&mut self) {
        for keys in &mut self.channels {
            // A cable or session change must not interrupt a panel-only latch.
            if keys.chord_has_midi {
                keys.clear_midi();
            }
        }
    }

    pub(super) fn panel(&mut self, before: &[Press], now: &[Press], tracks: &[Track; CHANNELS]) {
        // A complete snapshot recovers held and released keys after a dropped sweep.
        for old in before.iter().filter(|old| !now.contains(old)) {
            self.channels[old.channel.index()]
                .event(Key::Panel(old.key), MidiAction::Off(old.note.number));
        }
        for new in now.iter().filter(|new| !before.contains(new)) {
            let track = &tracks[new.channel.index()];
            if new.generation != track.assignment.generation {
                continue;
            }
            self.channels[new.channel.index()].event(Key::Panel(new.key), MidiAction::On(new.note));
        }
    }

    pub(super) fn midi(&mut self, channel: Channel, action: MidiAction, track: &mut Track) {
        let keys = &mut self.channels[channel.index()];
        match action {
            MidiAction::On(note) if track.assignment.part.is_kit() => {
                keys.event(Key::Midi(note.number), action);
                if let Some(voice) = track.assignment.part.drum(note.number)
                    && let Some(hits) = track.drum_hits_mut()
                {
                    hits.midi.push(voice, note.velocity);
                }
            }
            MidiAction::On(note) => keys.event(Key::Midi(note.number), action),
            MidiAction::Off(note) => keys.event(Key::Midi(note), action),
            MidiAction::Control(120 | 123, _) => {
                keys.clear_midi();
                track.bass.slide = false;
            }
            MidiAction::Control(1, value) => {
                track.lfo.depth = value;
            }
            MidiAction::Control(cc, value) if track.assignment.part == Part::Bass => {
                track.bass.set_midi(cc, value, &mut track.voice);
            }
            MidiAction::Control(..) => (),
        }
    }

    pub(super) fn chord(&self, channel: Channel) -> &Chord {
        &self.channels[channel.index()].chord
    }

    pub(super) fn set_latch(&mut self, channel: Channel, latch: bool) {
        let keys = &mut self.channels[channel.index()];
        if keys.latch && !latch {
            keys.chord.release_latch();
        }
        keys.latch = latch;
    }

    pub(super) fn snapshot(&self, channel: Channel, track: &mut Track) {
        let keys = &self.channels[channel.index()];
        if track.arp.enabled {
            track.arp_revision = keys.chord.revision();
        }
        if track.assignment.part.is_kit() {
            return;
        }
        let notes = keys.latest();
        if let Some(note) = notes[0].note
            && notes[0].strike != track.voice.gate.phrase
            && notes[0].strike == keys.serial
        {
            track.select_slice(note.number);
        }
        track.voice.gate = Gate {
            note: notes[0].note,
            // Keep the last sampler strike so an uncovered one-shot is not launched twice.
            phrase: if track.assignment.part == Part::Sampler {
                keys.serial
            } else {
                keys.phrase
            },
        };
        if track.assignment.part.is_polyphonic() {
            track.poly_keys = notes;
        }
    }
}
