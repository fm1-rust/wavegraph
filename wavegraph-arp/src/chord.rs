//! The notes available for the next arpeggio step.

use heapless::Vec;
use wavegraph_core::midi::{MidiMessage, Note};

/// One channel's chord, including strike velocities and key order.
/// Can be collected on an input task and handed to a sample-clocked [`Clock`](crate::Clock).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chord {
    pitches: Vec<Note, 128>,
    played: Vec<u8, 128>,
    down: [u32; 4],
    revision: u32,
    pub(crate) phrase: u32,
}

impl Default for Chord {
    fn default() -> Self {
        Self::new()
    }
}

impl Chord {
    /// An empty chord.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pitches: Vec::new(),
            played: Vec::new(),
            down: [0; 4],
            revision: 0,
            phrase: 0,
        }
    }

    /// Whether there are any held or latched notes to play.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pitches.is_empty()
    }

    /// Change counter for publishing chord snapshots between tasks.
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// Forget held and latched keys, including their strike order.
    pub fn clear(&mut self) {
        self.pitches.clear();
        self.played.clear();
        self.down = [0; 4];
        self.revision = self.revision.wrapping_add(1);
    }

    /// Discard released notes when latch is switched off.
    pub fn release_latch(&mut self) {
        let down = self.down;
        let held = |key: u8| down[usize::from(key / 32)] & (1 << (key % 32)) != 0;
        self.pitches.retain(|note| held(note.number));
        self.played.retain(|&key| held(key));
        self.revision = self.revision.wrapping_add(1);
    }

    /// Receive one channel-routed MIDI message. With latch on, the first strike
    /// after all keys are up replaces the previous chord. CC 120/123 clear both.
    #[allow(
        clippy::missing_panics_doc,
        reason = "Each of the 128 MIDI pitches occupies at most one slot in either list."
    )]
    pub fn input(&mut self, message: MidiMessage, latch: bool) {
        let (key, velocity) = match message {
            MidiMessage::On(Note { number, velocity }) if number < 128 && velocity < 128 => {
                (number, velocity)
            }
            MidiMessage::Off(key) if key < 128 => (key, 0),
            MidiMessage::Control(120 | 123, _) => return self.clear(),
            _ => return,
        };
        if velocity != 0 && self.down == [0; 4] {
            self.clear();
            self.phrase = self.phrase.wrapping_add(1);
        }
        let bits = &mut self.down[usize::from(key / 32)];
        let mask = 1 << (key % 32);
        if velocity == 0 {
            if *bits & mask == 0 {
                return;
            }
            *bits &= !mask;
        } else {
            *bits |= mask;
        }
        self.revision = self.revision.wrapping_add(1);
        if velocity == 0 && latch {
            return;
        }
        let position = self.pitches.binary_search_by_key(&key, |note| note.number);
        if let Ok(i) = position {
            self.played.retain(|&n| n != key);
            self.pitches.remove(i);
        }
        if velocity != 0 {
            let i = position.unwrap_or_else(|i| i);
            self.pitches
                .insert(
                    i,
                    Note {
                        number: key,
                        velocity,
                    },
                )
                .expect("one entry per MIDI pitch");
            self.played.push(key).expect("one entry per MIDI pitch");
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The clock visits octave offsets 0 through 3."
    )]
    pub(crate) fn count(&self, octave: u8) -> usize {
        let highest = 127 - octave * 12;
        self.pitches.partition_point(|note| note.number <= highest)
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Only pitches fitting a seven-bit transposition count toward the index."
    )]
    pub(crate) fn note(&self, mut index: usize, octaves: u8, played: bool) -> Option<Note> {
        for octave in 0..octaves {
            let len = self.count(octave);
            if index < len {
                let shift = octave * 12;
                let mut note = if played {
                    let key = self
                        .played
                        .iter()
                        .filter(|&&key| key <= 127 - shift)
                        .nth(index)?;
                    let pitch = self
                        .pitches
                        .binary_search_by_key(key, |note| note.number)
                        .ok()?;
                    self.pitches[pitch]
                } else {
                    self.pitches[index]
                };
                note.number += shift;
                return Some(note);
            }
            index -= len;
        }
        None
    }
}
