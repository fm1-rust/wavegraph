//! Bass-preserving soft clipping at the output of the VCA.

use crate::dsp::tau_to_coef;

/// Positions of the Devil Fish-inspired Muffler switch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Muffler {
    /// Pass the original VCA output.
    #[default]
    Off,
    /// Round the strongest peaks with a broad knee.
    One,
    /// Catch more of each accent with a lower, narrower knee.
    Two,
}

const RAMP: u8 = 64;

#[derive(Debug)]
pub(crate) struct Stage {
    mode: Muffler,
    position: f32,
    step: f32,
    remaining: u8,
    low: f32,
    coefficient: f32,
}

impl Stage {
    pub(crate) fn new(rate: f32) -> Self {
        Self {
            mode: Muffler::Off,
            position: 0.0,
            step: 0.0,
            remaining: 0,
            low: 0.0,
            coefficient: tau_to_coef(1.0 / (core::f32::consts::TAU * 145.0), rate),
        }
    }

    pub(crate) fn select(&mut self, mode: Muffler) {
        if mode != self.mode {
            self.mode = mode;
            self.step = (self.target() - self.position) / f32::from(RAMP);
            self.remaining = RAMP;
        }
    }

    fn target(&self) -> f32 {
        match self.mode {
            Muffler::Off => 0.0,
            Muffler::One => 1.0,
            Muffler::Two => 2.0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.low = 0.0;
        self.position = self.target();
        self.remaining = 0;
    }

    pub(crate) fn bypassed(&self) -> bool {
        self.mode == Muffler::Off && self.remaining == 0
    }

    // The crossover still discharges on a sleeping voice. Once below the Q23
    // floor it can be cleared, and a silent control ramp can advance in bulk.
    pub(crate) fn skip_silence(&mut self, frames: usize) -> bool {
        if self.low.abs() > 1e-9 {
            return false;
        }
        self.low = 0.0;
        if frames >= usize::from(self.remaining) {
            self.reset();
        } else {
            for _ in 0..frames {
                self.remaining = self.remaining.saturating_sub(1);
                self.position += self.step;
            }
        }
        true
    }

    #[inline]
    pub(crate) fn sample(&mut self, dry: f32) -> f32 {
        if self.bypassed() {
            return dry;
        }
        let high = dry - self.low;
        self.low += self.coefficient * high;
        if self.remaining != 0 {
            return self.transition(dry, high);
        }
        match self.mode {
            Muffler::Off => dry,
            Muffler::One => dry - rounded_peaks(high),
            Muffler::Two => dry - firm_peaks(high),
        }
    }

    // Interpolate only the removed peaks. A second switch during the ramp
    // starts from the current mix, including when crossing the middle position.
    #[inline(never)]
    fn transition(&mut self, dry: f32, high: f32) -> f32 {
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            self.position = self.target();
            if self.mode == Muffler::Off {
                self.low = 0.0;
                return dry;
            }
        } else {
            self.position += self.step;
        }
        let one = rounded_peaks(high);
        let removed = if self.position <= 1.0 {
            self.position * one
        } else {
            one + (self.position - 1.0) * (firm_peaks(high) - one)
        };
        dry - removed
    }
}

// Native VCA levels, before output trim. Both curves have unity gain below
// the knee and join their flat rail with a continuous first derivative.
fn rounded_peaks(high: f32) -> f32 {
    removed(high, 0.125, 0.1875, 0.5 / 0.1875)
}

fn firm_peaks(high: f32) -> f32 {
    removed(high, 0.0625, 0.0625, 8.0)
}

fn removed(high: f32, threshold: f32, width: f32, curvature: f32) -> f32 {
    let over = high.abs() - threshold;
    if over <= 0.0 {
        return 0.0;
    }
    let delta = if over < width {
        over * over * curvature
    } else {
        over - 0.5 * width
    };
    delta.copysign(high)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bass_and_quiet_signals_pass_both_modes_while_loud_treble_hits_distinct_rails() {
        for (mode, rail) in [(Muffler::One, 0.21875), (Muffler::Two, 0.09375)] {
            let mut stage = Stage::new(44_118.0);
            stage.select(mode);
            stage.reset();
            for frame in 0..8192_u16 {
                let phase = f32::from(frame) * (core::f32::consts::TAU * 45.0 / 44_118.0);
                let sample = 0.15 * micromath::F32Ext::sin(phase);
                assert_eq!(stage.sample(sample), sample);
            }
            stage.reset();
            for sample in [-0.01, 0.01, -0.02, 0.02] {
                assert_eq!(stage.sample(sample), sample);
            }
            stage.reset();
            assert_eq!(stage.sample(0.5), rail);
            stage.reset();
            assert_eq!(stage.sample(-0.5), -rail);
        }
    }
}
