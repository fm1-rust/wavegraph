//! Four channel faders, post-fader sends and peak metering.

use crate::{
    instrument::{EventBatch, at},
    panel::{CHANNELS, Channel},
};
use wavegraph_fx::MixerMessage;
use wavegraph_sample::I24;

pub(super) const UNITY: u8 = 128;
pub(super) const MAX_LEVEL: u8 = 255;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Strip {
    pub(super) enabled: bool,
    pub(super) level: u8,
    pub(super) reverb: u8,
    pub(super) delay: u8,
}

impl Strip {
    const UNITY: Self = Self {
        enabled: true,
        level: UNITY,
        reverb: 0,
        delay: 0,
    };

    pub(super) fn output_level(self) -> u8 {
        if self.enabled { self.level } else { 0 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Mixer(pub(super) [Strip; CHANNELS]);

impl Mixer {
    // Instrument headroom is reserved upstream of the faders.
    pub(super) const DRY: Self = Self([Strip::UNITY; CHANNELS]);
    pub(super) const INITIAL: Self = Self([
        Strip {
            enabled: true,
            level: 108,
            reverb: 80,
            delay: 104,
        },
        Strip {
            enabled: true,
            level: 120,
            reverb: 96,
            delay: 112,
        },
        Strip {
            enabled: true,
            level: 88,
            reverb: 12,
            delay: 0,
        },
        Strip {
            level: 80,
            ..Strip::UNITY
        },
    ]);

    pub(super) fn turn_knobs(&mut self, channel: Channel, [level, reverb, delay, _]: [i8; 4]) {
        let strip = &mut self.0[channel.index()];
        strip.level = turn(strip.level, level, MAX_LEVEL);
        strip.reverb = turn(strip.reverb, reverb, UNITY);
        strip.delay = turn(strip.delay, delay, UNITY);
    }

    pub(super) fn turn_faders(&mut self, knobs: [i8; CHANNELS]) {
        for (strip, delta) in self.0.iter_mut().zip(knobs) {
            strip.level = turn(strip.level, delta, MAX_LEVEL);
        }
    }

    pub(super) fn events(self, previous: Option<Self>) -> EventBatch {
        let mut events = EventBatch::new();
        for channel in Channel::ALL {
            let track = channel as u8;
            let strip = self.0[channel.index()];
            let old = previous.map(|mixer| mixer.0[channel.index()]);
            for (changed, message) in [
                (
                    old.is_none_or(|old| old.output_level() != strip.output_level()),
                    MixerMessage::Level {
                        track,
                        level: strip.output_level(),
                    },
                ),
                (
                    old.is_none_or(|old| old.reverb != strip.reverb),
                    MixerMessage::Send {
                        track,
                        level: strip.reverb,
                    },
                ),
                (
                    old.is_none_or(|old| old.delay != strip.delay),
                    MixerMessage::SendTo {
                        send: 1,
                        track,
                        level: strip.delay,
                    },
                ),
            ] {
                if changed {
                    events
                        .push(at(message))
                        .expect("at most twelve mixer changes");
                }
            }
        }
        events
    }
}

fn turn(value: u8, detents: i8, max: u8) -> u8 {
    if detents == 0 {
        return value;
    }
    // Round the saturated endpoint (255) back to detent 64 so unity
    // is reachable again in either direction.
    let position = u16::from(value).div_ceil(4);
    let next = position.saturating_add_signed(i16::from(detents));
    u8::try_from(next.saturating_mul(4).min(u16::from(max))).expect("bounded mixer control")
}

// Peak thresholds in Q23 at -54, -48, -42, -36, -30, -24, -18,
// -12, -9, -6, -3 and 0 dBFS. Include the negative rail at full scale.
pub(super) const SEGMENTS: usize = 12;
const THRESHOLDS: [u32; SEGMENTS] = [
    16_737, 33_396, 66_633, 132_950, 265_271, 529_285, 1_056_063, 2_107_123, 2_976_391, 4_204_264,
    5_938_680, 8_388_607,
];
const HOLD_BLOCKS: u8 = 12; // 35 ms at the FM-1's 128-frame block size.

#[derive(Default)]
pub(super) struct Meter {
    peak: [u32; CHANNELS],
    hold: [u8; CHANNELS],
}

impl Meter {
    #[cfg(target_os = "none")]
    pub(super) fn clear(&mut self, channel: usize) {
        self.peak[channel] = 0;
        self.hold[channel] = 0;
    }

    pub(super) fn update<const N: usize>(
        &mut self,
        mixer: Mixer,
        buses: &[[I24; N]],
    ) -> [u8; CHANNELS] {
        core::array::from_fn(|i| {
            let input = buses[i]
                .iter()
                .map(|sample| sample.inner().unsigned_abs())
                .max()
                .unwrap_or(0);
            let level = mixer.0[i].output_level();
            let peak = input.saturating_mul(u32::from(level)) >> 7;
            if level == 0 {
                self.peak[i] = 0;
                self.hold[i] = 0;
            } else if peak >= self.peak[i] {
                self.peak[i] = peak;
                self.hold[i] = HOLD_BLOCKS;
            } else if self.hold[i] != 0 {
                self.hold[i] = self.hold[i].saturating_sub(1);
            } else {
                self.peak[i] = peak.max(self.peak[i].saturating_sub((self.peak[i] >> 5).max(1)));
            }
            u8::try_from(THRESHOLDS.partition_point(|&threshold| threshold <= self.peak[i]))
                .expect("twelve meter segments")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_knobs_move_the_four_faders_without_touching_the_sends() {
        let mut mixer = Mixer::INITIAL;
        mixer.turn_faders([1, -2, i8::MIN, i8::MAX]);
        assert_eq!(mixer.0.map(|s| s.level), [112, 112, 0, 255]);
        assert_eq!(
            mixer.0.map(|s| (s.reverb, s.delay)),
            Mixer::INITIAL.0.map(|s| (s.reverb, s.delay))
        );
        assert_eq!(mixer.events(Some(Mixer::INITIAL)).len(), 4);
    }

    #[test]
    fn strip_edits_keep_the_other_strips_and_support_boost() {
        let mut mixer = Mixer::INITIAL;
        mixer.turn_knobs(Channel::Two, [i8::MAX, 1, 2, 100]);
        assert_eq!(
            mixer.0[1],
            Strip {
                enabled: true,
                level: MAX_LEVEL,
                reverb: 100,
                delay: 120
            }
        );
        assert_eq!(mixer.0[0], Mixer::INITIAL.0[0]);
        assert_eq!(mixer.0[2..], Mixer::INITIAL.0[2..]);
        assert_eq!(mixer.events(Some(Mixer::INITIAL)).len(), 3);
        mixer.turn_knobs(Channel::Two, [-32, 0, 0, 0]);
        assert_eq!(mixer.0[1].level, UNITY);
        mixer.turn_knobs(Channel::Two, [i8::MIN; 4]);
        assert_eq!(
            mixer.0[1],
            Strip {
                enabled: true,
                level: 0,
                reverb: 0,
                delay: 0
            }
        );
    }

    #[test]
    fn muted_fader_edits_are_recalled_on_enable_and_clear_the_post_fader_meter() {
        let mut mixer = Mixer::INITIAL;
        let original = mixer;
        let ch = Channel::Two;
        let mut meter = Meter::default();
        let loud = [[I24::new_unchecked(1 << 22)]; CHANNELS];
        let peaks = meter.update(mixer, &loud);
        mixer.0[ch.index()].enabled = false;
        assert_eq!(
            mixer.events(Some(original)).as_slice(),
            &[at(MixerMessage::Level { track: 1, level: 0 })]
        );
        assert_eq!(
            meter.update(mixer, &loud),
            [peaks[0], 0, peaks[2], peaks[3]]
        );
        let muted = mixer;
        mixer.turn_knobs(ch, [3, 0, 0, 0]);
        assert!(mixer.events(Some(muted)).is_empty());
        let edited = mixer;
        mixer.0[ch.index()].enabled = true;
        assert_eq!(
            mixer.events(Some(edited)).as_slice(),
            &[at(MixerMessage::Level {
                track: 1,
                level: 132
            })]
        );
        assert!(meter.update(mixer, &loud)[ch.index()] > 0);
    }

    #[test]
    fn each_meter_measures_its_post_fader_peak_and_holds_transients() {
        let sample = I24::new_unchecked(1 << 22);
        let mut buses = [[sample], [I24::default()], [sample], [sample]];
        let mut mixer = Mixer::DRY;
        mixer.0[1].level = 0;
        mixer.0[2].level = MAX_LEVEL;
        buses[2][0] = I24::new_unchecked(-(1 << 23));
        let mut meter = Meter::default();
        let lit = meter.update(mixer, &buses);
        assert_eq!(lit, [9, 0, 12, 9]);
        buses[2][0] = I24::default();
        for _ in 0..HOLD_BLOCKS {
            assert_eq!(meter.update(mixer, &buses)[2], 12);
        }
        mixer.0[0].level = 0;
        for _ in 0..330 {
            meter.update(mixer, &buses);
        }
        assert_eq!(meter.update(mixer, &buses), [0, 0, 0, 9]);
    }
}
