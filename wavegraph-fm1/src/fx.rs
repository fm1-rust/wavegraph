//! Global rack and channel insert controls.

use crate::instrument::{Message, Part, at};
use heapless::Vec;
use wavegraph_dsp::Event;
use wavegraph_fx::{
    BeatClock, BeatDivision, ChorusControls, Delay, Dynamics, EqControls, FlangerControls,
    MasterControls, Message as FxMessage, PhaserControls, SaturatorControls, SlicerControls,
};

// Original delay-storage budget and upper limit for the free-time rack control.
pub(super) const MAX_DELAY_MS: u16 = 900;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BeatEffect {
    Echo,
    Flanger,
    Slicer,
}

impl BeatEffect {
    pub(super) fn turn(self, detents: i8) -> Self {
        [Self::Echo, Self::Flanger, Self::Slicer]
            [usize::from((self as u8).saturating_add_signed(detents).min(2))]
    }
}

/// Physical white keys from F3 to G5, independent of the keyboard octave.
pub(super) const STEP_KEYS: [u8; 16] = [0, 2, 4, 6, 7, 9, 11, 12, 14, 16, 18, 19, 21, 23, 24, 26];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Device {
    Reverb,
    Delay,
    Eq,
    Dynamics,
}

impl Device {
    pub(super) const ALL: [Self; 4] = [Self::Reverb, Self::Delay, Self::Eq, Self::Dynamics];

    // The first clockwise detent selects the top row; anticlockwise starts at the bottom.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Four rows plus an i8 fit i16"
    )]
    pub(super) fn select(current: Option<Self>, delta: i8) -> Option<Self> {
        if delta == 0 {
            return current;
        }
        let start = current.map_or(if delta > 0 { -1 } else { 0 }, |row| i16::from(row as u8));
        let i = (start + i16::from(delta)).rem_euclid(4);
        Some(Self::ALL[usize::try_from(i).expect("four devices")])
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
    Kick808,
    Kit808,
}

