//! FM-1 encoder mappings and the VCA envelope preview.

use crate::capture::{Trace, WIDTH};
use wavegraph_adsr::Adsr;

// Approximately three detents per octave. The SH-101 attack has the shorter range.
const TIME_US: [u32; 39] = [
    0, 1_500, 2_000, 3_000, 4_000, 5_000, 6_000, 8_000, 10_000, 12_000, 16_000, 20_000, 25_000,
    32_000, 40_000, 50_000, 64_000, 80_000, 100_000, 125_000, 160_000, 200_000, 250_000, 320_000,
    400_000, 500_000, 640_000, 800_000, 1_000_000, 1_250_000, 1_600_000, 2_000_000, 2_500_000,
    3_200_000, 4_000_000, 5_000_000, 6_400_000, 8_000_000, 10_000_000,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EnvelopeControls(Adsr);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EnvelopeTarget {
    Amp,
    Filter,
}

fn turn_time(us: u32, delta: i8, steps: &[u32]) -> u32 {
    if delta == 0 {
        return us;
    }
    let index = steps.partition_point(|&time| time < us);
    let next = index.saturating_add_signed(isize::from(delta));
    steps[next.min(steps.len().saturating_sub(1))]
}

impl EnvelopeControls {
    pub(super) const INITIAL: Self = Self(Adsr {
        attack_us: 8_000,
        decay_us: 160_000,
        sustain_percent: 70,
        release_us: 125_000,
    });

    pub(super) const WAVETABLE: Self = Self(wavegraph_wavetable::DEFAULT_ENVELOPE);
    pub(super) const SUPER_SAW_AMP: Self = Self(wavegraph_supersaw::DEFAULT_AMP_ENVELOPE);
    pub(super) const SUPER_SAW_FILTER: Self = Self(wavegraph_supersaw::DEFAULT_FILTER_ENVELOPE);

    pub(super) const ORGAN: Self = Self(wavegraph_organ::DEFAULT_ENVELOPE);

    pub(super) const HOUSE_CHORDS: Self = Self(Adsr {
        attack_us: 4_000,
        decay_us: 180_000,
        sustain_percent: 92,
        release_us: 200_000,
    });

    fn turn_shape(&mut self, [a, d, s]: [i8; 3], attack: &[u32], decay: &[u32]) {
        self.0.attack_us = turn_time(self.0.attack_us, a, attack);
        self.0.decay_us = turn_time(self.0.decay_us, d, decay);
        self.0.sustain_percent = self.0.sustain_percent.saturating_add_signed(s).min(100);
    }

    pub(super) fn turn(&mut self, [a, d, s, r]: [i8; 4]) {
        // SH-101 panel ranges: attack starts at 1.5 ms; decay and release at 2 ms.
        self.turn_shape([a, d, s], &TIME_US[1..=34], &TIME_US[2..]);
        self.0.release_us = turn_time(self.0.release_us, r, &TIME_US[2..]);
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "An i8 delta times 5000 fits i32."
    )]
    pub(super) fn turn_organ(&mut self, [a, d, s, r]: [i8; 4]) {
        self.turn_shape([a, d, s], &TIME_US[..=34], &TIME_US);
        // The organ scope and ENV page both move release in 5 ms steps.
        self.0.release_us = self
            .0
            .release_us
            .saturating_add_signed(i32::from(r) * 5_000)
            .min(1_000_000);
    }

    pub(super) const fn adsr(self) -> Adsr {
        self.0
    }

    /// Scale segment widths by their time range so millisecond attacks remain
    /// visible alongside multi-second tails. The plateau represents a held key.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "Only nonempty segments are interpolated. The curve fits in 224 by 132 pixels."
    )]
    pub(super) fn trace(self) -> Trace {
        const START: usize = 8;
        const END: usize = WIDTH - START;
        const BASE: usize = 172;
        const HEIGHT: usize = 132;
        let width = |us| {
            if us == 0 {
                0
            } else {
                8 + TIME_US[1..].partition_point(|&time| time < us)
            }
        };
        let a = width(self.0.attack_us);
        let d = width(self.0.decay_us);
        let r = width(self.0.release_us);
        let hold = 20;
        let total = a + d + hold + r;
        let x_at = |units| START + (END - START) * units / total;
        let peak = x_at(a);
        let held = x_at(a + d);
        let off = x_at(a + d + hold);
        let sustain = HEIGHT * usize::from(self.0.sustain_percent) / 100;
        core::array::from_fn(|x| {
            let height = if !(START..END).contains(&x) {
                0
            } else if x < peak {
                HEIGHT * (x - START) / (peak - START)
            } else if x < held {
                sustain + (HEIGHT - sustain) * (held - x).pow(2) / (held - peak).pow(2)
            } else if x < off {
                sustain
            } else {
                sustain * (END - x).pow(2) / (END - off).pow(2)
            };
            (BASE - height) as u8
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_knobs_redraw_the_preview_and_stop_at_the_sh101_ranges() {
        let mut controls = EnvelopeControls::INITIAL;
        let original = controls.trace();
        for knob in 0..4 {
            let mut changed = controls;
            let mut detents = [0; 4];
            detents[knob] = 3;
            changed.turn(detents);
            assert_ne!(changed.trace(), original);
        }
        for (delta, attack, decay, sustain, release) in [
            (i8::MIN, 1_500, 2_000, 0, 2_000),
            (i8::MAX, 4_000_000, 10_000_000, 100, 10_000_000),
        ] {
            controls.turn([delta; 4]);
            assert_eq!(
                controls.adsr(),
                Adsr {
                    attack_us: attack,
                    decay_us: decay,
                    sustain_percent: sustain,
                    release_us: release,
                }
            );
            let trace = controls.trace();
            assert_eq!(trace.iter().min(), Some(&40));
            assert_eq!((trace[0], trace[WIDTH - 1]), (172, 172));
        }
    }
}
