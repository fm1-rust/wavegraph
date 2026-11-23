//! Held-note ownership across panel keys, MIDI channels and instrument swaps.

use crate::{
    instrument::{Gate, Message, Note, Part, RoutedEvents, Track, at},
    panel::{Assignment, CHANNELS, Channel},
    sequencer::DrumMessage,
};
use heapless::Vec;
use wavegraph::bytecode::Error;
use wavegraph_909::Voice as Drum;
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
        if self.generation == track.assignment.generation {
            track.panel_hits = self.strikes;
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

    pub(super) fn press(&mut self, channel: Channel, assignment: Assignment, key: Note) {
        self.release(key.number);
        if assignment.part.is_kit() {
            if let Some(i) = [53, 55, 57, 59, 60, 62]
                .iter()
                .position(|&n| n == key.number)
                && let Some(&drum) = assignment.part.pads().get(i)
            {
                let pads = &mut self.pads[channel.index()];
                if pads.generation != assignment.generation {
                    *pads = PanelPads {
                        generation: assignment.generation,
                        strikes: HitLog::EMPTY,
                    };
                }
                pads.strikes.push(drum, key.velocity);
            }
            return;
        }
        self.serial = self.serial.wrapping_add(1);
        self.held
            .push(Press {
                key: key.number,
                channel,
                generation: assignment.generation,
                note: Note {
                    number: key
                        .number
                        .saturating_add_signed(self.octave.saturating_mul(12)),
                    velocity: key.velocity,
                },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MidiAction {
    On(Note),
    Off(u8),
    Clear,
}

pub(super) fn decode_midi([header, status, key, value]: [u8; 4]) -> Option<(Channel, MidiAction)> {
    let kind = status >> 4;
    if header != kind || (key | value) >= 128 {
        return None;
    }
    let channel = *Channel::ALL.get(usize::from(status & 0x0f))?;
    let action = match (kind, key, value) {
        (9, number, velocity @ 1..) => MidiAction::On(Note { number, velocity }),
        (8 | 9, number, _) => MidiAction::Off(number),
        (11, 123, _) => MidiAction::Clear,
        _ => return None,
    };
    Some((channel, action))
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
}

impl<const PANEL: usize> ChannelKeys<PANEL> {
    const fn new() -> Self {
        Self {
            midi: Vec::new(),
            panel: Vec::new(),
            serial: 0,
            phrase: 0,
        }
    }

    fn release(&mut self, key: Key) {
        let held = match key {
            Key::Midi(_) => &mut self.midi[..],
            Key::Panel(_) => &mut self.panel[..],
        };
        if let Some(index) = held.iter().position(|held| held.key == key) {
            match key {
                Key::Midi(_) => {
                    self.midi.remove(index);
                }
                Key::Panel(_) => {
                    self.panel.remove(index);
                }
            }
        }
    }

    fn press(&mut self, key: Key, note: Note) {
        self.release(key);
        if self.midi.is_empty() && self.panel.is_empty() {
            self.phrase = self.phrase.wrapping_add(1);
        }
        self.serial = self.serial.wrapping_add(1);
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
    }

    pub(super) fn clear_midi(&mut self) {
        for keys in &mut self.channels {
            keys.midi.clear();
        }
    }

    pub(super) fn panel(&mut self, before: &[Press], now: &[Press], tracks: &[Track; CHANNELS]) {
        // A complete snapshot recovers held and released keys after a dropped sweep.
        for old in before.iter().filter(|old| !now.contains(old)) {
            self.channels[old.channel.index()].release(Key::Panel(old.key));
        }
        for new in now.iter().filter(|new| !before.contains(new)) {
            let track = &tracks[new.channel.index()];
            if new.generation != track.assignment.generation {
                continue;
            }
            self.channels[new.channel.index()].press(Key::Panel(new.key), new.note);
        }
    }

    pub(super) fn midi(&mut self, channel: Channel, action: MidiAction, track: &mut Track) {
        let keys = &mut self.channels[channel.index()];
        match action {
            MidiAction::On(note) if track.assignment.part.is_kit() => {
                let part = track.assignment.part;
                let voice = if part == Part::Tr808 {
                    wavegraph_808::Voice::from_midi_note(note.number)
                        .map(|v| part.pads()[v as usize])
                } else {
                    Drum::from_midi_note(note.number)
                };
                if let Some(voice) = voice {
                    track.midi_hits.push(voice, note.velocity);
                }
            }
            MidiAction::On(note) => keys.press(Key::Midi(note.number), note),
            MidiAction::Off(note) => keys.release(Key::Midi(note)),
            MidiAction::Clear => keys.midi.clear(),
        }
    }

    pub(super) fn snapshot(&self, channel: Channel, track: &mut Track) {
        let keys = &self.channels[channel.index()];
        let notes = keys.latest();
        track.voice.gate = Gate {
            note: notes[0].note,
            phrase: keys.phrase,
        };
        if matches!(track.assignment.part, Part::Organ | Part::Wavetable) {
            track.poly_keys = notes;
        }
    }
}
