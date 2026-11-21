//! FM-1 encoder positions and the corresponding VCA envelope preview.

use crate::capture::{Trace, WIDTH};
use wavegraph_monosynth::Adsr;

// Approximately three detents per octave. The SH-101 attack has the shorter range.
const TIME_US: [u32; 38] = [
    1_500, 2_000, 3_000, 4_000, 5_000, 6_000, 8_000, 10_000, 12_000, 16_000, 20_000, 25_000,
    32_000, 40_000, 50_000, 64_000, 80_000, 100_000, 125_000, 160_000, 200_000, 250_000, 320_000,
    400_000, 500_000, 640_000, 800_000, 1_000_000, 1_250_000, 1_600_000, 2_000_000, 2_500_000,
    3_200_000, 4_000_000, 5_000_000, 6_400_000, 8_000_000, 10_000_000,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EnvelopeControls {
    attack: u8,
    decay: u8,
    sustain: u8,
    release: u8,
}

impl EnvelopeControls {
    pub(super) const INITIAL: Self = Self {
        attack: 6, // 8 ms
        decay: 19, // 160 ms
        sustain: 70,
        release: 18, // 125 ms
    };

    pub(super) fn turn(&mut self, [a, d, s, r]: [i8; 4]) {
        self.attack = self.attack.saturating_add_signed(a).min(33);
        self.decay = self.decay.saturating_add_signed(d).clamp(1, 37);
        self.sustain = self.sustain.saturating_add_signed(s).min(100);
        self.release = self.release.saturating_add_signed(r).clamp(1, 37);
    }

    pub(super) fn adsr(self) -> Adsr {
        Adsr {
            attack_us: TIME_US[usize::from(self.attack)],
            decay_us: TIME_US[usize::from(self.decay)],
            sustain_percent: self.sustain,
            release_us: TIME_US[usize::from(self.release)],
        }
    }

    /// Scale segment widths by encoder position so millisecond attacks remain
    /// visible alongside multi-second tails. The plateau represents a held key.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "The positive stage widths divide 224 pixels; all heights fit in the 132-pixel plot."
    )]
    pub(super) fn trace(self) -> Trace {
        const START: usize = 8;
        const END: usize = WIDTH - START;
        const BASE: usize = 172;
        const HEIGHT: usize = 132;
        let a = 8 + usize::from(self.attack);
        let d = 8 + usize::from(self.decay);
        let r = 8 + usize::from(self.release);
        let hold = 20;
        let total = a + d + hold + r;
        let x_at = |units| START + (END - START) * units / total;
        let peak = x_at(a);
        let held = x_at(a + d);
        let off = x_at(a + d + hold);
        let sustain = HEIGHT * usize::from(self.sustain) / 100;
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
