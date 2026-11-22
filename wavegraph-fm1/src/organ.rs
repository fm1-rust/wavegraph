//! M1-inspired organ panel controls, separate from the DSP's voice state.

use crate::instrument::{EventBatch, Note, at};
use wavegraph_organ::Message;

/// A physical organ key and its strike generation, retained across dropped scans.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OrganKey {
    pub(super) note: Option<Note>,
    pub(super) strike: u32,
}

impl OrganKey {
    pub(super) const EMPTY: Self = Self {
        note: None,
        strike: 0,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Knob {
    #[default]
    Tone,
    Percussion,
    Decay,
    Release,
}

impl Knob {
    const ALL: [Self; 4] = [Self::Tone, Self::Percussion, Self::Decay, Self::Release];
}

/// Four persistent knob values in the DSP's own units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    values: [u16; 4],
    last: Knob,
}

impl Controls {
    pub(super) const INITIAL: Self = Self {
        values: [80, 44, 70, 55],
        last: Knob::Tone,
    };

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "An i8 delta times five fits i16"
    )]
    pub(super) fn turn_knobs(&mut self, detents: [i8; 4]) {
        for (knob, delta) in Knob::ALL.into_iter().zip(detents) {
            if delta == 0 {
                continue;
            }
            let (minimum, maximum, scale) = match knob {
                Knob::Tone | Knob::Percussion => (0, 127, 1),
                Knob::Decay => (5, 500, 5),
                Knob::Release => (0, 1000, 5),
            };
            let i = knob as usize;
            self.values[i] = self.values[i]
                .saturating_add_signed(i16::from(delta) * scale)
                .clamp(minimum, maximum);
            self.last = knob;
        }
    }

    pub(super) fn readout(self) -> (&'static [u8], u16, &'static [u8]) {
        let value = self.values[self.last as usize];
        match self.last {
            Knob::Tone => (b"TONE     ", value, b"/127"),
            Knob::Percussion => (b"PERCUSS  ", value, b"/127"),
            Knob::Decay => (b"DECAY    ", value, b"MS  "),
            Knob::Release => (b"RELEASE  ", value, b"MS  "),
        }
    }

    pub(super) fn events(self, previous: Option<Self>) -> EventBatch {
        let mut batch = EventBatch::new();
        for knob in Knob::ALL {
            let i = knob as usize;
            if previous.is_some_and(|p| p.values[i] == self.values[i]) {
                continue;
            }
            let value = self.values[i];
            let message = match knob {
                Knob::Tone => Message::Tone(u8::try_from(value).unwrap_or(127)),
                Knob::Percussion => Message::Percussion(u8::try_from(value).unwrap_or(127)),
                Knob::Decay => Message::DecayMs(value),
                Knob::Release => Message::ReleaseMs(value),
            };
            batch.push(at(message)).expect("four organ knobs");
        }
        batch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_bounded_and_retain_last_parameter() {
        let mut controls = Controls::INITIAL;
        assert_eq!(controls.events(Some(Controls::INITIAL)).len(), 0);
        assert_eq!(controls.events(None).len(), 4);
        controls.turn_knobs([i8::MAX; 4]);
        assert_eq!(
            controls.readout(),
            (b"RELEASE  ".as_slice(), 690, b"MS  ".as_slice())
        );
        controls.turn_knobs([i8::MIN; 4]);
        assert_eq!(controls.readout().1, 50);
    }
}
