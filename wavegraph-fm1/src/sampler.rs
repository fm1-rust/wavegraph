//! The sampler remembers a separate instrument program for each recording on a strip.

use crate::{
    envelope::EnvelopeTarget,
    instrument::{CUTOFF_HZ, MAX_CUTOFF_STEP},
};
use wavegraph_adsr::Shape;
use wavegraph_sampler::{FULL_SLICES, Playback, PlaybackMode, SliceTrims, Timbre};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Pads {
    pub(super) trims: SliceTrims,
    // None is the complete four-bar loop on C4.
    pub(super) selected: Option<u8>,
}

impl Pads {
    pub(super) const INITIAL: Self = Self {
        trims: FULL_SLICES,
        selected: Some(0),
    };

    pub(super) fn select(&mut self, note: u8) {
        if note == 60 {
            self.selected = None;
        } else if let Some(index) = crate::assets::PAD_NOTES.iter().position(|&n| n == note) {
            self.selected = Some(u8::try_from(index).expect("eight pads"));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Program {
    pub(super) playback: Playback,
    pub(super) timbre: Timbre,
}

impl Program {
    pub(super) const DRY: Self = Self {
        playback: Playback::INITIAL,
        timbre: Timbre::DRY,
    };

    pub(super) const BANK: [Self; crate::assets::SAMPLE_NAMES.len()] = [
        // A single step can launch all four bars with the recorded drum transients intact.
        Self {
            playback: Playback {
                mode: PlaybackMode::OneShot,
                ..Playback::INITIAL
            },
            ..Self::DRY
        },
        // The chord's layers and delay are printed. Let the release ring after key-up.
        Self {
            playback: Playback {
                release_ms: 95,
                ..Playback::INITIAL
            },
            ..Self::DRY
        },
        Self {
            timbre: Timbre {
                cutoff_hz: 4454,
                env_semitones: 12,
                filter: Shape::new(320_000, 160_000, 90, 125_000),
                ..Timbre::DRY
            },
            ..Self::DRY
        },
        // Leave the horn and zap wide open at full velocity, and darken softer hits.
        Self {
            timbre: Timbre {
                velocity_semitones: 24,
                filter: Shape::new(0, 160_000, 35, 80_000),
                ..Timbre::DRY
            },
            ..Self::DRY
        },
        Self {
            timbre: Timbre {
                velocity_semitones: 36,
                filter: Shape::new(0, 80_000, 0, 40_000),
                ..Timbre::DRY
            },
            ..Self::DRY
        },
        Self {
            timbre: Timbre {
                cutoff_hz: 1575,
                resonance: 28,
                env_semitones: 23,
                velocity_semitones: 12,
                key_tracking: 75,
                amp: Shape::new(0, 250_000, 78, 100_000),
                filter: Shape::new(0, 200_000, 18, 80_000),
            },
            ..Self::DRY
        },
        Self::DRY, // Clipboard: each strip can shape the same recording independently.
    ];

    pub(super) fn cutoff_step(self) -> u8 {
        u8::try_from(CUTOFF_HZ.partition_point(|&hz| hz < self.timbre.cutoff_hz))
            .expect("short cutoff table")
            .min(MAX_CUTOFF_STEP)
    }

    #[inline(never)]
    pub(super) fn turn(&mut self, [cutoff, q, env, velocity]: [i8; 4]) {
        if cutoff != 0 {
            let step = self
                .cutoff_step()
                .saturating_add_signed(cutoff)
                .min(MAX_CUTOFF_STEP);
            self.timbre.cutoff_hz = if step == MAX_CUTOFF_STEP {
                Timbre::OPEN
            } else {
                CUTOFF_HZ[usize::from(step)]
            };
        }
        self.timbre.resonance = self.timbre.resonance.saturating_add_signed(q).min(127);
        self.timbre.env_semitones = self.timbre.env_semitones.saturating_add(env).clamp(-48, 48);
        self.timbre.velocity_semitones = self
            .timbre
            .velocity_semitones
            .saturating_add_signed(velocity)
            .min(48);
    }

    pub(super) fn shape(&mut self, target: EnvelopeTarget) -> &mut Shape {
        if target == EnvelopeTarget::Amp {
            &mut self.timbre.amp
        } else {
            &mut self.timbre.filter
        }
    }

    #[inline(never)]
    pub(super) fn edit(&mut self, [mode, start, end, tracking]: [i8; 4], pads: Option<&mut Pads>) {
        let p = &mut self.playback;
        let modes = [
            PlaybackMode::OneShot,
            PlaybackMode::Gate,
            PlaybackMode::Loop,
            PlaybackMode::Slice,
        ];
        let max = if pads.is_some() { 3 } else { 2 };
        p.mode = modes[usize::from((p.mode as u8).saturating_add_signed(mode).min(max))];
        let bounds = if p.mode == PlaybackMode::Slice {
            pads.and_then(|pads| {
                let i = usize::from(pads.selected?);
                let [first, last] = &mut pads.trims[i];
                Some((first, last))
            })
        } else {
            Some((&mut p.start, &mut p.end))
        };
        if let Some((first, last)) = bounds {
            *first = first
                .saturating_add_signed(start)
                .min(last.saturating_sub(1));
            *last = last
                .saturating_add_signed(end)
                .clamp(first.saturating_add(1), 100);
        }
        self.timbre.key_tracking = self
            .timbre
            .key_tracking
            .saturating_add_signed(tracking)
            .min(100);
    }
}
