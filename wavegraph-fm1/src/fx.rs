//! FX return and 303 sidechain controls. The post-fader sends live in GLO.

use crate::instrument::{Message, at};
use heapless::Vec;
use wavegraph::Event;
use wavegraph_fx::{Delay, Message as FxMessage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FxPage {
    Reverb,
    Delay,
    BassComp,
}

impl FxPage {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "An i8 delta and page index fit i16."
    )]
    pub(super) fn turn(&mut self, detents: i8) {
        *self = match (i16::from(*self as u8) + i16::from(detents)).rem_euclid(3) {
            0 => Self::Reverb,
            1 => Self::Delay,
            _ => Self::BassComp,
        };
    }
}

// Order of detector sources on the 303 compressor page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KeySource {
    Bass,
    Kick,
    Kit,
    Organ,
    Mono,
}

impl KeySource {
    pub(super) const ALL: [KeySource; 5] =
        [Self::Bass, Self::Kick, Self::Kit, Self::Organ, Self::Mono];

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "An i8 delta and inlet index fit i16."
    )]
    fn turn(&mut self, detents: i8) {
        let index = (i16::from(*self as u8) + i16::from(detents)).rem_euclid(5);
        *self = Self::ALL[usize::try_from(index).expect("five sources")];
    }

    #[cfg(target_os = "none")]
    pub(super) fn name(self) -> &'static [u8] {
        match self {
            Self::Bass => b"303 SELF",
            Self::Kick => b"909 KICK",
            Self::Kit => b"909 KIT ",
            Self::Organ => b"M1 ORGAN",
            Self::Mono => b"MONO    ",
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
    pub(super) key: KeySource,
    pub(super) depth: u8,
    pub(super) split_hz: u16,
    pub(super) comp_enabled: bool,
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
        time_ms: 369, // Dotted eighth at the factory 122 BPM.
        feedback: Delay::DEFAULT_FEEDBACK,
        key: KeySource::Kick,
        depth: 75,
        split_hz: 180,
        comp_enabled: true,
    };

    pub(super) fn enabled(self, page: FxPage) -> bool {
        match page {
            FxPage::Reverb => self.reverb.enabled,
            FxPage::Delay => self.delay.enabled,
            FxPage::BassComp => self.comp_enabled,
        }
    }

    pub(super) fn edit(&mut self, page: FxPage, knobs: [i8; 4], toggle: bool) {
        let (output, wet_delta) = match page {
            FxPage::BassComp => {
                self.key.turn(knobs[0]);
                self.depth = self.depth.saturating_add_signed(knobs[1]).min(100);
                self.split_hz = self
                    .split_hz
                    .saturating_add_signed(i16::from(knobs[2]).saturating_mul(5))
                    .clamp(40, 1000);
                if toggle {
                    self.comp_enabled = !self.comp_enabled;
                }
                return;
            }
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
                    .expect("at most four effect controls");
            }
        };
        let (out, old) = match page {
            FxPage::Reverb => {
                push(
                    previous.is_none_or(|p| p.decay != self.decay),
                    FxMessage::Decay(self.decay),
                );
                (self.reverb, previous.map(|p| p.reverb))
            }
            FxPage::Delay => {
                push(
                    previous.is_none_or(|p| p.time_ms != self.time_ms),
                    FxMessage::TimeMs(self.time_ms),
                );
                push(
                    previous.is_none_or(|p| p.feedback != self.feedback),
                    FxMessage::Feedback(self.feedback),
                );
                (self.delay, previous.map(|p| p.delay))
            }
            FxPage::BassComp => {
                push(
                    previous.is_none_or(|p| p.key != self.key),
                    FxMessage::SidechainKey(self.key as u8),
                );
                push(
                    previous.is_none_or(|p| p.depth != self.depth),
                    FxMessage::SidechainDepth(self.depth),
                );
                push(
                    previous.is_none_or(|p| p.split_hz != self.split_hz),
                    FxMessage::SidechainHz(self.split_hz),
                );
                push(
                    previous.is_none_or(|p| p.comp_enabled != self.comp_enabled),
                    FxMessage::Enabled(self.comp_enabled),
                );
                return events;
            }
        };
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
