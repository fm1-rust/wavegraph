//! Original 303 voice and insert patches; these do not replace sequence notes.
//!
//! The additional patches explore Reason/ReBirth-style acid, hollow square bass,
//! and driven 303 sounds. Pump Panel is an approximation inspired by Confusion,
//! not a transcription of the artists' settings or a model of their unknown pedal.

use super::Controls;
use crate::fx::{BassFx, INSERT_SLOTS, Insert};
use crate::instrument::Part;
#[cfg(any(test, target_os = "none"))]
use crate::{fx::Fx, mixer::Mixer, panel::Channel};
use wavegraph_fx::{PhaserControls, SaturatorControls, SlicerControls, StepPattern};

/// A complete sound, including both channel inserts and channel-owned Beat FX.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Preset {
    pub name: &'static str,
    pub controls: Controls,
    pub waveform: u8,
    pub inserts: [Insert; INSERT_SLOTS],
    pub echo_send: u8,
    pub echo_steps: StepPattern,
    pub slicer: SlicerControls,
    pub slicer_steps: StepPattern,
}

impl Preset {
    /// Do not change the shared beat clock, Echo return, or another strip's Flanger.
    #[cfg(any(test, target_os = "none"))]
    pub(crate) fn apply_beat_fx(self, channel: Channel, fx: &mut Fx, mixer: &mut Mixer) {
        let i = channel.index();
        mixer.0[i].delay = self.echo_send;
        fx.slicers[i] = self.slicer;
    }
}

const fn saturation(drive: u8, mix: u8, level: u8, shape: u8) -> Insert {
    Insert::Saturator(SaturatorControls {
        drive,
        mix,
        level,
        shape,
        enabled: true,
    })
}

const SIDECHAIN: Insert = Insert::Compressor(BassFx::INITIAL);

/// Factory tracks first, followed by original sound-design starting points.
pub(crate) const PRESETS: [Preset; 7] = [
    Preset {
        name: "DEEP HOUSE",
        echo_send: 104,
        echo_steps: StepPattern::ALL,
        slicer: SlicerControls::INITIAL,
        slicer_steps: StepPattern::FIRST_HALF,
        controls: Controls::DEEP_HOUSE,
        waveform: 0,
        inserts: Insert::factory(Part::Bass),
    },
    Preset {
        name: "HARMONY",
        echo_send: 112,
        echo_steps: StepPattern::ALL,
        slicer: SlicerControls::INITIAL,
        slicer_steps: StepPattern::FIRST_HALF,
        controls: Controls::HARMONY,
        waveform: 1,
        inserts: Insert::factory(Part::Bass),
    },
    Preset {
        name: "PUMP PANEL",
        echo_send: 32,
        echo_steps: StepPattern::from_bits(0x8888),
        slicer: SlicerControls::INITIAL,
        slicer_steps: StepPattern::FIRST_HALF,
        controls: Controls {
            // A pedal after the VCA retains the resonant filter's sweep before clipping.
            // 500 Hz, 86% resonance, 3 octaves, 450 ms, 92% accent, 67% OD.
            values: [18, 86, 36, 45, 120, 5, 92, 24],
            ..Controls::INITIAL
        },
        waveform: 0,
        inserts: [saturation(16, 32, 96, 0), SIDECHAIN],
    },
    Preset {
        name: "REBIRTH BITE",
        echo_send: 72,
        echo_steps: StepPattern::ALL,
        slicer: SlicerControls::INITIAL,
        slicer_steps: StepPattern::FIRST_HALF,
        controls: Controls {
            // 397 Hz, 78% resonance, 3 octaves, 400 ms, 80% accent, 28% OD.
            values: [16, 78, 36, 40, 120, 5, 80, 10],
            ..Controls::INITIAL
        },
        waveform: 0,
        inserts: [saturation(64, 96, 80, 40), SIDECHAIN],
    },
    Preset {
        name: "RUBBER SQR",
        echo_send: 48,
        echo_steps: StepPattern::ALL,
        slicer: SlicerControls::INITIAL,
        slicer_steps: StepPattern::FIRST_HALF,
        controls: Controls {
            // 250 Hz, 48% resonance, 20 semitones, 260 ms, 65% accent, 8% OD.
            values: [12, 48, 20, 26, 120, 5, 65, 3],
            ..Controls::INITIAL
        },
        waveform: 1,
        inserts: [saturation(96, 128, 96, 0), SIDECHAIN],
    },
    Preset {
        name: "ACID PLUCK",
        echo_send: 100,
        echo_steps: StepPattern::from_bits(0x1111),
        slicer: SlicerControls::INITIAL,
        slicer_steps: StepPattern::FIRST_HALF,
        controls: Controls {
            // 354 Hz, 86% resonance, 3.5 octaves, 140 ms, 0.75 ms attack, 17% OD.
            values: [15, 86, 42, 14, 120, 3, 90, 6],
            ..Controls::INITIAL
        },
        waveform: 1,
        inserts: [
            saturation(80, 128, 96, 8),
            Insert::Phaser(PhaserControls {
                rate: 10,
                depth: 80,
                feedback: 32,
                mix: 40,
                ..PhaserControls::INITIAL
            }),
        ],
    },
    Preset {
        name: "DEVIL DRIVE",
        echo_send: 48,
        echo_steps: StepPattern::from_bits(0x1111),
        slicer: SlicerControls {
            depth: 56,
            enabled: true,
        },
        slicer_steps: StepPattern::from_bits(0x3f3f),
        controls: Controls {
            // 445 Hz, 76% resonance, 32 semitones, 420 ms, 88% accent, 78% OD.
            values: [17, 76, 32, 42, 120, 5, 88, 28],
            ..Controls::INITIAL
        },
        waveform: 0,
        inserts: [saturation(48, 112, 18, 24), SIDECHAIN],
    },
];

