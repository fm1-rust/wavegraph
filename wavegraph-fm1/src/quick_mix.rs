//! Momentary GLO mixer over an instrument page.

use crate::{
    mixer::Mixer,
    panel::{CHANNELS, Channel, Navigation, View},
};

pub(super) const MUTE_KEYS: [u8; CHANNELS] = [
    crate::fx::STEP_KEYS[0],
    crate::fx::STEP_KEYS[1],
    crate::fx::STEP_KEYS[2],
    crate::fx::STEP_KEYS[3],
];

#[derive(Default)]
pub(super) enum HoldButton {
    #[default]
    Idle,
    Waiting(u32),
    Active,
}

impl HoldButton {
    /// A short, unused press becomes a tap on release. A modifier acts on its first scan.
    pub(super) fn tap(
        &mut self,
        pressed: bool,
        down: bool,
        used: bool,
        tick: u32,
        hold_ticks: u32,
    ) -> bool {
        if !down {
            let tap =
                matches!(self, Self::Waiting(since) if tick.wrapping_sub(*since) < hold_ticks);
            *self = Self::Idle;
            return tap;
        }
        if pressed {
            *self = Self::Waiting(tick);
        }
        if let Self::Waiting(since) = self
            && (used || tick.wrapping_sub(*since) >= hold_ticks)
        {
            *self = Self::Active;
        }
        false
    }

    pub(super) fn active(&self) -> bool {
        matches!(self, Self::Active)
    }
}

pub(super) fn available(nav: &Navigation) -> bool {
    nav.instrument_swap.is_none()
        && matches!(
            nav.view,
            View::Scope
                | View::VoiceMode
                | View::SampleEdit
                | View::Envelope
                | View::Arp
                | View::Lfo
                | View::Fx
        )
}

/// Preserve the original page, its cursor, and focus in the panel's navigation state.
pub(super) fn display(mut nav: Navigation, active: bool) -> Navigation {
    if active {
        nav.view = View::Mixer;
        nav.picker_feedback = None;
    }
    nav
}

/// One edge per strip, including simultaneous SEL and the focused strip's mute key.
pub(super) fn mutes(mixer: &mut Mixer, focus: Channel, sel: bool, pressed: [bool; CHANNELS]) {
    for (i, (strip, key)) in mixer.0.iter_mut().zip(pressed).enumerate() {
        strip.enabled ^= key || (sel && i == focus.index());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::PageButton;

    #[test]
    fn tap_fires_once_and_a_hold_restores_the_editor_and_its_cursor() {
        let mut nav = Navigation::INITIAL;
        nav.press(PageButton::Fx);
        nav.press(PageButton::Fx);
        nav.turn(1, 0);
        let editor = nav;
        let mut mix = HoldButton::default();
        assert!(!mix.tap(true, true, false, u32::MAX - 10, 35));
        assert!(!mix.tap(false, true, false, 23, 35));
        assert_eq!(display(nav, mix.active()), editor);
        assert!(!mix.tap(false, true, false, 24, 35));
        assert_eq!(display(nav, mix.active()).view, View::Mixer);
        assert_eq!(display(nav, mix.active()).focus, nav.focus);
        assert!(!mix.tap(false, false, false, 50, 35));
        assert_eq!(display(nav, mix.active()), editor);

        assert!(!mix.tap(true, true, false, 51, 35));
        assert!(mix.tap(false, false, false, 52, 35));
        assert!(!mix.tap(false, false, false, 53, 35));
        // No scan between crossing the hold deadline and releasing.
        assert!(!mix.tap(true, true, false, 60, 35));
        assert!(!mix.tap(false, false, false, 95, 35));
    }

    #[test]
    fn immediate_mix_claims_only_new_mute_presses_and_consumes_the_glo_release() {
        let mut mix = HoldButton::default();
        let mut mixer = Mixer::INITIAL;
        // A white-key press or fader detent can arrive in the same sweep as GLO.
        assert!(!mix.tap(true, true, true, 0, 35));
        assert!(mix.active());
        assert_eq!(MUTE_KEYS, [0, 2, 4, 6]);
        // First key already held, second just pressed along with SEL on channel 2.
        mutes(&mut mixer, Channel::Two, true, [false, true, false, false]);
        mutes(&mut mixer, Channel::Two, false, [false; CHANNELS]);
        assert_eq!(mixer.0.map(|s| s.enabled), [true, false, true, true]);
        assert!(!mix.tap(false, false, false, 1, 35));
        assert!(!mix.active());
    }

    #[test]
    fn another_page_can_cancel_the_pending_tap_or_an_active_layer() {
        for used in [false, true] {
            let mut mix = HoldButton::Idle;
            assert!(!mix.tap(true, true, used, 0, 35));
            mix = HoldButton::Idle;
            assert!(!mix.tap(false, true, true, 80, 35));
            assert!(!mix.active());
            assert!(!mix.tap(false, false, false, 81, 35));
        }
        let mut nav = Navigation::INITIAL;
        assert!(available(&nav));
        nav.press(PageButton::Seq);
        assert!(!available(&nav));
    }
}
