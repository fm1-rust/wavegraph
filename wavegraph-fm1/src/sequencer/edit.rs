//! Session edits to individual sixteenths of the flash-backed clips.
use super::pattern::Slot;
use crate::{instrument::Part, panel::Channel};
use core::sync::atomic::{AtomicU8, AtomicU16, Ordering};
use wavegraph_dsp::midi::{MidiMessage, Note};

const VOICES: usize = 6;
const EDITED: u8 = 1 << 7;
const BARS: usize = 16;
const GATE: u16 = 6; // eighths of one sixteenth

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Address {
    pub(crate) channel: Channel,
    pub(crate) part: Part,
    pub(crate) slot: Slot,
    pub(crate) bar: u8,
    pub(crate) step: u8,
}

impl Address {
    fn bar_id(self) -> u16 {
        // The one-based slot guarantees a nonzero identity.
        (self.channel as u16) << 12
            | (self.part as u16) << 8
            | u16::from(self.slot.number()) << 4
            | u16::from(self.bar)
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Step {
    mask: u8,
    notes: [u16; VOICES],
}

impl Step {
    pub(crate) const EMPTY: Self = Self {
        mask: 0,
        notes: [0; VOICES],
    };

    pub(crate) fn edited(self) -> bool {
        self.mask != 0
    }

    pub(crate) fn notes(&self) -> impl Iterator<Item = Note> + '_ {
        self.notes.iter().enumerate().filter_map(|(i, packed)| {
            let [number, velocity] = packed.to_le_bytes();
            (self.mask & (1 << i) != 0 && velocity != 0).then_some(Note { number, velocity })
        })
    }

    pub(crate) fn events(&self, phase: u16) -> impl Iterator<Item = MidiMessage> + '_ {
        let count = if self.edited() && matches!(phase, 0 | GATE) {
            VOICES
        } else {
            0
        };
        self.notes().take(count).map(move |note| {
            if phase == 0 {
                MidiMessage::On(note)
            } else {
                MidiMessage::Off(note.number)
            }
        })
    }
}

/// Pair releases with the factory strikes they belong to, even across a step boundary.
#[derive(Clone, Copy, Default)]
pub(crate) struct Suppressed([u32; 4]);

impl Suppressed {
    pub(crate) const EMPTY: Self = Self([0; 4]);

    pub(crate) fn cut(&mut self, pitch: u8) {
        self.0[usize::from(pitch / 32)] |= 1 << (pitch % 32);
    }

    pub(crate) fn factory(&mut self, step: Step, part: Part, msg: MidiMessage, phase: u16) -> bool {
        match msg {
            MidiMessage::On(n)
                if step.edited()
                    && (!part.is_kit()
                        || part
                            .drum(n.number)
                            .is_some_and(|v| step.mask & (1 << v as usize) != 0)) =>
            {
                self.cut(n.number);
                false
            }
            MidiMessage::Off(n) => {
                let word = &mut self.0[usize::from(n / 32)];
                let bit = 1 << (n % 32);
                let pass = *word & bit == 0;
                *word &= !bit;
                pass && !(phase > 0
                    && phase <= GATE
                    && step.edited()
                    && step.notes().any(|note| note.number == n))
            }
            _ => true,
        }
    }
}

struct Cell {
    // Zero inherits the factory events. EDITED alone is a melodic rest.
    mask: AtomicU8,
    notes: [AtomicU16; VOICES],
}

impl Cell {
    const fn new() -> Self {
        Self {
            mask: AtomicU8::new(0),
            notes: [const { AtomicU16::new(0) }; VOICES],
        }
    }

    fn read(&self) -> Step {
        let mask = self.mask.load(Ordering::Acquire);
        Step {
            mask,
            notes: core::array::from_fn(|i| self.notes[i].load(Ordering::Relaxed)),
        }
    }

