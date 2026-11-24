//! Factory clips compiled from the editable Standard MIDI Files in `patterns/`.

use crate::{assets::flash_asset, bass, instrument::Part, panel::Channel};
use wavegraph::midi::MidiMessage;

pub(crate) const BAR: u16 = 128; // eight ticks per sixteenth
pub(crate) const STEPS: usize = 64;
pub(crate) const SLOTS: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Slot(u8);

impl Slot {
    pub(crate) const FIRST: Self = Self(0);
    /// First eight white keys, F3 through F4, independent of the octave setting.
    pub(crate) const KEYS: [(u8, Self); SLOTS] = [
        (0, Self(0)),
        (2, Self(1)),
        (4, Self(2)),
        (6, Self(3)),
        (7, Self(4)),
        (9, Self(5)),
        (11, Self(6)),
        (12, Self(7)),
    ];

    pub(crate) fn number(self) -> u8 {
        self.0.saturating_add(1)
    }

    pub(crate) fn clip(self, part: Part, channel: Channel) -> &'static Clip {
        let bank = if part == Part::Bass && channel == Channel::Two {
            7
        } else {
            part as usize
        };
        &BANKS[bank][usize::from(self.0)]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Timed(u16, [u8; 2]);

impl Timed {
    pub(crate) fn tick(self) -> u16 {
        self.0 >> 4
    }

    pub(crate) fn message(self) -> MidiMessage {
        let status = u8::try_from(self.0 & 15).expect("status nibble") << 4;
        MidiMessage::decode([status, self.1[0], self.1[1]]).expect("build-validated MIDI")
    }
}

pub(crate) struct Clip {
    pub(crate) bars: u8,
    pub(crate) pitches: &'static [u8],
    pub(crate) events: &'static [Timed],
}

impl Clip {
    pub(crate) fn length(&self) -> u16 {
        u16::from(self.bars) * BAR
    }
}

include!(concat!(env!("OUT_DIR"), "/patterns.rs"));

/// Last value actually written by each 303 automation lane (255 means untouched).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Automation([u8; bass::CONTROLLERS.len()]);

impl Automation {
    pub(crate) const EMPTY: Self = Self([255; bass::CONTROLLERS.len()]);

    pub(crate) fn set(&mut self, cc: u8, value: u8) {
        if let Some(i) = bass::CONTROLLERS.iter().position(|&n| n == cc) {
            self.0[i] = value;
        }
    }

    pub(crate) fn get(self, cc: u8) -> Option<u8> {
        let i = bass::CONTROLLERS.iter().position(|&n| n == cc)?;
        (self.0[i] < 128).then_some(self.0[i])
    }

    pub(crate) fn values(self) -> impl Iterator<Item = (u8, u8)> {
        bass::CONTROLLERS
            .into_iter()
            .zip(self.0)
            .filter(|&(_, v)| v < 128)
    }
}