/// Wrap an encoder movement around the bank, including batched negative turns.
pub(crate) fn turn(index: u8, detents: i8) -> u8 {
    let index = i16::from(index)
        .saturating_add(i16::from(detents))
        .rem_euclid(i16::try_from(PRESETS.len()).expect("small preset bank"));
    u8::try_from(index).expect("wrapped preset index")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bass::Knob;
    use crate::fx::InsertKind;

    #[test]
    fn first_presets_retain_both_factory_voices_and_restore_sidechain() {
        assert_eq!(PRESETS[0].controls, Controls::DEEP_HOUSE);
        assert_eq!(PRESETS[1].controls, Controls::HARMONY);
        assert_eq!(PRESETS[0].waveform, 0);
        assert_eq!(PRESETS[1].waveform, 1);
        assert_eq!(PRESETS[2].name, "PUMP PANEL");
        for preset in &PRESETS[..2] {
            assert_eq!(preset.inserts, Insert::factory(Part::Bass));
            assert_eq!(preset.inserts[1], SIDECHAIN);
        }
    }

    #[test]
    fn bank_controls_fit_the_panel_and_include_an_effect_type_change() {
        for preset in PRESETS {
            assert!(preset.waveform <= 1);
            for knob in Knob::PANEL {
                let (low, high) = knob.limits();
                assert!((low..=high).contains(&preset.controls.values[knob as usize]));
            }
            assert_eq!(preset.controls.midi, [None; 8]);
        }
        assert_eq!(PRESETS[5].inserts[1].kind(), InsertKind::Phaser);
        assert_eq!(PRESETS[2].controls.values[Knob::Overdrive as usize], 24);
    }

    #[test]
    fn preset_beat_fx_is_channel_local_and_resets_on_recall() {
        let mut fx = Fx::INITIAL;
        let mut mixer = Mixer::INITIAL;
        fx.flanger_target = Some(Channel::Two);
        let before_fx = fx;
        let before_mixer = mixer;
        PRESETS[6].apply_beat_fx(Channel::One, &mut fx, &mut mixer);
        assert_eq!(fx.slicers[0], PRESETS[6].slicer);
        assert_eq!(mixer.0[0].delay, PRESETS[6].echo_send);
        let mut expected_fx = before_fx;
        expected_fx.slicers[0] = PRESETS[6].slicer;
        let mut expected_mixer = before_mixer;
        expected_mixer.0[0].delay = PRESETS[6].echo_send;
        assert_eq!(fx, expected_fx);
        assert_eq!(mixer, expected_mixer);
        PRESETS[0].apply_beat_fx(Channel::One, &mut fx, &mut mixer);
        assert_eq!(fx, before_fx);
        assert_eq!(mixer, before_mixer);
    }

    #[test]
    fn preset_grids_follow_recall_and_keep_other_channels_untouched() {
        use crate::panel::Navigation;
        let mut nav = Navigation::INITIAL;
        let original = nav;
        nav.turn_algorithm(6);
        assert_eq!(nav.beat_steps[0], PRESETS[6].echo_steps);
        assert_eq!(nav.slicer_steps[0], PRESETS[6].slicer_steps);
        assert_eq!(nav.beat_steps[1..], original.beat_steps[1..]);
        assert_eq!(nav.slicer_steps[1..], original.slicer_steps[1..]);
        nav.turn_algorithm(1);
        assert_eq!(nav.beat_steps, original.beat_steps);
        assert_eq!(nav.slicer_steps, original.slicer_steps);
    }

    #[test]
    fn encoder_wraps_both_directions_and_extreme_deltas() {
        assert_eq!(turn(0, -1), 6);
        assert_eq!(turn(6, 1), 0);
        assert_eq!(turn(2, 0), 2);
        assert_eq!(turn(0, i8::MAX), 1);
        assert_eq!(turn(0, i8::MIN), 5);
        assert_eq!(turn(u8::MAX, i8::MAX), 4);
        for index in u8::MIN..=u8::MAX {
            for delta in i8::MIN..=i8::MAX {
                assert!(usize::from(turn(index, delta)) < PRESETS.len());
            }
        }
    }
}
