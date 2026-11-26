//! Channel selection and page navigation for the FM-1 panel.

use wavegraph_organ::{Chord, VoiceMode};

#[cfg(test)]
use crate::bass::Knob;
use crate::{
    bass::{Control, PresetSelection, presets::PRESETS},
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

/// A scope tap is resolved on release. All other EDIT actions still use the press edge.
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
pub(super) struct Navigation {
    pub(super) clip_request: crate::clipboard::Request,
    pub(super) view: View,
    pub(super) focus: Channel,
    pub(super) preset_since: Option<u32>,
    /// Instrument shown in the swap header. The current assignment keeps playing until it settles.
    pub(super) instrument_swap: Option<Part>,
    bass_slot: Option<u8>,
    fx_parent: View,
    beat_parent: View,
    pub(super) beat_steps: [wavegraph_fx::StepPattern; CHANNELS],
    pub(super) slicer_steps: [wavegraph_fx::StepPattern; CHANNELS],
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
        preset_since: None,
        instrument_swap: None,
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
            // Leaving the scope or confirming with EDIT takes the displayed choice immediately.
            self.assign(selected);
            self.preset_since = None;
            self.bass_slot = None;
            if matches!(button, PageButton::Edit | PageButton::Home) {
                return;
            }
        }
        if matches!(button, PageButton::Home) && self.bass_slot.take().is_some() {
            return;
        }
        if matches!(button, PageButton::Home) && self.insert_slot.take().is_some() {
            return;
        }
        let next = match button {
            PageButton::HoldEdit => {
                if self.view == View::Scope {
                    self.instrument_swap = Some(self.part());
                    self.preset_since = None;
                    self.bass_slot = None;
                }
                return;
            }
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

    pub(super) fn preset_role(&self) -> PresetRole {
        match (self.view, self.insert_slot) {
            (View::Channel, _) => PresetRole::Instrument,
            (View::Scope, _) => self
                .instrument_swap
                .map_or(PresetRole::Channel, PresetRole::Audition),
            (View::Fx, Some(slot)) => PresetRole::Insert(slot),
            _ => PresetRole::Channel,
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

    /// Settle from the last PRESETS detent, even if the scan carrying it could not be queued.
    pub(super) fn poll_presets(&mut self, tick: u32, turned: bool) {
        const SETTLE_TICKS: u32 = 20; // 200 ms at 100 Hz
        if turned {
            self.preset_since = Some(tick);
        }
        let Some(since) = self.preset_since else {
            return;
        };
        if let Some(selected) = self.instrument_swap
            && selected != self.part()
            && tick.wrapping_sub(since) >= SETTLE_TICKS
        {
            self.assign(selected);
        }
        // Auditioning the sound and dismissing the picker have independent deadlines.
        if preset_age(since, tick).is_none() {
            self.preset_since = None;
        }
    }

    pub(super) fn insert_slot(self) -> Option<u8> {
        self.insert_slot.filter(|_| self.view == View::Fx)
    }

    #[cfg(test)]
    pub(super) fn bass_knob(self) -> Option<Knob> {
        match self.bass_control()? {
            Control::Knob(knob) => Some(knob),
            Control::Waveform => None,
        }
    }

    pub(super) fn bass_control(self) -> Option<Control> {
        self.bass_slot.map(|i| Control::PANEL[usize::from(i)])
    }

    /// Preset requests and their insert types travel in the same retained snapshot.
    pub(super) fn turn_algorithm(&mut self, detents: i8) {
        if detents == 0
            || self.part() != Part::Bass
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
            let count = i16::try_from(Control::PANEL.len()).expect("nine controls");
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
        match self.preset_role() {
            PresetRole::Insert(slot) => {
                let kind = &mut self.insert_kinds[self.focus.index()][usize::from(slot)];
                *kind = kind.turn(presets);
            }
            PresetRole::Instrument => {
                let mut part = self.part();
                part.turn(presets);
                self.assign(part);
            }
            PresetRole::Audition(mut selected) => {
                selected.turn(presets);
                self.instrument_swap = Some(selected);
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
    fn scope_swap_uses_the_mixer_order_and_only_reassigns_the_focused_strip() {
        for ch in Channel::ALL {
            let mut nav = Navigation::INITIAL;
            nav.focus = ch;
            let mut mixer = nav;
            mixer.press(PageButton::Glo);
            mixer.turn(1, 0);
            nav.press(PageButton::HoldEdit);
            assert_eq!(nav.instrument_swap, Some(nav.part()));
            for delta in [1, -2, 10, i8::MIN, i8::MAX] {
                let previous = nav.assignments;
                nav.turn(0, delta);
                mixer.turn(0, delta);
                nav.poll_presets(0, true);
                assert_eq!(nav.assignments, previous);
                assert_eq!(nav.instrument_swap, Some(mixer.part()));
                nav.poll_presets(20, false);
                assert_eq!(nav.assignments, mixer.assignments);
                assert_eq!(
                    (nav.focus, nav.view, nav.preset_role()),
                    (ch, View::Scope, PresetRole::Audition(mixer.part()))
                );
            }
            nav.press(PageButton::Edit);
            assert_eq!(nav.instrument_swap, None);
            assert_eq!(
                (nav.view, nav.preset_role()),
                (View::Scope, PresetRole::Channel)
            );
            nav.turn(0, 1);
            assert_ne!(nav.focus, ch);
            assert_eq!(nav.assignments, mixer.assignments);
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
            nav.poll_presets(now, true);
            assert_eq!(nav.instrument_swap, Some(name));
            assert_eq!(nav.assignments, original);
        }
        nav.poll_presets(24, false);
        assert_eq!(nav.assignments, original);
        nav.poll_presets(25, false);
        assert_eq!(nav.preset_since, Some(5)); // The picker stays up after the sound changes.
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
        nav.poll_presets(50, true);
        nav.turn(0, -1);
        nav.poll_presets(55, true);
        nav.poll_presets(300, false);
        assert_eq!(nav.preset_since, None);
        assert_eq!(nav.assignments, loaded);
        // EDIT takes the displayed choice even a single tick after the detent.
        nav.turn(0, -1);
        nav.poll_presets(310, true);
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
        nav.poll_presets(311, false);
        nav.turn(0, 1);
        nav.poll_presets(500, false);
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
            nav.turn(1, 0);
            assert!(nav.insert_slot().is_some());
            nav.press(PageButton::Edit);
            assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
            nav.press(PageButton::Home);
            assert_eq!(nav.view, View::Fx);
            nav.press(PageButton::Glo);
            nav.turn(1, 0);
            let rack = nav.view;
            nav.press(PageButton::Edit);
            assert_eq!(nav.view, View::BeatFx(BeatEffect::Echo));
            nav.press(PageButton::Edit);
            assert_eq!(nav.view, rack);
        }
    }

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
        assert_eq!(nav.bass_knob(), Some(Knob::Attack));
        nav.turn(i8::MAX, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Overdrive));
        nav.turn(2, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Tuning));

        nav.turn(0, 1);
        assert_eq!((nav.part(), nav.bass_knob()), (Part::Bass, None));
        nav.turn(4, 0);
        assert_eq!(nav.bass_knob(), Some(Knob::Tuning));
        nav.press(PageButton::Env);
        assert_eq!((nav.view, nav.bass_knob()), (View::Envelope, None));
        nav.turn(-4, 0);
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
            [InsertKind::Saturator, InsertKind::Compressor]
        );
        nav.turn(0, -1);
        assert_eq!(
            nav.insert_kinds[0],
            [InsertKind::Compressor, InsertKind::Compressor]
        );
        nav.turn(1, -2);
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
