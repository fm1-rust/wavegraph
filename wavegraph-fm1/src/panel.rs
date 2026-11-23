//! Channel selection and page navigation for the FM-1 panel.

use crate::{fx::Device, instrument::Part};

pub(super) const CHANNELS: usize = 4;

/// Elapsed 100 Hz ticks while PRESETS feedback is visible.
pub(super) fn preset_age(since: u32, now: u32) -> Option<u32> {
    let age = now.wrapping_sub(since);
    (age < 150).then_some(age)
}

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
    Arp,
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
    Arp,
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
    pub(super) preset_since: Option<u32>,
    fx_parent: View,
    show_sends: bool,
    pub(super) assignments: [Assignment; CHANNELS],
}

impl Navigation {
    pub(super) const INITIAL: Self = Self {
        view: View::Scope,
        focus: Channel::One,
        preset_since: None,
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
            PageButton::Fx if matches!(self.view, View::MasterFx(_)) => {
                if self.fx_parent == View::Channel {
                    View::Channel
                } else {
                    View::Mixer
                }
            }
            PageButton::Glo if matches!(self.view, View::MasterFx(_)) => View::Fx,
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
            PageButton::Arp if self.view == View::Scope => View::Arp,
            PageButton::Arp if self.view != View::Arp => self.view,
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
    fn preset_feedback_expires_after_150_ticks_across_clock_wrap() {
        let since = u32::MAX - 20;
        assert_eq!(preset_age(since, since), Some(0));
        assert_eq!(preset_age(since, 128), Some(149));
        assert_eq!(preset_age(since, 129), None);
    }

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
    fn leaving_the_global_rack_toggles_only_the_button_pressed() {
        for parent in [View::Mixer, View::Channel, View::Fx] {
            for selected in [false, true] {
                let mut nav = Navigation::INITIAL;
                nav.focus = Channel::Two;
                nav.view = parent;
                nav.press(if parent.is_mixer() {
                    PageButton::Fx
                } else {
                    PageButton::Glo
                });
                if selected {
                    nav.turn(1, 0);
                }
                let mut glo_off = nav;
                nav.press(PageButton::Fx);
                assert_eq!(
                    nav.view,
                    if parent == View::Channel {
                        View::Channel
                    } else {
                        View::Mixer
                    }
                );
                assert_eq!(nav.focus, Channel::Two);
                glo_off.press(PageButton::Glo);
                assert_eq!(glo_off.view, View::Fx);
                assert_eq!(glo_off.focus, Channel::Two);
            }
        }
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
        for turn in [-1, 1] {
            nav.turn(turn, 0);
            assert!(!nav.bass_insert());
        }
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