    fn write(&self, part: Part, note: Note, erase: bool) -> bool {
        if erase && !part.is_kit() {
            self.mask.store(EDITED, Ordering::Release);
            return true;
        }
        let old = self.read();
        let voice = if part.is_kit() {
            let Some(voice) = part.drum(note.number) else {
                return false;
            };
            voice as usize
        } else if part.is_polyphonic() {
            let i = (0..4).find(|&i| {
                old.mask & (1 << i) == 0 || old.notes[i].to_le_bytes()[0] == note.number
            });
            let Some(i) = i else { return false };
            i
        } else {
            0
        };
        let velocity = if erase { 0 } else { note.velocity };
        self.notes[voice].store(
            u16::from_le_bytes([note.number, velocity]),
            Ordering::Relaxed,
        );
        self.mask
            .store(old.mask | EDITED | (1 << voice), Ordering::Release);
        true
    }
}

struct Bar {
    id: AtomicU16,
    steps: [Cell; 16],
}

/// One panel writer, with independent audio and display readers in coherent SRAM.
/// A step is latched at its onset. The audio worker never waits on the panel.
pub(crate) struct Edits {
    bars: [Bar; BARS],
}

pub(crate) static EDITS: Edits = Edits::new();

impl Edits {
    pub(crate) const fn new() -> Self {
        Self {
            bars: [const {
                Bar {
                    id: AtomicU16::new(0),
                    steps: [const { Cell::new() }; 16],
                }
            }; BARS],
        }
    }

    fn find(&self, id: u16) -> Option<&Bar> {
        self.bars
            .iter()
            .find(|bar| bar.id.load(Ordering::Acquire) == id)
    }

    pub(crate) fn step(&self, at: Address) -> Step {
        let id = at.bar_id();
        if let Some(bar) = self.find(id) {
            let step = bar.steps[usize::from(at.step)].read();
            if bar.id.load(Ordering::Acquire) == id {
                return step;
            }
        }
        Step::default()
    }

    pub(crate) fn write(&self, at: Address, note: Note, erase: bool) -> bool {
        let id = at.bar_id();
        let Some(bar) = self.find(id).or_else(|| self.find(0)) else {
            return false;
        };
        let ok = bar.steps[usize::from(at.step)].write(at.part, note, erase);
        if ok {
            bar.id.store(id, Ordering::Release);
        }
        ok
    }

    pub(crate) fn restore(&self, at: Address) {
        if let Some(bar) = self.find(at.bar_id()) {
            bar.steps[usize::from(at.step)]
                .mask
                .store(0, Ordering::Release);
            if bar
                .steps
                .iter()
                .all(|step| step.mask.load(Ordering::Relaxed) == 0)
            {
                bar.id.store(0, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(part: Part) -> Address {
        Address {
            channel: Channel::One,
            part,
            slot: Slot::FIRST,
            bar: 0,
            step: 15,
        }
    }

    #[test]
    fn chords_restrike_by_pitch_clear_and_restore() {
        let pool = Edits::new();
        let at = at(Part::Organ);
        let note = |number, velocity| Note { number, velocity };
        for pitch in [60, 64, 67, 71] {
            assert!(pool.write(at, note(pitch, 65), false));
        }
        assert!(!pool.write(at, note(72, 70), false));
        assert!(pool.write(at, note(64, 103), false));
        assert_eq!(pool.step(at).notes().count(), 4);
        assert!(pool.step(at).notes().any(|n| n == note(64, 103)));
        assert!(pool.write(at, note(60, 127), true));
        assert!(pool.step(at).edited());
        assert_eq!(pool.step(at).notes().count(), 0);
        pool.restore(at);
        assert!(!pool.step(at).edited());
    }

    #[test]
    fn full_pool_keeps_existing_edits_and_restoring_a_bar_reclaims_it() {
        let pool = Edits::new();
        let note = Note {
            number: 67,
            velocity: 90,
        };
        let base = at(Part::Mono);
        for channel in Channel::ALL {
            for bar in 0..4 {
                assert!(pool.write(
                    Address {
                        channel,
                        bar,
                        ..base
                    },
                    note,
                    false
                ));
            }
        }
        let other = Address {
            part: Part::Bass,
            ..base
        };
        assert!(!pool.write(other, note, false));
        assert_eq!(pool.step(base).notes().next(), Some(note));
        pool.restore(base);
        assert!(pool.write(other, note, false));
        for different in [
            base,
            Address {
                slot: Slot::KEYS[1].1,
                ..other
            },
            Address { step: 14, ..other },
        ] {
            assert!(!pool.step(different).edited());
        }
    }
}
