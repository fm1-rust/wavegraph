//! The FX page controls the returns. The four post-fader sends live in GLO.

use crate::instrument::{Message, at};
use heapless::Vec;
use wavegraph::Event;
use wavegraph_fx::{Delay, Message as FxMessage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FxPage {
    Reverb,
    Delay,
}

impl FxPage {
    pub(super) fn turn(&mut self, detents: i8) {
        if detents % 2 != 0 {
            *self = match self {
                Self::Reverb => Self::Delay,
                Self::Delay => Self::Reverb,
            };
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Return {
    pub(super) wet: u8,
    pub(super) enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Fx {
    pub(super) reverb: Return,
    pub(super) delay: Return,
    pub(super) decay: u8,
    pub(super) time_ms: u16,
    pub(super) feedback: u8,
}

impl Fx {
    pub(super) const INITIAL: Self = Self {
        reverb: Return {
            wet: 64,
            enabled: true,
        },
        delay: Return {
            wet: 64,
            enabled: true,
        },
        decay: 80,
        time_ms: Delay::DEFAULT_TIME_MS,
        feedback: Delay::DEFAULT_FEEDBACK,
    };

    pub(super) fn output(self, page: FxPage) -> Return {
        match page {
            FxPage::Reverb => self.reverb,
            FxPage::Delay => self.delay,
        }
    }

    pub(super) fn edit(&mut self, page: FxPage, knobs: [i8; 4], toggle: bool) {
        let (output, wet_delta) = match page {
            FxPage::Reverb => {
                self.decay = self.decay.saturating_add_signed(knobs[0]).min(127);
                (&mut self.reverb, knobs[1])
            }
            FxPage::Delay => {
                self.time_ms = self
                    .time_ms
                    .saturating_add_signed(i16::from(knobs[0]).saturating_mul(5))
                    .clamp(5, 1000);
                self.feedback = self
                    .feedback
                    .saturating_add_signed(knobs[1])
                    .min(Delay::MAX_FEEDBACK);
                (&mut self.delay, knobs[2])
            }
        };
        output.wet = output.wet.saturating_add_signed(wet_delta).min(128);
        if toggle {
            output.enabled = !output.enabled;
        }
    }

    pub(super) fn events(self, previous: Option<Self>, page: FxPage) -> Vec<Event<Message>, 4> {
        let mut events = Vec::new();
        let mut push = |changed, message: FxMessage| {
            if changed {
                events
                    .push(at(message))
                    .expect("at most four return controls");
            }
        };
        match page {
            FxPage::Reverb => push(
                previous.is_none_or(|p| p.decay != self.decay),
                FxMessage::Decay(self.decay),
            ),
            FxPage::Delay => {
                push(
                    previous.is_none_or(|p| p.time_ms != self.time_ms),
                    FxMessage::TimeMs(self.time_ms),
                );
                push(
                    previous.is_none_or(|p| p.feedback != self.feedback),
                    FxMessage::Feedback(self.feedback),
                );
            }
        }
        let out = self.output(page);
        let old = previous.map(|p| p.output(page));
        push(
            old.is_none_or(|o| o.wet != out.wet),
            FxMessage::Wet(out.wet),
        );
        push(
            old.is_none_or(|o| o.enabled != out.enabled),
            FxMessage::Enabled(out.enabled),
        );
        events
    }
}
