//! One editable FM program per assigned channel.

use crate::envelope::{EnvelopeControls, EnvelopeTarget};
use wavegraph_fm::{Patch, Ratio};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    pub(super) program: u8,
    pub(super) patch: Patch,
}

impl Controls {
    pub(super) const INITIAL: Self = Self {
        program: 0,
        patch: Patch::TINE,
    };

    pub(super) fn select(&mut self, detents: i8) {
        if detents == 0 {
            return;
        }
        let count = i16::try_from(Patch::PRESETS.len()).expect("small program bank");
        let next = i16::from(self.program)
            .saturating_add(i16::from(detents))
            .rem_euclid(count);
        let next = u8::try_from(next).expect("wrapped program index");
        if self.program != next {
            self.program = next;
            self.patch = Patch::PRESETS[usize::from(next)].1;
        }
    }

    pub(super) fn turn(&mut self, [ratio, index, decay, feedback]: [i8; 4]) {
        let patch = &mut self.patch;
        let index_of_ratio = (patch.ratio as usize).saturating_add_signed(isize::from(ratio));
        patch.ratio = Ratio::ALL[index_of_ratio.min(Ratio::ALL.len().saturating_sub(1))];
        patch.index = patch.index.saturating_add_signed(index).min(127);
        patch.modulator.decay = patch.modulator.decay.turn(decay);
        patch.feedback = patch.feedback.saturating_add_signed(feedback).min(127);
    }

    pub(super) fn envelope(&self, which: EnvelopeTarget) -> EnvelopeControls {
        let shape = match which {
            EnvelopeTarget::Amp => self.patch.carrier,
            EnvelopeTarget::Modulation => self.patch.modulator,
            EnvelopeTarget::Pitch => self.patch.pitch,
        };
        EnvelopeControls::new(shape.adsr())
    }

    pub(super) fn turn_envelope(&mut self, which: EnvelopeTarget, [a, d, s, r]: [i8; 4]) {
        let shape = match which {
            EnvelopeTarget::Amp => &mut self.patch.carrier,
            EnvelopeTarget::Modulation => &mut self.patch.modulator,
            EnvelopeTarget::Pitch => &mut self.patch.pitch,
        };
        shape.attack = shape.attack.turn(a);
        shape.decay = shape.decay.turn(d);
        shape.sustain_percent = shape.sustain_percent.saturating_add_signed(s).min(100);
        shape.release = shape.release.turn(r);
    }

    pub(super) fn turn_depth(&mut self, detents: i8) {
        self.patch.pitch_cents = self
            .patch
            .pitch_cents
            .saturating_add(i16::from(detents).saturating_mul(10))
            .clamp(-1200, 1200);
    }
}
