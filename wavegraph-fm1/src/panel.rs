//! Channel selection and page navigation for the FM-1 panel.

use crate::{fx::Device, instrument::Part};

pub(super) const CHANNELS: usize = 4;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Channel {
    #[default]
    One,
    Two,
    Three,
    Four,
}

impl Channel {
    pub(super) const ALL: [Self; CHANNELS] = [Self::One, Self::Two, Self::Three, Self::Four];

    pub(super) const fn index(self) -> usize {
        self as usize
    }

    /// Each channel produces its source signal followed by an isolated kick.
    pub(super) const fn source_bus(self) -> usize {
        self.index() << 1
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A channel index plus an i8 fits i16"
    )]
    pub(super) fn turn(&mut self, detents: i8) {
        let index = (i16::from(*self as u8) + i16::from(detents)).rem_euclid(4);
        *self = Self::ALL[usize::try_from(index).expect("four channels")];
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Assignment {
    pub(super) part: Part,
    pub(super) generation: u32,
}

impl Assignment {
    pub(super) const fn new(part: Part) -> Self {
        Self {
            part,
            generation: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum View {
    Scope,
    Envelope,
    Fx,
    Mixer,
    Channel,
    MasterFx(Option<Device>),
    Sequencer,
}

impl View {
    pub(super) fn is_mixer(self) -> bool {
        matches!(self, Self::Mixer | Self::Channel)
    }
}

pub(super) enum PageButton {
    Home,
    Env,
    Fx,
    Glo,
    GlobalFx,
    Seq,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Navigation {
    pub(super) view: View,
    pub(super) focus: Channel,
    fx_parent: View,
    show_sends: bool,
    pub(super) assignments: [Assignment; CHANNELS],
}

impl Navigation {
    pub(super) const INITIAL: Self = Self {
        view: View::Scope,
        focus: Channel::One,
        fx_parent: View::Mixer,
        show_sends: false,
        assignments: [
            Assignment::new(Part::Bass),
            Assignment::new(Part::Bass),
            Assignment::new(Part::Tr808),
            Assignment::new(Part::Tr909),
        ],
    };

    #[cfg(test)]
    pub(super) const SYNTH_RACK: Self = Self {
        assignments: [
            Assignment::new(Part::Mono),
            Assignment::new(Part::Organ),
            Assignment::new(Part::Bass),
            Assignment::new(Part::Tr909),
        ],
        ..Self::INITIAL
    };

    pub(super) fn press(&mut self, button: PageButton) {
        if self.view != View::Fx && !matches!(self.view, View::MasterFx(_)) {
            self.show_sends = false;
        }
        self.view = match button {
            PageButton::Home if matches!(self.view, View::MasterFx(Some(_))) => {
                View::MasterFx(None)
            }
            PageButton::Home if self.view == View::MasterFx(None) => self.fx_parent,
            PageButton::GlobalFx => {
                if !matches!(self.view, View::MasterFx(_)) {
                    self.fx_parent = if self.view.is_mixer() {
                        self.view
                    } else {
                        View::Fx
                    };
                }
                View::MasterFx(None)
            }
            PageButton::Fx if self.view.is_mixer() => {
                self.fx_parent = self.view;
                View::MasterFx(None)
            }
            PageButton::Glo if self.view == View::Fx => {
                self.fx_parent = View::Fx;
                View::MasterFx(None)
            }
            PageButton::Home if self.view == View::Channel => View::Mixer,
            PageButton::Home => View::Scope,
            PageButton::Glo if self.view.is_mixer() => View::Scope,
            PageButton::Glo => View::Mixer,
            PageButton::Fx if self.view != View::Fx => View::Fx,
            PageButton::Seq if self.view != View::Sequencer => View::Sequencer,
            PageButton::Env if self.view != View::Envelope && self.part().has_envelope() => {
                View::Envelope
            }
            PageButton::Env if !self.part().has_envelope() => self.view,
            _ => View::Scope,
        };
    }

    pub(super) fn part(self) -> Part {
        self.assignments[self.focus.index()].part
    }

    pub(super) fn bass_insert(self) -> bool {
        self.view == View::Fx && self.part() == Part::Bass && !self.show_sends
    }

    pub(super) fn piano_channel(self, last_melodic: &mut Channel) -> Channel {
        if !self.part().is_kit() {
            *last_melodic = self.focus;
        } else if self.view == View::Sequencer {
            if !self.assignments[last_melodic.index()].part.is_kit() {
                return *last_melodic;
            }
            if let Some(channel) = Channel::ALL
                .into_iter()
                .find(|c| !self.assignments[c.index()].part.is_kit())
            {
                *last_melodic = channel;
                return channel;
            }
        }
        self.focus
    }

    pub(super) fn turn(&mut self, select: i8, presets: i8) {
        if self.view == View::Fx && self.part() == Part::Bass && select.unsigned_abs() % 2 == 1 {
            self.show_sends = !self.show_sends;
        }
        if let View::MasterFx(row) = self.view {
            self.view = View::MasterFx(Device::select(row, select));
        }
        if select != 0 && self.view.is_mixer() {
            let detents = if self.view == View::Mixer && select > 0 {
                select.saturating_sub(1)
            } else {
                select
            };
            self.focus.turn(detents);
            self.view = View::Channel;
        }
        if presets == 0 {
            return;
        }
        if self.view == View::Channel {
            let assignment = &mut self.assignments[self.focus.index()];
            let old = assignment.part;
            assignment.part.turn(presets);
            if assignment.part != old {
                assignment.generation = assignment.generation.wrapping_add(1);
            }
        } else {
            self.focus.turn(presets);
            self.show_sends = false;
            if self.view == View::Envelope {
                self.view = View::Scope;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_button_orders_reach_the_same_rack_and_home_retraces_the_route() {
        for (first, second, parent) in [
            (PageButton::Glo, PageButton::Fx, View::Mixer),
            (PageButton::Fx, PageButton::Glo, View::Fx),
        ] {
            let mut nav = Navigation::INITIAL;
            nav.press(first);
            assert_eq!(nav.view, parent);
            nav.press(second);
            assert_eq!(nav.view, View::MasterFx(None));
            nav.turn(-1, 0);
            assert_eq!(nav.view, View::MasterFx(Some(Device::Dynamics)));
            for row in Device::ALL {
                nav.turn(1, 0);
                assert_eq!(nav.view, View::MasterFx(Some(row)));
            }
            nav.press(PageButton::Home);
            assert_eq!(nav.view, View::MasterFx(None));
            nav.press(PageButton::Home);
            assert_eq!(nav.view, parent);
            assert_eq!(nav.focus, Channel::One);
        }
        let mut nav = Navigation::INITIAL;
        nav.press(PageButton::GlobalFx);
        nav.turn(3, 0);
        assert_eq!(nav.view, View::MasterFx(Some(Device::Eq)));
    }

    #[test]
    fn instrument_fx_shows_only_the_selected_channels_insert_and_sends() {
        let mut nav = Navigation::INITIAL;
        nav.press(PageButton::Fx);
        assert!(nav.bass_insert());
        nav.turn(1, 0);
        assert!(!nav.bass_insert());
        nav.turn(-1, 0);
        assert!(nav.bass_insert());
        nav.turn(0, 2);
        assert_eq!(nav.part(), Part::Tr808);
        assert!(!nav.bass_insert());
    }

    #[test]
    fn drum_pattern_keeps_the_last_melodic_keyboard_until_all_four_channels_are_kits() {
        let mut nav = Navigation::SYNTH_RACK;
        let mut last = Channel::One;
        nav.focus = Channel::Three;
        assert_eq!(nav.piano_channel(&mut last), Channel::Three);
        nav.focus = Channel::Four;
        assert_eq!(nav.piano_channel(&mut last), Channel::Four);
        nav.press(PageButton::Seq);
        assert_eq!(nav.piano_channel(&mut last), Channel::Three);
        nav.assignments[2] = Assignment::new(Part::Tr909);
        assert_eq!(nav.piano_channel(&mut last), Channel::One);
        nav.assignments.fill(Assignment::new(Part::Tr909));
        assert_eq!(nav.piano_channel(&mut last), Channel::Four);
    }

    #[test]
    fn presets_and_home_follow_the_mixer_hierarchy() {
        let mut nav = Navigation::SYNTH_RACK;
        nav.press(PageButton::Glo);
        nav.turn(0, 1);
        assert_eq!((nav.view, nav.focus), (View::Mixer, Channel::Two));
        assert_eq!(nav.assignments, Navigation::SYNTH_RACK.assignments);
        nav.turn(1, 0);
        assert_eq!((nav.view, nav.focus), (View::Channel, Channel::Two));
        nav.turn(0, 1);
        assert_eq!(nav.part(), Part::Bass);
        assert_eq!(nav.assignments[2].part, Part::Bass);
        nav.turn(-2, 0);
        assert_eq!(nav.focus, Channel::Four);
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.focus), (View::Mixer, Channel::Four));
        nav.press(PageButton::Home);
        assert_eq!(nav.view, View::Scope);
        nav.press(PageButton::Env);
        assert_eq!(nav.view, View::Scope);
        nav.press(PageButton::Seq);
        nav.press(PageButton::Fx);
        assert_eq!(nav.view, View::Fx);
        nav.turn(0, 1);
        assert_eq!(nav.focus, Channel::One);
    }
}
