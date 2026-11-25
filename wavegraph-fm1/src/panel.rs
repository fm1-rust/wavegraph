//! Channel selection and page navigation for the FM-1 panel.

use wavegraph_organ::{Chord, VoiceMode};

use crate::{
    bass::Knob,
    fx::{BeatEffect, Device, INSERT_SLOTS, InsertKind},
    instrument::Part,
};

pub(super) const CHANNELS: usize = 4;

/// Elapsed 100 Hz ticks while PRESETS feedback is visible.
pub(super) fn preset_age(since: u32, now: u32) -> Option<u32> {
    let age = now.wrapping_sub(since);
    (age < 150).then_some(age)
}

/// Override the pressed pattern key for 100 ms off, then 100 ms on.
pub(super) fn pattern_press_lamp(since: &mut Option<u32>, now: u32) -> Option<bool> {
    const PHASE_TICKS: u32 = 10; // 100 Hz panel clock
    let age = now.wrapping_sub((*since)?);
    if age >= 2 * PHASE_TICKS {
        *since = None;
        return None;
    }
    Some(age >= PHASE_TICKS)
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

    #[cfg(target_os = "none")]
    pub(super) const fn label(self) -> &'static str {
        ["CH 1", "CH 2", "CH 3", "CH 4"][self.index()]
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
    SampleEdit,
    Clipboard,
    VoiceMode,
    Envelope,
    Arp,
    Lfo,
    Fx,
    Mixer,
    Channel,
    MasterFx(Option<Device>),
    BeatFx(BeatEffect),
    Sequencer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PresetRole {
    Channel,
    Instrument,
    Insert(u8),
}

impl View {
    pub(super) fn presets(self, slot: Option<u8>) -> PresetRole {
        match (self, slot) {
            (Self::Channel, _) => PresetRole::Instrument,
            (Self::Fx, Some(slot)) => PresetRole::Insert(slot),
            _ => PresetRole::Channel,
        }
    }

    fn editor(part: Part) -> Self {
        if part == Part::Sampler {
            Self::SampleEdit
        } else if part.has_voice_modes() {
            Self::VoiceMode
        } else {
            Self::Scope
        }
    }

    pub(super) fn is_mixer(self) -> bool {
        matches!(self, Self::Mixer | Self::Channel)
    }

    pub(super) fn shows_trace(self, part: Part) -> bool {
        match self {
            Self::Scope | Self::Fx => true,
            Self::SampleEdit => part == Part::Sampler,
            Self::Clipboard
            | Self::Envelope
            | Self::VoiceMode
            | Self::Lfo
            | Self::Arp
            | Self::Mixer
            | Self::Channel
            | Self::BeatFx(_)
            | Self::MasterFx(_)
            | Self::Sequencer => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Polyphony {
    pub(super) mode: VoiceMode,
    chord: Chord,
}

impl Polyphony {
    pub(super) const INITIAL: Self = Self {
        mode: VoiceMode::Poly,
        chord: Chord::MAJOR,
    };

    pub(super) fn label(mode: VoiceMode) -> &'static str {
        match mode {
            VoiceMode::Poly => "POLY",
            VoiceMode::Unison => "UNISON",
            VoiceMode::Chord(chord) => chord.name(),
        }
    }

    pub(super) fn edit(&mut self, select: i8, algorithm: i8, sel: bool, knob1: i8) {
        let index: i16 = match self.mode {
            VoiceMode::Poly => 0,
            VoiceMode::Chord(_) => 1,
            VoiceMode::Unison => 2,
        };
        let next = index
            .saturating_add(i16::from(select))
            .saturating_add(i16::from(sel))
            .rem_euclid(3);
        self.mode = match next {
            0 => VoiceMode::Poly,
            1 => {
                self.chord = self.chord.turn(algorithm).turn(knob1);
                VoiceMode::Chord(self.chord)
            }
            _ => VoiceMode::Unison,
        };
    }
}

pub(super) enum PageButton {
    Home,
    Edit,
    Arp,
    Lfo,
    Env,
    Fx,
    Glo,
    GlobalFx,
    Seq,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Navigation {
    pub(super) clip_request: crate::clipboard::Request,
    pub(super) view: View,
    pub(super) focus: Channel,
    pub(super) preset_since: Option<u32>,
    bass_slot: Option<u8>,
    fx_parent: View,
    beat_parent: View,
    pub(super) beat_steps: [wavegraph_fx::StepPattern; CHANNELS],
    pub(super) slicer_steps: [wavegraph_fx::StepPattern; CHANNELS],
    insert_slot: Option<u8>,
    pub(super) insert_kinds: [[InsertKind; INSERT_SLOTS]; CHANNELS],
    pub(super) assignments: [Assignment; CHANNELS],
}

impl Navigation {
    pub(super) const INITIAL: Self = Self {
        clip_request: crate::clipboard::Request::INITIAL,
        view: View::Scope,
        focus: Channel::One,
        preset_since: None,
        bass_slot: None,
        fx_parent: View::Mixer,
        beat_parent: View::Fx,
        beat_steps: [wavegraph_fx::StepPattern::ALL; CHANNELS],
        slicer_steps: [wavegraph_fx::StepPattern::FIRST_HALF; CHANNELS],
        insert_slot: None,
        insert_kinds: [
            InsertKind::factory(Part::Bass),
            InsertKind::factory(Part::Bass),
            InsertKind::factory(Part::Tr808),
            InsertKind::factory(Part::Tr909),
        ],
        assignments: [
            Assignment::new(Part::Bass),
            Assignment::new(Part::Bass),
            Assignment::new(Part::Tr808),
            Assignment::new(Part::Tr909),
        ],
    };

    #[cfg(test)]
    pub(super) const SYNTH_RACK: Self = Self {
        insert_kinds: [
            InsertKind::factory(Part::Mono),
            InsertKind::factory(Part::Organ),
            InsertKind::factory(Part::Bass),
            InsertKind::factory(Part::Tr909),
        ],
        assignments: [
            Assignment::new(Part::Mono),
            Assignment::new(Part::Organ),
            Assignment::new(Part::Bass),
            Assignment::new(Part::Tr909),
        ],
        ..Self::INITIAL
    };

    pub(super) fn beat_pattern_mut(&mut self) -> Option<&mut wavegraph_fx::StepPattern> {
        let i = self.focus.index();
        match self.view {
            View::BeatFx(BeatEffect::Echo) => Some(&mut self.beat_steps[i]),
            View::BeatFx(BeatEffect::Slicer) => Some(&mut self.slicer_steps[i]),
            _ => None,
        }
    }

    // Page transitions run only on button edges. Keep them out of the SRAM panel ISR.
    #[cfg_attr(target_os = "none", inline(never))]
    pub(super) fn press(&mut self, button: PageButton) {
        if matches!(button, PageButton::Home) && self.bass_slot.take().is_some() {
            return;
        }
        if matches!(button, PageButton::Home) && self.insert_slot.take().is_some() {
            return;
        }
        let next = match button {
            PageButton::Edit | PageButton::Home if matches!(self.view, View::BeatFx(_)) => {
                self.beat_parent
            }
            PageButton::Edit if matches!(self.view, View::Fx | View::MasterFx(_)) => {
                self.beat_parent = self.view;
                View::BeatFx(BeatEffect::Echo)
            }
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
            PageButton::Edit if self.view.is_mixer() => View::Clipboard,
            PageButton::Edit if self.view == View::Clipboard => View::Mixer,
            PageButton::Home if self.view == View::Channel => View::Mixer,
            PageButton::Home => View::Scope,
            PageButton::Glo if self.view.is_mixer() => View::Scope,
            PageButton::Glo => View::Mixer,
            PageButton::Edit if self.view == View::Scope => View::editor(self.part()),
            PageButton::Edit if !matches!(self.view, View::VoiceMode | View::SampleEdit) => {
                self.view
            }
            PageButton::Lfo if self.view != View::Lfo => View::Lfo,
            PageButton::Arp if self.view != View::Arp => View::Arp,
            PageButton::Fx if self.view != View::Fx => View::Fx,
            PageButton::Seq if self.view != View::Sequencer => View::Sequencer,
            PageButton::Env if self.view != View::Envelope => View::Envelope,
            _ => View::Scope,
        };
        if self.view != next {
            self.bass_slot = None;
            self.insert_slot = None;
        }
        self.view = next;
    }

    pub(super) fn part(self) -> Part {
        self.assignments[self.focus.index()].part
    }

    pub(super) fn insert_slot(self) -> Option<u8> {
        self.insert_slot.filter(|_| self.view == View::Fx)
    }

    pub(super) fn bass_knob(self) -> Option<Knob> {
        self.bass_slot.map(|i| Knob::PANEL[usize::from(i)])
    }

    #[cfg_attr(target_os = "none", inline(never))]
    pub(super) fn turn(&mut self, select: i8, presets: i8) {
        if select != 0
            && matches!(self.view, View::Scope | View::Envelope)
            && self.part() == Part::Bass
        {
            // Enter at Attack clockwise or Drive counterclockwise, counting every detent.
            const ATTACK_SLOT: i16 = 6; // Index in Knob::PANEL.
            let start = self
                .bass_slot
                .map_or(if select > 0 { ATTACK_SLOT - 1 } else { 0 }, i16::from);
            let count = i16::try_from(Knob::PANEL.len()).expect("eight knobs");
            let slot = start.saturating_add(i16::from(select)).rem_euclid(count);
            self.bass_slot = Some(u8::try_from(slot).expect("wrapped knob index"));
        }
        if let View::BeatFx(effect) = self.view {
            self.view = View::BeatFx(effect.turn(select));
        }
        if self.view == View::Fx && select != 0 {
            let start = self
                .insert_slot
                .map_or(if select > 0 { -1 } else { 0 }, i16::from);
            let slot = start.saturating_add(i16::from(select)).rem_euclid(2);
            self.insert_slot = Some(u8::try_from(slot).expect("two insert slots"));
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
        match self.view.presets(self.insert_slot()) {
            PresetRole::Insert(slot) => {
                let kind = &mut self.insert_kinds[self.focus.index()][usize::from(slot)];
                *kind = kind.turn(presets);
            }
            PresetRole::Instrument => {
                let assignment = &mut self.assignments[self.focus.index()];
                let old = assignment.part;
                assignment.part.turn(presets);
                if assignment.part != old {
                    assignment.generation = assignment.generation.wrapping_add(1);
                }
            }
            PresetRole::Channel => {
                let previous = self.focus;
                self.focus.turn(presets);
                if self.focus != previous {
                    self.bass_slot = None;
                    if matches!(self.view, View::SampleEdit | View::VoiceMode) {
                        self.view = View::editor(self.part());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_waveform_pages_request_audio_samples() {
        for view in [
            View::Scope,
            View::Fx,
            View::Envelope,
            View::Arp,
            View::Mixer,
            View::Channel,
            View::MasterFx(None),
            View::Sequencer,
        ] {
            assert_eq!(
                view.shows_trace(Part::Bass),
                matches!(view, View::Scope | View::Fx)
            );
            assert_eq!(
                view.shows_trace(Part::Tr909),
                matches!(view, View::Scope | View::Fx)
            );
        }
    }

    #[test]
    fn presets_keep_the_channel_page_across_duplicate_parts_and_drum_kits() {
        for (button, view) in [
            (PageButton::Fx, View::Fx),
            (PageButton::Env, View::Envelope),
            (PageButton::Lfo, View::Lfo),
            (PageButton::Arp, View::Arp),
            (PageButton::Seq, View::Sequencer),
        ] {
            let mut nav = Navigation::INITIAL;
            nav.press(button);
            for channel in [Channel::Two, Channel::Three, Channel::Four, Channel::One] {
                nav.turn(0, 1);
                assert_eq!((nav.focus, nav.view), (channel, view));
                assert_eq!(nav.view.presets(nav.insert_slot()), PresetRole::Channel);
            }
            nav.turn(0, -1);
            assert_eq!((nav.focus, nav.view), (Channel::Four, view));
        }
    }

    #[test]
    fn pattern_press_goes_dark_then_lights_even_across_clock_wrap() {
        let start = u32::MAX - 4;
        let mut since = Some(start);
        for (age, lamp) in [(0, false), (9, false), (10, true), (19, true)] {
            assert_eq!(
                pattern_press_lamp(&mut since, start.wrapping_add(age)),
                Some(lamp)
            );
        }
        assert_eq!(pattern_press_lamp(&mut since, start.wrapping_add(20)), None);
        assert_eq!(pattern_press_lamp(&mut since, start), None);
    }

    #[test]
    fn bass_selection_wraps_and_belongs_to_the_current_channel_and_page() {
        let mut nav = Navigation::INITIAL;
        nav.turn(0, 0);
        assert_eq!(nav.bass_knob(), None);
        nav.turn(-1, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Overdrive));
        nav.turn(i8::MIN, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Overdrive));
        nav.turn(i8::MAX, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Attack));
        nav.turn(2, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Tuning));

        nav.turn(0, 1);
        assert_eq!((nav.part(), nav.bass_knob()), (Part::Bass, None));
        nav.turn(3, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Tuning));
        nav.press(PageButton::Env);
        assert_eq!((nav.view, nav.bass_knob()), (View::Envelope, None));
        nav.turn(-3, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Accent));
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.bass_knob()), (View::Envelope, None));
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.bass_knob()), (View::Scope, None));

        nav.turn(1, 0);
        nav.press(PageButton::Fx);
        nav.turn(1, 0);
        assert_eq!((nav.view, nav.bass_knob()), (View::Fx, None));
        nav.press(PageButton::Home); // Deselect the insert first.
        nav.press(PageButton::Home);
        nav.turn(0, 1);
        nav.turn(1, 0);
        assert_eq!((nav.part(), nav.bass_knob()), (Part::Tr808, None));
    }

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
    fn beat_fx_selection_counts_detents_and_channel_focus_preserves_both_grids() {
        let mut nav = Navigation::INITIAL;
        nav.press(PageButton::Fx);
        nav.press(PageButton::Edit);
        assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
        nav.turn(2, 0);
        assert_eq!(nav.view, View::BeatFx(BeatEffect::Slicer));
        nav.beat_pattern_mut().expect("Slicer grid").toggle(0);
        nav.turn(1, 1);
        assert_eq!(
            (nav.view, nav.focus),
            (View::BeatFx(BeatEffect::Slicer), Channel::Two)
        );
        assert!(nav.slicer_steps[1].active(0));
        nav.turn(-1, -1);
        assert_eq!(
            (nav.view, nav.focus),
            (View::BeatFx(BeatEffect::Flanger), Channel::One)
        );
        assert!(nav.beat_pattern_mut().is_none());
        nav.turn(-8, 0);
        assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
        assert!(nav.beat_steps[0].active(0));
        assert!(!nav.slicer_steps[0].active(0));
    }

    #[test]
    fn instrument_fx_selects_and_assigns_independent_channel_slots() {
        let mut nav = Navigation::INITIAL;
        nav.press(PageButton::Fx);
        assert_eq!(nav.insert_slot(), None);
        nav.turn(1, 0);
        assert_eq!(nav.insert_slot(), Some(0));
        assert_eq!(
            nav.insert_kinds[0],
            [InsertKind::Saturator, InsertKind::None]
        );
        nav.turn(0, -1);
        assert_eq!(
            nav.insert_kinds[0],
            [InsertKind::Compressor, InsertKind::None]
        );
        nav.turn(1, -1);
        assert_eq!(nav.insert_slot(), Some(1));
        assert_eq!(
            nav.insert_kinds[0],
            [InsertKind::Compressor, InsertKind::Chorus]
        );
        assert_eq!(nav.focus, Channel::One);
        assert_eq!(nav.insert_kinds[1], InsertKind::factory(Part::Bass));
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.insert_slot()), (View::Fx, None));
        nav.turn(0, 2);
        assert_eq!(nav.part(), Part::Tr808);
        nav.turn(-1, 1);
        assert_eq!(nav.insert_slot(), Some(1));
        assert_eq!(
            nav.insert_kinds[2],
            [InsertKind::None, InsertKind::Compressor]
        );
        assert_eq!(nav.focus, Channel::Three);
    }

    #[test]
    fn glo_presets_cycle_in_alphabetical_display_order() {
        let parts = Part::ALL;
        assert!(parts.windows(2).all(|w| w[0].name() < w[1].name()));
        let mut nav = Navigation::INITIAL;
        nav.assignments[0] = Assignment::new(parts[0]);
        nav.press(PageButton::Glo);
        nav.turn(1, 0);
        for &expected in parts.iter().cycle().skip(1).take(parts.len()) {
            nav.turn(0, 1);
            assert_eq!(nav.part(), expected);
        }
        for &expected in parts.iter().rev() {
            nav.turn(0, -1);
            assert_eq!(nav.part(), expected);
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
        assert_eq!(nav.part(), Part::Mono);
        assert_eq!(nav.assignments[2].part, Part::Bass);
        nav.turn(-2, 0);
        assert_eq!(nav.focus, Channel::Four);
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.focus), (View::Mixer, Channel::Four));
        nav.press(PageButton::Home);
        assert_eq!(nav.view, View::Scope);
        nav.press(PageButton::Env);
        assert_eq!(nav.view, View::Envelope);
        nav.press(PageButton::Seq);
        nav.press(PageButton::Fx);
        assert_eq!(nav.view, View::Fx);
        nav.turn(0, 1);
        assert_eq!(nav.focus, Channel::One);
    }
}
