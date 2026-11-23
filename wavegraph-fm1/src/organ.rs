//! M1-inspired organ panel controls, separate from the DSP's voice state.

use crate::{
    envelope::EnvelopeControls,
    instrument::{EventBatch, at},
};
use wavegraph_organ::{Message, Organ, Params};

pub(super) const fn instrument(numerator: u32, denominator: u32) -> Organ {
    Organ::new(numerator, denominator)
        .expect("organ sample clock")
        .with_voice_level(28)
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

/// Organ timbre and amplitude controls retained across panel views.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    tone: u8,
    percussion: u8,
    decay_ms: u16,
    pub(super) envelope: EnvelopeControls,
    last: Knob,
}

impl Controls {
    pub(super) const INITIAL: Self = Self {
        tone: Params::INITIAL.tone,
        percussion: Params::INITIAL.percussion,
        decay_ms: Params::INITIAL.decay_ms,
        envelope: EnvelopeControls::ORGAN,
        last: Knob::Tone,
    };

    /// Bring out the upper partials of the factory chords with a soft key attack.
    pub(super) const DEEP_HOUSE: Self = Self {
        tone: 90,
        percussion: 40,
        envelope: EnvelopeControls::HOUSE_CHORDS,
        ..Self::INITIAL
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
            match knob {
                Knob::Tone => self.tone = self.tone.saturating_add_signed(delta).min(127),
                Knob::Percussion => {
                    self.percussion = self.percussion.saturating_add_signed(delta).min(127)
                }
                Knob::Decay => {
                    self.decay_ms = self
                        .decay_ms
                        .saturating_add_signed(i16::from(delta) * 5)
                        .clamp(5, 500)
                }
                Knob::Release => self.envelope.turn_organ([0, 0, 0, delta]),
            }
            self.last = knob;
        }
    }

    pub(super) fn readout(self) -> (&'static [u8], u16, &'static [u8]) {
        match self.last {
            Knob::Tone => (b"TONE     ", u16::from(self.tone), b"/127"),
            Knob::Percussion => (b"PERCUSS  ", u16::from(self.percussion), b"/127"),
            Knob::Decay => (b"KEY DECAY", self.decay_ms, b"MS  "),
            Knob::Release => (
                b"RELEASE  ",
                u16::try_from(self.envelope.adsr().release_us / 1000)
                    .expect("panel release fits in milliseconds"),
                b"MS  ",
            ),
        }
    }

    pub(super) fn events(self, previous: Option<Self>) -> EventBatch {
        let mut batch = EventBatch::new();
        let old = previous.unwrap_or(Self::INITIAL);
        for (changed, msg) in [
            (self.tone != old.tone, Message::Tone(self.tone)),
            (
                self.percussion != old.percussion,
                Message::Percussion(self.percussion),
            ),
            (
                self.decay_ms != old.decay_ms,
                Message::DecayMs(self.decay_ms),
            ),
            (
                self.envelope != old.envelope,
                Message::VcaEnvelope(self.envelope.adsr()),
            ),
        ] {
            if previous.is_none() || changed {
                batch.push(at(msg)).expect("four organ controls");
            }
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

    #[test]
    fn both_views_edit_one_release_and_key_decay_is_independent_of_vca_decay() {
        let mut panel = Controls::INITIAL;
        assert_eq!(panel.envelope.adsr(), wavegraph_organ::DEFAULT_ENVELOPE);
        let initial = panel;
        panel.turn_knobs([0, 0, 1, 2]);
        assert_eq!(panel.envelope.adsr().release_us, 65_000);
        assert_eq!(
            panel.envelope.adsr().decay_us,
            initial.envelope.adsr().decay_us
        );
        assert_eq!(
            panel.events(Some(initial)).as_slice(),
            &[
                at(Message::DecayMs(75)),
                at(Message::VcaEnvelope(panel.envelope.adsr())),
            ]
        );
        let old = panel;
        panel.envelope.turn_organ([1, 1, -30, -1]);
        assert_eq!(panel.readout().1, 60);
        assert_eq!(
            panel.events(Some(old)).as_slice(),
            &[at(Message::VcaEnvelope(panel.envelope.adsr()))]
        );
        panel.envelope.turn_organ([i8::MIN; 4]);
        assert_eq!(
            panel.envelope.adsr(),
            wavegraph_adsr::Adsr {
                sustain_percent: 0,
                ..wavegraph_adsr::Adsr::GATE
            }
        );
        let trace = panel.envelope.trace();
        assert!(trace.iter().all(|&y| y == 172));
    }
}
