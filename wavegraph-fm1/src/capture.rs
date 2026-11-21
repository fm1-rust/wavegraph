//! A triggered sweep, assembled from consecutive blocks accepted by the DAC.

pub(super) const WIDTH: usize = 240;
const LAST_ROW: u8 = 191;
pub(super) const HEIGHT: usize = LAST_ROW as usize + 1;
const CENTER: u8 = LAST_ROW / 2 + 1;
pub(super) type Trace = [u8; WIDTH];
pub(super) const FLAT: Trace = [CENTER; WIDTH];

pub(super) struct Capture {
    trace: Trace,
    filled: usize,
    previous: Option<i32>,
    holdoff: u8,
}

impl Capture {
    pub(super) const fn new() -> Self {
        Self {
            trace: FLAT,
            filled: 0,
            previous: None,
            holdoff: 0,
        }
    }

    /// An actual silent output also acquires, so stopping clears the last note.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The x coordinate resets at WIDTH. Shifted PCM fits in i32 after scaling and the y coordinate is clamped to 0..=191."
    )]
    pub(super) fn push(&mut self, samples: &[i32], silent: bool, gap: bool) -> Option<Trace> {
        if gap {
            self.filled = 0;
            self.holdoff = 0;
            self.previous = None;
        }
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
            // Q23 at the instrument's full level occupies about +/-84 rows.
            let offset = ((sample >> 7) * 21) >> 10;
            self.trace[self.filled] =
                (i32::from(CENTER) - offset).clamp(0, i32::from(LAST_ROW)) as u8;
            self.filled += 1;
            if self.filled == WIDTH {
                self.filled = 0;
                self.holdoff = 8;
                self.previous = samples.last().copied();
                return Some(self.trace);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_across_blocks_and_rearms_after_a_gap() {
        assert_eq!(HEIGHT, 192);
        let mut scope = Capture::new();
        assert_eq!(scope.push(&[-4096; 128], false, false), None);
        assert_eq!(scope.push(&[4096; 128], false, false), None);
        assert_eq!(scope.push(&[4096; 128], false, true), None);
        assert_eq!(scope.push(&[4096; 128], false, false), None);
        assert_eq!(scope.push(&[-4096; 128], false, false), None);
        assert_eq!(scope.push(&[524288; 128], false, false), None);
        let trace = scope
            .push(&[-524288; 128], false, false)
            .expect("contiguous sweep");
        assert_eq!(trace[..128], [12; 128]);
        assert_eq!(trace[128..], [180; WIDTH - 128]);
        assert_eq!(scope.push(&[0; 128], true, false), None);
        assert_eq!(scope.push(&[0; 128], true, false), Some(FLAT));
    }
}
