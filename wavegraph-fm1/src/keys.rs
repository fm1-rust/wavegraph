//! Held-note ownership across panel keys, MIDI channels and instrument swaps.

use crate::{
    instrument::{Gate, Note, Part, Track},
    organ::OrganKey,
    panel::{Assignment, CHANNELS, Channel},
};
use heapless::Vec;
use wavegraph_909::Voice as Drum;
use wavegraph_organ::POLYPHONY;

const MIDI_NOTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Press {
    key: u8,
    channel: Channel,
    generation: u32,
    note: Note,
    strike: u32,
}

/// Strike counts survive a quick press and release between two UI snapshots.
#[derive(Clone, Copy, Default)]
pub(super) struct PanelPads {
    generation: u32,
    strikes: [u8; 6],
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
                strikes: [0; 6],
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
        if assignment.part == Part::Drums {
            if let Some(i) = [53, 55, 57, 59, 60, 62]
                .iter()
                .position(|&n| n == key.number)
            {
                let pads = &mut self.pads[channel.index()];
                if pads.generation != assignment.generation {
                    *pads = PanelPads {
                        generation: assignment.generation,
                        ..PanelPads::default()
                    };
                }
                pads.strikes[i] = pads.strikes[i].wrapping_add(1);
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

    fn latest(&self) -> [OrganKey; POLYPHONY] {
        let mut midi = self.midi.iter().rev().peekable();
        let mut panel = self.panel.iter().rev().peekable();
        core::array::from_fn(|_| {
            let next = match (midi.peek(), panel.peek()) {
                (Some(m), Some(p)) if p.strike.wrapping_sub(m.strike) < (1 << 31) => panel.next(),
                (Some(_), _) => midi.next(),
                (None, _) => panel.next(),
            };
            next.map_or(OrganKey::EMPTY, |held| OrganKey {
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
            MidiAction::On(note) if track.assignment.part == Part::Drums => {
                if let Some(voice) = Drum::from_midi_note(note.number) {
                    track.hit(voice);
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
        if track.assignment.part == Part::Organ {
            track.organ_keys = notes;
        }
    }
}
