//! Channel selection and page navigation for the FM-1 panel.

use wavegraph_organ::{Chord, VoiceMode};

#[cfg(test)]
use crate::bass::Knob;
use crate::{
    bass::{Control, PresetSelection, presets::PRESETS},
    fx::{BeatEffect, Device, FilterPick, INSERT_SLOTS, InsertKind},
    instrument::Part,
};

pub(super) const CHANNELS: usize = 4;

/// Whether this picker is visible at the current 100 Hz panel tick.
pub(super) fn preset_feedback_visible(since: u32, now: u32, role: PresetRole) -> bool {
    let lifetime = match role {
        PresetRole::Channel => 75, // 750 ms after the last turn
        PresetRole::Audition(_) | PresetRole::Insert(_) => 100, // one-second choice list
    };
    now.wrapping_sub(since) < lifetime
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

    /// Each channel produces a mono source followed by its instrument-specific auxiliary signal.
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
    Audition(Part),
    Insert(u8),
}

impl View {
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

    fn is_fx_bank(self) -> bool {
        matches!(self, Self::BeatFx(_) | Self::Fx)
    }

    pub(super) fn shows_trace(self, part: Part) -> bool {
        match self {
            Self::Scope | Self::Fx | Self::BeatFx(_) => true,
            Self::SampleEdit => part == Part::Sampler,
            Self::Clipboard
            | Self::Envelope
            | Self::VoiceMode
            | Self::Lfo
            | Self::Arp
            | Self::Mixer
            | Self::Channel
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

#[derive(Debug, Eq, PartialEq)]
pub(super) enum PageButton {
    Home,
    Edit,
    HoldEdit,
    Arp,
    Lfo,
    Env,
    Fx,
    Glo,
    GlobalFx,
    Seq,
}

/// On pages with a hold action, a tap is resolved on release.
#[derive(Default)]
pub(super) struct EditButton {
    since: Option<u32>,
}

impl EditButton {
    pub(super) fn update(
        &mut self,
        wait_for_hold: bool,
        pressed: bool,
        down: bool,
        tick: u32,
    ) -> Option<PageButton> {
        if !wait_for_hold {
            self.since = None;
            return pressed.then_some(PageButton::Edit);
        }
        if pressed {
            self.since = Some(tick);
        }
        let held = tick.wrapping_sub(self.since?) >= 60; // 600 ms at 100 Hz
        if down && !held {
            return None;
        }
        self.since = None;
        Some(if held {
            PageButton::HoldEdit
        } else {
            PageButton::Edit
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PickerFeedback {
    pub(super) since: u32,
    insert: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Navigation {
    pub(super) clip_request: crate::clipboard::Request,
    pub(super) view: View,
    pub(super) focus: Channel,
    pub(super) picker_feedback: Option<PickerFeedback>,
    /// Instrument shown in the swap header. The current assignment keeps playing until it settles.
    pub(super) instrument_swap: Option<Part>,
    bass_slot: Option<u8>,
    master_parent: View,
    bank_parent: View,
    beat_effect: BeatEffect,
    pub(super) beat_steps: [wavegraph_fx::StepPattern; CHANNELS],
    pub(super) slicer_steps: [wavegraph_fx::StepPattern; CHANNELS],
    pub(super) filter_picks: [FilterPick; CHANNELS],
    insert_slot: Option<u8>,
    pub(super) insert_kinds: [[InsertKind; INSERT_SLOTS]; CHANNELS],
    pub(super) bass_presets: [PresetSelection; CHANNELS],
    pub(super) assignments: [Assignment; CHANNELS],
}

impl Navigation {
    pub(super) const INITIAL: Self = Self {
        clip_request: crate::clipboard::Request::INITIAL,
        view: View::Scope,
        focus: Channel::One,
        picker_feedback: None,
        instrument_swap: None,
        bass_slot: None,
        master_parent: View::Mixer,
        bank_parent: View::Scope,
        beat_effect: BeatEffect::Echo,
        beat_steps: [wavegraph_fx::StepPattern::ALL; CHANNELS],
        slicer_steps: [wavegraph_fx::StepPattern::FIRST_HALF; CHANNELS],
        filter_picks: [FilterPick::INITIAL; CHANNELS],
        insert_slot: None,
        insert_kinds: [
            InsertKind::factory(Part::Bass),
            InsertKind::factory(Part::Bass),
            InsertKind::factory(Part::Tr808),
            InsertKind::factory(Part::Tr909),
        ],
        bass_presets: [
            PresetSelection::FIRST,
            PresetSelection::SECOND,
            PresetSelection::FIRST,
            PresetSelection::FIRST,
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
        bass_presets: [PresetSelection::FIRST; CHANNELS],
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
        if let Some(selected) = self.instrument_swap.take() {
            // Leaving the picker takes the displayed choice immediately.
            self.assign(selected);
            self.picker_feedback = None;
            self.bass_slot = None;
            if matches!(button, PageButton::Edit | PageButton::Home) {
                return;
            }
        }
        if matches!(button, PageButton::Home) && self.bass_slot.take().is_some() {
            return;
        }
        if matches!(button, PageButton::Home)
            && self.view == View::Fx
            && self.insert_slot.take().is_some()
        {
            return;
        }
        let next = match button {
            PageButton::HoldEdit => {
                if self.can_swap() {
                    self.instrument_swap = Some(self.part());
                    self.picker_feedback = None;
                    self.bass_slot = None;
                }
                return;
            }
            PageButton::Fx | PageButton::Edit if matches!(self.view, View::BeatFx(_)) => View::Fx,
            PageButton::Fx | PageButton::Edit if self.view == View::Fx => {
                View::BeatFx(self.beat_effect)
            }
            PageButton::Edit if matches!(self.view, View::MasterFx(_)) => {
                self.bank_parent = self.view;
                View::BeatFx(self.beat_effect)
            }
            PageButton::Home if self.view.is_fx_bank() => {
                core::mem::replace(&mut self.bank_parent, View::Scope)
            }
            PageButton::Home if matches!(self.view, View::MasterFx(Some(_))) => {
                View::MasterFx(None)
            }
            PageButton::Home if self.view == View::MasterFx(None) => match self.master_parent {
                View::BeatFx(_) => View::BeatFx(self.beat_effect),
                origin => origin,
            },
            PageButton::Fx if matches!(self.view, View::MasterFx(_)) => {
                if self.master_parent == View::Channel {
                    View::Channel
                } else {
                    View::Mixer
                }
            }
            PageButton::Glo if matches!(self.view, View::MasterFx(_)) => {
                self.bank_parent = View::Scope;
                if self.master_parent == View::Fx {
                    View::Fx
                } else {
                    View::BeatFx(self.beat_effect)
                }
            }
            PageButton::GlobalFx => self.open_master(),
            PageButton::Fx if self.view.is_mixer() => self.open_master(),
            PageButton::Glo if self.view.is_fx_bank() => self.open_master(),
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
            PageButton::Fx => {
                self.bank_parent = View::Scope;
                View::BeatFx(self.beat_effect)
            }
            PageButton::Seq if self.view != View::Sequencer => View::Sequencer,
            PageButton::Env if self.view != View::Envelope => View::Envelope,
            _ => View::Scope,
        };
        if self.view != next {
            self.bass_slot = None;
        }
        self.view = next;
    }

    fn open_master(&mut self) -> View {
        if !matches!(self.view, View::MasterFx(_)) {
            self.master_parent = self.view;
            self.bank_parent = View::Scope;
        }
        View::MasterFx(None)
    }

    pub(super) fn part(self) -> Part {
        self.assignments[self.focus.index()].part
    }

    pub(super) fn can_swap(&self) -> bool {
        matches!(self.view, View::Scope | View::Mixer | View::Channel)
            && self.instrument_swap.is_none()
    }

    pub(super) fn preset_role(&self) -> PresetRole {
        if let Some(selected) = self.instrument_swap {
            PresetRole::Audition(selected)
        } else if let Some(slot) = self.picker_feedback.and_then(|feedback| feedback.insert) {
            PresetRole::Insert(slot)
        } else {
            PresetRole::Channel
        }
    }

    fn assign(&mut self, part: Part) {
        let assignment = &mut self.assignments[self.focus.index()];
        if assignment.part != part {
            assignment.part = part;
            assignment.generation = assignment.generation.wrapping_add(1);
            self.bass_slot = None;
        }
    }

    /// Time picker feedback and instrument audition from the retained panel scan.
    pub(super) fn poll_pickers(&mut self, tick: u32, presets_turned: bool, algorithm_turned: bool) {
        const SETTLE_TICKS: u32 = 20; // 200 ms at 100 Hz
        let insert = self.insert_slot().filter(|_| algorithm_turned);
        if presets_turned || insert.is_some() {
            self.picker_feedback = Some(PickerFeedback {
                since: tick,
                insert,
            });
        }
        let Some(PickerFeedback { since, .. }) = self.picker_feedback else {
            return;
        };
        if let Some(selected) = self.instrument_swap
            && selected != self.part()
            && tick.wrapping_sub(since) >= SETTLE_TICKS
        {
            self.assign(selected);
        }
        // Auditioning the sound and dismissing the picker have independent deadlines.
        if !preset_feedback_visible(since, tick, self.preset_role()) {
            self.picker_feedback = None;
        }
    }

    pub(super) fn insert_slot(self) -> Option<u8> {
        self.insert_slot.filter(|_| self.view == View::Fx)
    }

    #[cfg(test)]
    pub(super) fn bass_knob(self) -> Option<Knob> {
        match self.bass_control()? {
            Control::Knob(knob) => Some(knob),
            Control::Waveform | Control::Muffler => None,
        }
    }

    pub(super) fn bass_control(self) -> Option<Control> {
        self.bass_slot.map(|i| Control::PANEL[usize::from(i)])
    }

    /// Preset requests and their insert types travel in the same retained snapshot.
    pub(super) fn turn_algorithm(&mut self, detents: i8) {
        if detents == 0 {
            return;
        }
        if let Some(slot) = self.insert_slot() {
            let kind = &mut self.insert_kinds[self.focus.index()][usize::from(slot)];
            *kind = kind.turn(detents);
            return;
        }
        if self.part() != Part::Bass
            || !matches!(self.view, View::Scope | View::Envelope)
            || self.bass_control().is_some()
        {
            return;
        }
        let i = self.focus.index();
        self.bass_presets[i].turn(detents);
        let sound = PRESETS[usize::from(self.bass_presets[i].index)];
        self.insert_kinds[i] = sound.inserts.map(|insert| insert.kind());
        self.beat_steps[i] = sound.echo_steps;
        self.slicer_steps[i] = sound.slicer_steps;
    }

    #[cfg_attr(target_os = "none", inline(never))]
    pub(super) fn turn(&mut self, select: i8, presets: i8) {
        if select != 0
            && matches!(self.view, View::Scope | View::Envelope)
            && self.part() == Part::Bass
        {
            // Enter at Attack clockwise or the waveform switch counterclockwise.
            const ATTACK_SLOT: i16 = 6; // Index in Control::PANEL.
            let start = self
                .bass_slot
                .map_or(if select > 0 { ATTACK_SLOT - 1 } else { 0 }, i16::from);
            let count = i16::try_from(Control::PANEL.len()).expect("small 303 panel");
            let slot = start.saturating_add(i16::from(select)).rem_euclid(count);
            self.bass_slot = Some(u8::try_from(slot).expect("wrapped knob index"));
        }
        if let View::BeatFx(effect) = self.view {
            self.beat_effect = effect.turn(select);
            self.view = View::BeatFx(self.beat_effect);
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
        if select != 0 && self.view.is_mixer() && self.instrument_swap.is_none() {
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
        if let Some(selected) = &mut self.instrument_swap {
            selected.turn(presets);
        } else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_hold_fires_once_and_a_short_scope_press_still_taps() {
        let mut edit = EditButton::default();
        let start = u32::MAX - 20;
        assert_eq!(edit.update(true, true, true, start), None);
        assert_eq!(edit.update(true, false, true, 38), None); // 59 ticks
        assert_eq!(edit.update(true, false, false, 38), Some(PageButton::Edit));
        assert_eq!(edit.update(true, true, true, start), None);
        assert_eq!(
            edit.update(true, false, true, 39),
            Some(PageButton::HoldEdit)
        );
        assert_eq!(edit.update(false, false, true, 40), None);
        assert_eq!(edit.update(false, false, false, 41), None);
        assert_eq!(edit.update(false, true, true, 42), Some(PageButton::Edit));
        assert_eq!(edit.update(true, false, false, 45), None);

        // Navigating away during the hold cancels the pending tap and long press.
        assert_eq!(edit.update(true, true, true, 50), None);
        assert_eq!(edit.update(false, false, true, 51), None);
        assert_eq!(edit.update(true, false, true, 115), None);
        assert_eq!(edit.update(true, false, false, 116), None);
    }

    #[test]
    fn audition_stays_on_its_original_channel_and_view() {
        for view in [View::Scope, View::Mixer, View::Channel] {
            for ch in Channel::ALL {
                let mut nav = Navigation::INITIAL;
                nav.view = view;
                nav.focus = ch;
                let mut expected = nav.assignments;
                nav.press(PageButton::HoldEdit);
                for delta in [1, -2, 10, i8::MIN, i8::MAX] {
                    let previous = expected;
                    let slot = &mut expected[ch.index()];
                    let mut part = slot.part;
                    part.turn(delta);
                    if part != slot.part {
                        slot.part = part;
                        slot.generation += 1;
                    }
                    // SELECT must not move the destination strip while auditioning.
                    nav.turn(1, delta);
                    nav.poll_pickers(0, true, false);
                    assert_eq!(nav.assignments, previous);
                    nav.poll_pickers(20, false, false);
                    assert_eq!(nav.assignments, expected);
                    assert_eq!(
                        (nav.focus, nav.view, nav.preset_role()),
                        (ch, view, PresetRole::Audition(part))
                    );
                }
                nav.press(PageButton::Edit);
                assert_eq!(nav.instrument_swap, None);
                assert_eq!((nav.view, nav.preset_role()), (view, PresetRole::Channel));
                nav.turn(0, 1);
                assert_ne!(nav.focus, ch);
                assert_eq!(nav.assignments, expected);
                if view.is_mixer() {
                    nav.press(PageButton::Edit);
                    assert_eq!(nav.view, View::Clipboard);
                }
            }
        }
    }

    #[test]
    fn audition_settles_only_the_last_detent_and_keeps_browsing() {
        let mut nav = Navigation::INITIAL;
        let original = nav.assignments;
        nav.press(PageButton::HoldEdit);
        // Overshoot the 808, then back up across the wrapping 100 Hz clock.
        for (delta, now, name) in [
            (1, u32::MAX - 12, Part::Tr808),
            (1, u32::MAX - 4, Part::Tr909),
            (-1, 5, Part::Tr808),
        ] {
            nav.turn(0, delta);
            nav.poll_pickers(now, true, false);
            assert_eq!(nav.instrument_swap, Some(name));
            assert_eq!(nav.assignments, original);
        }
        nav.poll_pickers(24, false, false);
        assert_eq!(nav.assignments, original);
        nav.poll_pickers(25, false, false);
        // The picker stays up after the sound changes.
        assert_eq!(nav.picker_feedback.map(|feedback| feedback.since), Some(5));
        let loaded = nav.assignments;
        assert_eq!(
            loaded[0],
            Assignment {
                part: Part::Tr808,
                generation: 1
            }
        );
        assert_eq!(loaded[1..], original[1..]);
        assert_eq!(nav.preset_role(), PresetRole::Audition(Part::Tr808));

        // Reversing back to the live sound leaves its playing voice and settings intact.
        nav.turn(0, 1);
        nav.poll_pickers(50, true, false);
        nav.turn(0, -1);
        nav.poll_pickers(55, true, false);
        nav.poll_pickers(300, false, false);
        assert_eq!(nav.picker_feedback, None);
        assert_eq!(nav.assignments, loaded);
        // EDIT takes the displayed choice even a single tick after the detent.
        nav.turn(0, -1);
        nav.poll_pickers(310, true, false);
        nav.press(PageButton::Edit);
        assert_eq!(
            nav.assignments[0],
            Assignment {
                part: Part::Bass,
                generation: 2
            }
        );
        assert_eq!(
            (nav.view, nav.preset_role()),
            (View::Scope, PresetRole::Channel)
        );
        nav.poll_pickers(311, false, false);
        nav.turn(0, 1);
        nav.poll_pickers(500, false, false);
        assert_eq!(nav.focus, Channel::Two);
        assert_eq!(nav.assignments[1..], original[1..]);
    }

    #[test]
    fn every_instrument_keeps_its_tap_editor_and_can_open_beat_fx() {
        for part in Part::ALL {
            let mut nav = Navigation::INITIAL;
            nav.assignments[0] = Assignment::new(part);
            nav.press(PageButton::Edit);
            assert_eq!(nav.view, View::editor(part));
            assert_eq!(nav.instrument_swap, None);
            nav.press(PageButton::Home);
            nav.press(PageButton::HoldEdit);
            nav.press(PageButton::Fx);
            assert_eq!(nav.instrument_swap, None);
            assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
            nav.press(PageButton::Fx);
            nav.turn(1, 0);
            assert!(nav.insert_slot().is_some());
            nav.press(PageButton::Edit);
            assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
            nav.press(PageButton::Home);
            assert_eq!(nav.view, View::Scope);
            nav.press(PageButton::Fx);
            nav.press(PageButton::Glo);
            nav.turn(1, 0);
            let rack = nav.view;
            nav.press(PageButton::Edit);
            assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
            nav.press(PageButton::Home);
            assert_eq!(nav.view, rack);
        }
    }

    #[test]
    fn only_waveform_pages_request_audio_samples() {
        for view in [
            View::Scope,
            View::Fx,
            View::BeatFx(BeatEffect::Echo),
            View::BeatFx(BeatEffect::Flanger),
            View::BeatFx(BeatEffect::Slicer),
            View::BeatFx(BeatEffect::Filter),
            View::Envelope,
            View::Arp,
            View::Mixer,
            View::Channel,
            View::MasterFx(None),
            View::Sequencer,
        ] {
            assert_eq!(
                view.shows_trace(Part::Bass),
                matches!(view, View::Scope | View::Fx | View::BeatFx(_))
            );
            assert_eq!(
                view.shows_trace(Part::Tr909),
                matches!(view, View::Scope | View::Fx | View::BeatFx(_))
            );
        }
    }

    #[test]
    fn presets_keep_the_channel_page_across_duplicate_parts_and_drum_kits() {
        for (button, view) in [
            (PageButton::Fx, View::BeatFx(BeatEffect::Echo)),
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
                assert_eq!(nav.preset_role(), PresetRole::Channel);
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
        assert_eq!(nav.bass_control(), Some(Control::Waveform));
        nav.turn(i8::MIN, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Cutoff));
        nav.turn(i8::MAX, 0);
        assert_eq!(nav.bass_control(), Some(Control::Muffler));
        nav.turn(2, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Tuning));

        nav.turn(0, 1);
        assert_eq!((nav.part(), nav.bass_knob()), (Part::Bass, None));
        nav.turn(5, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Tuning));
        nav.press(PageButton::Env);
        assert_eq!((nav.view, nav.bass_knob()), (View::Envelope, None));
        nav.turn(-5, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Accent));
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.bass_knob()), (View::Envelope, None));
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.bass_knob()), (View::Scope, None));

        nav.turn(1, 0);
        nav.press(PageButton::Fx);
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
    fn channel_feedback_follows_the_last_turn_across_clock_wrap() {
        let start = u32::MAX - 15;
        let mut nav = Navigation::INITIAL;
        nav.turn(0, 2);
        nav.poll_pickers(start, true, false);
        assert_eq!(nav.focus, Channel::Three);
        let last = start.wrapping_add(21);
        nav.turn(0, -1);
        nav.poll_pickers(last, true, false);
        assert_eq!(nav.focus, Channel::Two);
        nav.poll_pickers(last.wrapping_add(74), false, false);
        assert_eq!(nav.picker_feedback.map(|f| f.since), Some(last));
        nav.poll_pickers(last.wrapping_add(75), false, false);
        assert_eq!(nav.picker_feedback, None);
        assert_eq!(nav.focus, Channel::Two);
    }

    #[test]
    fn instrument_and_insert_choosers_keep_their_longer_timeout() {
        let since = u32::MAX - 20;
        for role in [PresetRole::Audition(Part::Tr808), PresetRole::Insert(0)] {
            assert!(preset_feedback_visible(since, since, role));
            assert!(preset_feedback_visible(since, 78, role));
            assert!(!preset_feedback_visible(since, 79, role));
        }
    }

    #[test]
    fn both_button_orders_reach_the_same_rack_and_home_retraces_the_route() {
        for (first, second, parent) in [
            (PageButton::Glo, PageButton::Fx, View::Mixer),
            (
                PageButton::Fx,
                PageButton::Glo,
                View::BeatFx(BeatEffect::Echo),
            ),
        ] {
            let mut nav = Navigation::INITIAL;
            nav.press(first);
            assert_eq!(nav.view, parent);
            nav.press(second);
            assert_eq!(nav.view, View::MasterFx(None));
            nav.press(PageButton::GlobalFx); // Second edge of a held GLO + FX chord.
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
        for parent in [
            View::Mixer,
            View::Channel,
            View::Fx,
            View::BeatFx(BeatEffect::Echo),
        ] {
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
                assert_eq!(
                    glo_off.view,
                    if parent == View::Fx {
                        View::Fx
                    } else {
                        View::BeatFx(BeatEffect::Echo)
                    }
                );
                assert_eq!(glo_off.focus, Channel::Two);
            }
        }
    }

    #[test]
    fn fx_cycles_banks_without_moving_channels_or_losing_either_cursor() {
        for initial in [
            View::Scope,
            View::Envelope,
            View::Lfo,
            View::Arp,
            View::VoiceMode,
            View::SampleEdit,
            View::Sequencer,
        ] {
            let mut nav = Navigation::INITIAL;
            nav.view = initial;
            let inserts = nav.insert_kinds;
            nav.press(PageButton::Fx);
            assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
            nav.turn(2, 0);
            nav.press(PageButton::Fx);
            assert_eq!((nav.view, nav.insert_slot()), (View::Fx, None));
            nav.turn(-1, 0);
            assert_eq!(nav.insert_slot(), Some(1));
            nav.press(PageButton::Fx);
            assert_eq!(nav.view, View::BeatFx(BeatEffect::Slicer));
            for channel in [Channel::Two, Channel::Three, Channel::Four, Channel::One] {
                nav.turn(0, 1);
                assert_eq!(
                    (nav.view, nav.focus),
                    (View::BeatFx(BeatEffect::Slicer), channel)
                );
                nav.press(PageButton::Fx);
                assert_eq!((nav.focus, nav.insert_slot()), (channel, Some(1)));
                nav.press(PageButton::Edit);
                assert_eq!(nav.view, View::BeatFx(BeatEffect::Slicer));
            }
            assert_eq!(nav.insert_kinds, inserts);
            nav.press(PageButton::Home); // Hidden insert cursor does not consume HOME.
            assert_eq!(nav.view, View::Scope);
            nav.press(PageButton::Fx);
            assert_eq!(nav.view, View::BeatFx(BeatEffect::Slicer));
            nav.press(PageButton::Fx);
            nav.press(PageButton::Home);
            assert_eq!((nav.view, nav.insert_slot()), (View::Fx, None));
            nav.press(PageButton::Home);
            assert_eq!(nav.view, View::Scope);
        }
    }

    #[test]
    fn master_edit_detour_unwinds_to_the_same_device_then_the_original_bank() {
        let mut nav = Navigation::INITIAL;
        nav.press(PageButton::Fx);
        nav.turn(1, 0);
        nav.press(PageButton::Glo);
        nav.turn(-1, 0);
        let device = View::MasterFx(Some(Device::Dynamics));
        assert_eq!(nav.view, device);
        nav.press(PageButton::Edit);
        nav.turn(1, 0); // Slicer.
        nav.press(PageButton::Fx);
        nav.turn(1, 0);
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.insert_slot()), (View::Fx, None));
        nav.press(PageButton::Home);
        assert_eq!(nav.view, device);
        nav.press(PageButton::Home);
        nav.press(PageButton::Home);
        assert_eq!(nav.view, View::BeatFx(BeatEffect::Slicer));
        nav.press(PageButton::Home);
        assert_eq!(nav.view, View::Scope);
    }

    #[test]
    fn beat_fx_selection_counts_detents_and_channel_focus_preserves_both_grids() {
        let mut nav = Navigation::INITIAL;
        nav.press(PageButton::Fx);
        assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
        nav.turn(2, 0);
        assert_eq!(nav.view, View::BeatFx(BeatEffect::Slicer));
        nav.beat_pattern_mut().expect("Slicer grid").toggle(0);
        nav.turn(0, 1);
        assert_eq!(
            (nav.view, nav.focus),
            (View::BeatFx(BeatEffect::Slicer), Channel::Two)
        );
        assert!(nav.slicer_steps[1].active(0));
        nav.turn(1, 0);
        assert_eq!(nav.view, View::BeatFx(BeatEffect::Filter));
        assert!(nav.beat_pattern_mut().is_none());
        nav.turn(-1, 0);
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
    fn channel_fx_keeps_the_slot_across_channels_and_browses_effects_with_algorithm() {
        let mut nav = Navigation::INITIAL;
        let initial = nav.insert_kinds;
        nav.press(PageButton::Fx);
        nav.press(PageButton::Fx);
        nav.turn_algorithm(1); // Sends overview.
        nav.poll_pickers(0, false, true);
        assert_eq!(nav.insert_kinds, initial);
        assert_eq!(nav.picker_feedback, None);

        nav.turn(1, 0);
        nav.turn_algorithm(-1);
        nav.poll_pickers(1, false, true);
        assert_eq!(nav.preset_role(), PresetRole::Insert(0));
        assert_eq!(nav.insert_kinds[0], [InsertKind::Compressor; 2]);

        nav.turn(0, 2);
        nav.poll_pickers(2, true, false);
        assert_eq!((nav.focus, nav.insert_slot()), (Channel::Three, Some(0)));
        assert_eq!(nav.preset_role(), PresetRole::Channel);
        assert_eq!(nav.insert_kinds[2], initial[2]);

        nav.turn(1, 0);
        nav.turn_algorithm(1);
        nav.poll_pickers(3, false, true);
        assert_eq!(nav.preset_role(), PresetRole::Insert(1));
        assert_eq!(
            nav.insert_kinds[2],
            [InsertKind::None, InsertKind::Compressor]
        );
        assert_eq!(nav.insert_kinds[0], [InsertKind::Compressor; 2]);
        assert_eq!(nav.insert_kinds[1], initial[1]);
        assert_eq!(nav.insert_kinds[3], initial[3]);
        nav.poll_pickers(303, false, false);
        assert_eq!(nav.picker_feedback, None);
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.insert_slot()), (View::Fx, None));
    }

    #[test]
    fn mixer_audition_cycles_in_alphabetical_display_order() {
        let parts = Part::ALL;
        assert!(parts.windows(2).all(|w| w[0].name() < w[1].name()));
        let mut nav = Navigation::INITIAL;
        nav.assignments[0] = Assignment::new(parts[0]);
        nav.press(PageButton::Glo);
        nav.press(PageButton::HoldEdit);
        for &expected in parts.iter().cycle().skip(1).take(parts.len()) {
            nav.turn(0, 1);
            assert_eq!(nav.instrument_swap, Some(expected));
        }
        for &expected in parts.iter().rev() {
            nav.turn(0, -1);
            assert_eq!(nav.instrument_swap, Some(expected));
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
        assert_eq!((nav.view, nav.focus), (View::Channel, Channel::Three));
        assert_eq!(nav.assignments, Navigation::SYNTH_RACK.assignments);
        nav.turn(1, 0);
        assert_eq!(nav.focus, Channel::Four);
        nav.press(PageButton::Home);
        assert_eq!((nav.view, nav.focus), (View::Mixer, Channel::Four));
        nav.press(PageButton::Home);
        assert_eq!(nav.view, View::Scope);
        nav.press(PageButton::Env);
        assert_eq!(nav.view, View::Envelope);
        nav.press(PageButton::Seq);
        nav.press(PageButton::Fx);
        assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
        nav.turn(0, 1);
        assert_eq!(nav.focus, Channel::One);
    }
}
