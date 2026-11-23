//! A triggered sweep, assembled from consecutive blocks accepted by the DAC.

pub(super) const WIDTH: usize = 240;
const LAST_ROW: u8 = 191;
pub(super) const HEIGHT: usize = LAST_ROW as usize + 1;
const CENTER: u8 = LAST_ROW / 2 + 1;
pub(super) type Trace = [u8; WIDTH];
pub(super) const FLAT: Trace = [CENTER; WIDTH];

#[derive(Clone, Copy)]
pub(super) enum Scale {
    Fixed,
    Auto,
}

pub(super) struct Capture {
    samples: [i32; WIDTH],
    filled: usize,
    previous: Option<i32>,
    holdoff: u8,
}

impl Capture {
    pub(super) const fn new() -> Self {
        Self {
            samples: [0; WIDTH],
            filled: 0,
            previous: None,
            holdoff: 0,
        }
    }

    /// An actual silent output also acquires, so stopping clears the last note.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The x coordinate resets at WIDTH; the holdoff decrements only when positive."
    )]
    pub(super) fn push(&mut self, samples: &[i32], gap: bool, scale: Scale) -> Option<Trace> {
        if gap {
            self.filled = 0;
            self.holdoff = 0;
            self.previous = None;
        }
        let silent = samples.iter().all(|&sample| sample == 0);
        if self.holdoff > 0 && !silent {
            self.holdoff -= 1;
            self.previous = samples.last().copied();
            return None;
        }
        for &sample in samples {
            let previous = self.previous.replace(sample);
            if self.filled == 0 && !silent && !previous.is_some_and(|p| p < 0 && sample >= 0) {
                continue;
            }
            self.samples[self.filled] = sample >> 7;
            self.filled += 1;
            if self.filled == WIDTH {
                self.filled = 0;
                self.holdoff = 8;
                self.previous = samples.last().copied();
                return Some(self.trace(scale));
            }
        }
        None
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Fixed gain fits shifted PCM in i32. Auto gain times the measured peak is at most 88 << 16. Rows are clamped before conversion."
    )]
    fn trace(&self, scale: Scale) -> Trace {
        let (gain, shift) = match scale {
            Scale::Fixed => (21, 10),
            Scale::Auto => {
                // At most 88 capture rows either side of zero. Below -36 dBFS
                // freeze the gain so a decaying note still shrinks to silence.
                let peak = self.samples.iter().fold(1024, |p, &s| p.max(s.abs()));
                ((88 << 16) / peak, 16)
            }
        };
        core::array::from_fn(|x| {
            let offset = (self.samples[x] * gain) >> shift;
            (i32::from(CENTER) - offset).clamp(0, i32::from(LAST_ROW)) as u8
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_across_blocks_and_rearms_after_a_gap() {
        assert_eq!(HEIGHT, 192);
        let mut scope = Capture::new();
        assert_eq!(scope.push(&[-4096; 128], false, Scale::Fixed), None);
        assert_eq!(scope.push(&[4096; 128], false, Scale::Fixed), None);
        assert_eq!(scope.push(&[4096; 128], true, Scale::Fixed), None);
        assert_eq!(scope.push(&[4096; 128], false, Scale::Fixed), None);
        assert_eq!(scope.push(&[-4096; 128], false, Scale::Fixed), None);
        assert_eq!(scope.push(&[524288; 128], false, Scale::Fixed), None);
        let trace = scope
            .push(&[-524288; 128], false, Scale::Fixed)
            .expect("contiguous sweep");
        assert_eq!(trace[..128], [12; 128]);
        assert_eq!(trace[128..], [180; WIDTH - 128]);
        assert_eq!(scope.push(&[0; 128], false, Scale::Fixed), None);
        assert_eq!(scope.push(&[0; 128], false, Scale::Fixed), Some(FLAT));
    }

    #[test]
    fn auto_range_preserves_shape_and_lets_quiet_tails_fade() {
        let sweep = |peak: i32| {
            let mut scope = Capture::new();
            scope.push(&[-1], false, Scale::Auto);
            let wave = core::array::from_fn::<_, WIDTH, _>(|x| [peak, peak / 2, -peak, 0][x % 4]);
            scope
                .push(&wave, false, Scale::Auto)
                .expect("one complete sweep")
        };
        assert_eq!(sweep(1 << 17)[..4], [8, 52, 184, CENTER]);
        assert_eq!(sweep(1 << 22), sweep(1 << 17));
        assert_eq!(sweep(1 << 14)[..4], [85, 91, 107, CENTER]);
        assert_eq!(sweep(0), FLAT);
    }
}