impl KeySource {
    pub(super) const ALL: [KeySource; 7] = [
        Self::Bass,
        Self::Kick,
        Self::Kit,
        Self::Organ,
        Self::Mono,
        Self::Kick808,
        Self::Kit808,
    ];

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "An i8 delta and inlet index fit i16."
    )]
    fn turn(&mut self, detents: i8) {
        let index = (i16::from(*self as u8) + i16::from(detents))
            .rem_euclid(i16::try_from(Self::ALL.len()).expect("small catalog"));
        *self = Self::ALL[usize::try_from(index).expect("wrapped key source index")];
    }

    #[cfg(target_os = "none")]
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Bass => "SELF",
            Self::Kick => "909 KICK",
            Self::Kit => "909 KIT",
            Self::Kick808 => "808 KICK",
            Self::Kit808 => "808 KIT",
            Self::Organ => "M1 ORGAN",
            Self::Mono => "MONO",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Return {
    pub(super) wet: u8,
    pub(super) enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BassFx {
    pub(super) key: KeySource,
    pub(super) depth: u8,
    pub(super) split_hz: u16,
    pub(super) enabled: bool,
}

impl BassFx {
    pub(super) const INITIAL: Self = Self {
        key: KeySource::Kick,
        depth: 75,
        split_hz: 180,
        enabled: true,
    };

    pub(super) fn edit(&mut self, knobs: [i8; 4], toggle: bool) {
        self.key.turn(knobs[0]);
        self.depth = self.depth.saturating_add_signed(knobs[1]).min(100);
        self.split_hz = self
            .split_hz
            .saturating_add_signed(i16::from(knobs[2]).saturating_mul(5))
            .clamp(40, 1000);
        self.enabled ^= toggle;
    }
}

pub(super) const INSERT_SLOTS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InsertKind {
    None,
    Compressor,
    Saturator,
    Phaser,
    Flanger,
    Eq,
    Chorus,
}

impl InsertKind {
    pub(super) const ALL: [Self; 7] = [
        Self::None,
        Self::Compressor,
        Self::Saturator,
        Self::Phaser,
        Self::Flanger,
        Self::Eq,
        Self::Chorus,
    ];

    pub(super) const fn factory(part: Part) -> [Self; INSERT_SLOTS] {
        [
            if matches!(part, Part::Bass) {
                Self::Saturator
            } else {
                Self::None
            },
            Self::None,
        ]
    }

    pub(super) fn turn(self, detents: i8) -> Self {
        let i = i16::from(self as u8)
            .saturating_add(i16::from(detents))
            .rem_euclid(i16::try_from(Self::ALL.len()).expect("small effect catalog"));
        Self::ALL[usize::try_from(i).expect("wrapped effect index")]
    }

    #[cfg(target_os = "none")]
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Compressor => "SIDECHAIN",
            Self::Saturator => "SATURATOR",
            Self::Phaser => "PHASER",
            Self::Flanger => "FLANGER",
            Self::Chorus => "CHORUS",
            Self::Eq => "3-BAND EQ",
        }
    }

    pub(super) const fn initial(self, part: Part) -> Insert {
        match self {
            Self::None => Insert::None,
            Self::Compressor => Insert::Compressor(BassFx::INITIAL),
            // Keep some dry attack and match the factory 303 levels.
            Self::Saturator if matches!(part, Part::Bass) => Insert::Saturator(SaturatorControls {
                drive: 80,
                mix: 96,
                level: 28,
                ..SaturatorControls::INITIAL
            }),
            Self::Saturator => Insert::Saturator(SaturatorControls::INITIAL),
            Self::Phaser => Insert::Phaser(PhaserControls::INITIAL),
            Self::Flanger => Insert::Flanger(FlangerControls::INITIAL),
            Self::Chorus => Insert::Chorus(ChorusControls::INITIAL),
            Self::Eq => Insert::Eq(EqControls::INITIAL),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Insert {
    None,
    Compressor(BassFx),
    Saturator(SaturatorControls),
    Phaser(PhaserControls),
    Flanger(FlangerControls),
    Chorus(ChorusControls),
    Eq(EqControls),
}

fn edit_eq(db: &mut [i8; 3], knobs: [i8; 4]) {
    for (gain, delta) in db.iter_mut().zip(knobs) {
        *gain = gain.saturating_add(delta).clamp(-12, 12);
    }
}

impl Insert {
    pub(super) const fn factory(part: Part) -> [Self; INSERT_SLOTS] {
        let [first, second] = InsertKind::factory(part);
        [first.initial(part), second.initial(part)]
    }

    pub(super) const fn kind(self) -> InsertKind {
        match self {
            Self::None => InsertKind::None,
            Self::Compressor(_) => InsertKind::Compressor,
            Self::Saturator(_) => InsertKind::Saturator,
            Self::Phaser(_) => InsertKind::Phaser,
            Self::Flanger(_) => InsertKind::Flanger,
            Self::Chorus(_) => InsertKind::Chorus,
            Self::Eq(_) => InsertKind::Eq,
        }
    }

    pub(super) fn add_shape(&mut self, offset: u8) {
        if let Self::Saturator(fx) = self {
            fx.shape = fx.shape.saturating_add(offset).min(128);
        }
    }

    pub(super) const fn enabled(self) -> bool {
        match self {
            Self::None => false,
            Self::Compressor(fx) => fx.enabled,
            Self::Saturator(fx) => fx.enabled,
            Self::Phaser(fx) | Self::Flanger(fx) => fx.enabled,
            Self::Chorus(fx) => fx.enabled,
            Self::Eq(fx) => fx.enabled,
        }
    }

    pub(super) fn edit(&mut self, knobs: [i8; 4], toggle: bool) {
        let turn = |value: &mut u8, delta| *value = value.saturating_add_signed(delta).min(128);
        match self {
            Self::None => (),
            Self::Compressor(fx) => fx.edit(knobs, toggle),
            Self::Saturator(fx) => {
                turn(&mut fx.drive, knobs[0]);
                turn(&mut fx.mix, knobs[1]);
                turn(&mut fx.level, knobs[2]);
                turn(&mut fx.shape, knobs[3]);
                fx.enabled ^= toggle;
            }
            Self::Chorus(fx) => {
                fx.rate = fx.rate.saturating_add_signed(knobs[0]).min(127);
                turn(&mut fx.depth, knobs[1]);
                turn(&mut fx.tone, knobs[2]);
                fx.mix = fx.mix.saturating_add_signed(knobs[3]).min(100);
                fx.enabled ^= toggle;
            }
            Self::Eq(fx) => {
                edit_eq(&mut fx.db, knobs);
                fx.enabled ^= toggle;
            }
            Self::Phaser(fx) | Self::Flanger(fx) => {
                turn(&mut fx.rate, knobs[0]);
                turn(&mut fx.depth, knobs[1]);
                turn(&mut fx.feedback, knobs[2]);
                turn(&mut fx.mix, knobs[3]);
                fx.enabled ^= toggle;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Fx {
    pub(super) reverb: Return,
    pub(super) delay: Return,
    pub(super) decay: u8,
    pub(super) time_ms: u16,
    pub(super) beats: BeatDivision,
    pub(super) clocked_delay: bool,
    pub(super) echo_input_open: bool,
    pub(super) flanger_target: Option<crate::panel::Channel>,
    pub(super) beat_flanger: FlangerControls,
    pub(super) slicers: [SlicerControls; crate::panel::CHANNELS],
    pub(super) feedback: u8,
    pub(super) eq: [i8; 3],
    pub(super) eq_enabled: bool,
    pub(super) dynamics: Dynamics,
    pub(super) dynamics_enabled: bool,
}

impl Fx {
    pub(super) const INITIAL: Self = Self {
        reverb: Return {
            wet: 104,
            enabled: true,
        },
        delay: Return {
            wet: 76,
            enabled: true,
        },
        decay: 90,
        clocked_delay: true,
        echo_input_open: true,
        beats: BeatDivision::DOTTED_EIGHTH,
        flanger_target: None,
        beat_flanger: FlangerControls::INITIAL,
        slicers: [SlicerControls::INITIAL; crate::panel::CHANNELS],
        time_ms: 369, // Dotted eighth at the factory 122 BPM.
        feedback: 43,
        eq: [0; 3],
        eq_enabled: true,
        dynamics: Dynamics::GLUE,
        dynamics_enabled: true,
    };

    pub(super) fn edit_beat(
        &mut self,
        channel: crate::panel::Channel,
        effect: BeatEffect,
        knobs: [i8; 4],
        sel: bool,
    ) {
        let [beats, depth, feedback, color] = knobs;
        self.beats = self.beats.turn(beats);
        match effect {
            BeatEffect::Echo => {
                self.clocked_delay = true;
                // Depth is the focused strip's existing post-fader send.
                self.feedback = self
                    .feedback
                    .saturating_add_signed(feedback)
                    .min(Delay::MAX_FEEDBACK);
                self.delay.wet = self.delay.wet.saturating_add_signed(color).min(128);
                if sel {
                    // A closed send lets the captured phrase ring. SEL also recalls a muted return.
                    self.echo_input_open = !(self.echo_input_open && self.delay.enabled);
                    self.delay.enabled = true;
                }
            }
            BeatEffect::Slicer => {
                let slicer = &mut self.slicers[channel.index()];
                slicer.depth = slicer.depth.saturating_add_signed(depth).min(128);
                slicer.enabled ^= sel;
            }
            BeatEffect::Flanger => {
                let f = &mut self.beat_flanger;
                f.mix = f.mix.saturating_add_signed(depth).min(128);
                f.feedback = f.feedback.saturating_add_signed(feedback).min(128);
                f.depth = f.depth.saturating_add_signed(color).min(128);
                if sel {
                    self.flanger_target = (self.flanger_target != Some(channel)).then_some(channel);
                }
            }
        }
    }

    pub(super) fn enabled(self, device: Device) -> bool {
        match device {
            Device::Reverb => self.reverb.enabled,
            Device::Delay => self.delay.enabled,
            Device::Eq => self.eq_enabled,
            Device::Dynamics => self.dynamics_enabled,
        }
    }

    pub(super) fn edit(&mut self, device: Device, knobs: [i8; 4], toggle: bool) {
        match device {
            Device::Reverb => {
                self.decay = self.decay.saturating_add_signed(knobs[0]).min(127);
                self.reverb.wet = self.reverb.wet.saturating_add_signed(knobs[1]).min(128);
                self.reverb.enabled ^= toggle;
            }
            Device::Delay => {
                if knobs[3] != 0 {
                    self.clocked_delay = knobs[3] > 0;
                }
                if !self.clocked_delay {
                    self.time_ms = self
                        .time_ms
                        .saturating_add_signed(i16::from(knobs[0]).saturating_mul(5))
                        .clamp(5, MAX_DELAY_MS);
                }
                self.feedback = self
                    .feedback
                    .saturating_add_signed(knobs[1])
                    .min(Delay::MAX_FEEDBACK);
                self.delay.wet = self.delay.wet.saturating_add_signed(knobs[2]).min(128);
                self.delay.enabled ^= toggle;
            }
            Device::Eq => {
                edit_eq(&mut self.eq, knobs);
                self.eq_enabled ^= toggle;
            }
            Device::Dynamics => {
                let d = &mut self.dynamics;
                d.threshold_db = d.threshold_db.saturating_add(knobs[0]).clamp(-36, 0);
                d.ratio_x2 = d.ratio_x2.saturating_add_signed(knobs[1]).clamp(2, 16);
                d.makeup_db = d.makeup_db.saturating_add_signed(knobs[2]).min(12);
                d.ceiling_db = d.ceiling_db.saturating_add(knobs[3]).clamp(-12, 0);
                self.dynamics_enabled ^= toggle;
            }
        }
    }

    pub(super) fn return_events(
        self,
        previous: Option<Self>,
        device: Device,
        bpm: u16,
        previous_bpm: u16,
    ) -> Vec<Event<Message>, 4> {
        let mut events = Vec::new();
        let mut push = |changed, message: FxMessage| {
            if changed {
                events
                    .push(at(message))
                    .expect("at most four return controls");
            }
        };
        let (out, old) = match device {
            Device::Reverb => {
                push(
                    previous.is_none_or(|p| p.decay != self.decay),
                    FxMessage::Decay(self.decay),
                );
                (self.reverb, previous.map(|p| p.reverb))
            }
            Device::Delay => {
                if self.clocked_delay {
                    push(
                        previous.is_none_or(|p| !p.clocked_delay || p.beats != self.beats)
                            || bpm != previous_bpm,
                        FxMessage::BeatTime(BeatClock {
                            beats: self.beats,
                            bpm,
                        }),
                    );
                } else {
                    push(
                        previous.is_none_or(|p| p.clocked_delay || p.time_ms != self.time_ms),
                        FxMessage::TimeMs(self.time_ms),
                    );
                }
                push(
                    previous.is_none_or(|p| p.feedback != self.feedback),
                    FxMessage::Feedback(self.feedback),
                );
                (self.delay, previous.map(|p| p.delay))
            }
            Device::Eq | Device::Dynamics => return events,
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

    pub(super) fn master_events(self, previous: Option<Self>) -> Vec<Event<Message>, 3> {
        let mut events = Vec::new();
        let controls = |fx: Self| {
            [
                MasterControls::Eq(fx.eq_enabled.then_some(fx.eq)),
                MasterControls::Dynamics(fx.dynamics_enabled.then_some(fx.dynamics)),
            ]
        };
        let old = previous.map(controls);
        for (i, control) in controls(self).into_iter().enumerate() {
            if old.is_none_or(|o| o[i] != control) {
                events
                    .push(at(Message::Master(control)))
                    .expect("two master inserts");
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_beat_value_follows_the_editor_and_only_a_tempo_change_retimes_echo() {
        use crate::panel::Channel;
        let mut fx = Fx::INITIAL;
        fx.edit_beat(Channel::Three, BeatEffect::Echo, [1, 0, 0, 0], false);
        assert_eq!(fx.beats, BeatDivision::ONE);
        fx.edit_beat(Channel::Three, BeatEffect::Flanger, [0; 4], true);
        assert_eq!(fx.flanger_target, Some(Channel::Three));
        fx.edit_beat(Channel::One, BeatEffect::Flanger, [-1, 0, 0, 0], false);
        assert_eq!(fx.beats, BeatDivision::DOTTED_EIGHTH);
        let events = fx.return_events(Some(fx), Device::Delay, 100, 122);
        assert_eq!(
            events.as_slice(),
            &[at(FxMessage::BeatTime(BeatClock {
                beats: fx.beats,
                bpm: 100
            }))]
        );
        assert!(
            fx.return_events(Some(fx), Device::Delay, 100, 100)
                .is_empty()
        );
        fx.edit(Device::Delay, [0, 0, 0, -1], false);
        assert!(
            fx.return_events(Some(fx), Device::Delay, 100, 122)
                .is_empty()
        );
        assert_eq!(
            STEP_KEYS.map(|k| (53 + k) % 12),
            [5, 7, 9, 11, 0, 2, 4, 5, 7, 9, 11, 0, 2, 4, 5, 7]
        );
    }

    #[test]
    fn echo_sel_recalls_a_muted_return_then_closes_only_its_input() {
        use crate::panel::Channel;
        for open in [false, true] {
            let mut fx = Fx::INITIAL;
            fx.echo_input_open = open;
            fx.edit(Device::Delay, [0; 4], true);
            assert!(!fx.delay.enabled);
            fx.edit_beat(Channel::One, BeatEffect::Echo, [0; 4], true);
            assert!(fx.echo_input_open && fx.delay.enabled);
            fx.edit_beat(Channel::One, BeatEffect::Echo, [0; 4], true);
            assert!(!fx.echo_input_open && fx.delay.enabled);
        }
    }

    #[test]
    fn insert_bypass_preserves_edited_controls() {
        for kind in InsertKind::ALL {
            let mut insert = kind.initial(Part::Mono);
            insert.edit([3, -4, 5, -6], false);
            assert_eq!(insert.kind(), kind);
            assert_eq!(insert.enabled(), kind != InsertKind::None);
            let edited = insert;
            insert.edit([0; 4], true);
            assert!(!insert.enabled());
            insert.edit([0; 4], true);
            assert_eq!(insert, edited);
        }
    }

    #[test]
    fn delay_time_is_bounded_by_the_allocated_line() {
        let mut fx = Fx::INITIAL;
        assert_eq!(fx.time_ms, 369);
        for _ in 0..3 {
            fx.edit(Device::Delay, [127, 0, 0, -1], false);
        }
        assert_eq!(fx.time_ms, MAX_DELAY_MS);
    }

    #[test]
    fn rack_edits_are_isolated_and_bypass_recalls_the_settings() {
        let mut fx = Fx::INITIAL;
        fx.edit(Device::Eq, [-127, 6, 127, 100], false);
        assert_eq!(fx.eq, [-12, 6, 12]);
        assert_eq!(fx.master_events(Some(Fx::INITIAL)).len(), 1);
        fx.edit(Device::Dynamics, [-100, 2, 6, -3], false);
        assert_eq!(
            fx.dynamics,
            Dynamics {
                threshold_db: -36,
                ratio_x2: 6,
                makeup_db: 8,
                ceiling_db: -4
            }
        );
        let before = fx;
        fx.edit(Device::Dynamics, [0; 4], true);
        assert!(!fx.enabled(Device::Dynamics));
        assert!(fx.enabled(Device::Eq));
        assert_eq!(
            fx.master_events(Some(before))[0].message,
            Message::Master(MasterControls::Dynamics(None))
        );
        fx.edit(Device::Dynamics, [0; 4], true);
        assert_eq!(fx, before);
        assert!(
            fx.return_events(Some(Fx::INITIAL), Device::Reverb, 122, 122)
                .is_empty()
        );
        assert!(
            fx.return_events(Some(Fx::INITIAL), Device::Delay, 122, 122)
                .is_empty()
        );
    }
}
