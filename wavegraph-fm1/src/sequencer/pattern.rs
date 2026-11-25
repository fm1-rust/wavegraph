//! Factory clips compiled from the editable Standard MIDI Files in `patterns/`.

use crate::{assets::flash_asset, bass, instrument::Part, panel::Channel};
use wavegraph_dsp::midi::MidiMessage;

pub(crate) const BAR: u16 = 128; // eight ticks per sixteenth
pub(crate) const STEPS: usize = 64;
pub(crate) const SLOTS: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Slot(u8);

/// One four-bit pattern selection per strip. The panel and DSP copy this in every snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ChannelSlots(u16);

impl ChannelSlots {
    pub(super) const FIRST: Self = Self(0);

    pub(super) fn get(self, channel: Channel) -> Slot {
        Slot(((self.0 >> ((channel as u8) << 2)) & 15) as u8)
    }

    pub(super) fn set(&mut self, channel: Channel, slot: Slot) {
        let shift = (channel as u8) << 2;
        self.0 = (self.0 & !(15 << shift)) | (u16::from(slot.0) << shift);
    }
}

const _: () = assert!(crate::panel::CHANNELS <= 4 && SLOTS <= 16);

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
        self.factory_clip(part, channel, false)
    }

    pub(crate) fn factory_clip(
        self,
        part: Part,
        channel: Channel,
        house_m7: bool,
    ) -> &'static Clip {
        let bank = match (part, channel) {
            (Part::Sampler, _) if house_m7 => 9,
            (Part::Bass, Channel::Two) => 7,
            // The electric piano starts with the organ chord clips. Recording
            // and slot selection remain keyed by the assigned part.
            (Part::Fm, _) => Part::Organ as usize,
            _ => part as usize,
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
pub(crate) struct Automation {
    cc: [u8; bass::AUTOMATION_COUNT],
    /// Local clip contribution to the slot-one Saturator (Q7).
    pub(crate) shape_offset: u8,
}

impl Automation {
    pub(crate) const EMPTY: Self = Self {
        cc: [255; bass::AUTOMATION_COUNT],
        shape_offset: 0,
    };

    pub(crate) fn set(&mut self, cc: u8, value: u8) {
        if cc == 121 {
            // Reset All Controllers also retires the receiver's external delay state.
            self.cc[bass::CONTROLLERS.len()..].fill(255);
        } else if let Some(i) = bass::automation_ccs().position(|n| n == cc) {
            self.cc[i] = value;
        }
    }

    pub(crate) fn get(self, cc: u8) -> Option<u8> {
        let i = bass::automation_ccs().position(|n| n == cc)?;
        (self.cc[i] < 128).then_some(self.cc[i])
    }

    pub(crate) fn values(self) -> impl Iterator<Item = (u8, u8)> {
        bass::automation_ccs()
            .zip(self.cc)
            .filter(|&(_, v)| v < 128)
    }
}
