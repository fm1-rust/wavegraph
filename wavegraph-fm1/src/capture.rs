//! A triggered sweep, assembled from consecutive blocks accepted by the DAC.

pub(super) const WIDTH: usize = 240;
// 1,200 audio frames span 27.2 ms, or two cycles of the factory bass line's low D.
const BASS_SAMPLES_PER_COLUMN: u8 = 5;
const LAST_ROW: u8 = 191;
pub(super) const HEIGHT: usize = LAST_ROW as usize + 1;
const CENTER: u8 = LAST_ROW / 2 + 1;
pub(super) type Trace = [u8; WIDTH];
pub(super) const FLAT: Trace = [CENTER; WIDTH];

#[derive(Default)]
pub(super) struct ScopeUpdates {
    last_labels: Option<(u32, bool)>,
    gap: bool,
}

impl ScopeUpdates {
    #[cfg(target_os = "none")]
    pub(super) fn missed(&mut self) {
        self.gap = true;
    }

    /// A visible trace needs every contiguous block. Other pages only need
    /// settings and graph gain changes. Retry until the observer accepts one.
    pub(super) fn publish(
        &mut self,
        labels: (u32, bool),
        visible: bool,
        send: impl FnOnce(bool) -> bool,
    ) {
        if visible || self.last_labels != Some(labels) {
            let sent = send(self.gap);
            self.gap = !visible || !sent;
            if sent {
                self.last_labels = Some(labels);
            }
        }
    }
}

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
    column_sum: i32,
    column_len: u8,
    silence_reported: bool,
}

impl Capture {
    pub(super) const fn new() -> Self {
        Self {
            samples: [0; WIDTH],
            filled: 0,
            previous: None,
            holdoff: 0,
            column_sum: 0,
            column_len: 0,
            silence_reported: false,
        }
    }

    pub(super) fn reset(&mut self) {
        self.filled = 0;
        self.previous = None;
        self.holdoff = 0;
        self.column_sum = 0;
        self.column_len = 0;
        self.silence_reported = false;
    }

    /// An actual silent output also acquires, so stopping clears the last note.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The x coordinate resets at WIDTH; the holdoff decrements only when positive."
    )]
    pub(super) fn push(&mut self, samples: &[i32], scale: Scale) -> Option<Trace> {
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

    /// Trigger on a real oscillator wrap and average five filtered samples per column.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Five shifted Q23 samples fit in i32. Both counters reset at their bounds."
    )]
    pub(super) fn push_bass(&mut self, samples: &[i32], cycle: Option<usize>) -> Option<Trace> {
        let mut active = self.filled != 0 || self.column_len != 0;
        if !active && cycle.is_none() {
            if !self.silence_reported && !samples.is_empty() && samples.iter().all(|&s| s == 0) {
                self.silence_reported = true;
                return Some(FLAT);
            }
            return None;
        }

        let mut trace = None;
        for (frame, &sample) in samples.iter().enumerate() {
            if !active {
                if cycle != Some(frame) {
                    continue;
                }
                active = true;
                self.silence_reported = false;
            }
            self.column_sum += sample >> 7;
            self.column_len += 1;
            if self.column_len == BASS_SAMPLES_PER_COLUMN {
                self.samples[self.filled] = self.column_sum / i32::from(BASS_SAMPLES_PER_COLUMN);
                self.column_sum = 0;
                self.column_len = 0;
                self.filled += 1;
                if self.filled == WIDTH {
                    trace = Some(self.trace(Scale::Auto));
                    self.filled = 0;
                    // A low D can wrap again just two samples after this sweep.
                    // Keep the rest of the DMA block available to the next trigger.
                    active = false;
                }
            }
        }
        trace
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
    fn hidden_pages_retry_labels_and_resume_with_a_fresh_trace() {
        let mut updates = ScopeUpdates::default();
        let mut offer = |revision, gain, visible, room| {
            let mut offered = None;
            updates.publish((revision, gain), visible, |gap| {
                offered = Some(gap);
                room
            });
            offered
        };
        assert_eq!(offer(0, false, true, true), Some(false));
        assert_eq!(offer(1, false, false, false), Some(false)); // queue full on exit
        assert_eq!(offer(1, false, false, true), Some(true)); // retry
        assert_eq!(offer(1, false, false, true), None);
        assert_eq!(offer(1, true, false, true), Some(true)); // graph edit only
        assert_eq!(offer(2, true, true, true), Some(true)); // scope reopens
        assert_eq!(offer(2, true, true, true), Some(false));
        assert_eq!(offer(3, true, false, false), Some(false));
        assert_eq!(offer(4, true, true, true), Some(true)); // hidden page never delivered
    }

    #[test]
    fn triggers_across_blocks_and_rearms_after_a_gap() {
        assert_eq!(HEIGHT, 192);
        let mut scope = Capture::new();
        assert_eq!(scope.push(&[-4096; 128], Scale::Fixed), None);
        assert_eq!(scope.push(&[4096; 128], Scale::Fixed), None);
        scope.reset();
        assert_eq!(scope.push(&[4096; 128], Scale::Fixed), None);
        assert_eq!(scope.push(&[4096; 128], Scale::Fixed), None);
        assert_eq!(scope.push(&[-4096; 128], Scale::Fixed), None);
        assert_eq!(scope.push(&[524288; 128], Scale::Fixed), None);
        let trace = scope
            .push(&[-524288; 128], Scale::Fixed)
            .expect("contiguous sweep");
        assert_eq!(trace[..128], [12; 128]);
        assert_eq!(trace[128..], [180; WIDTH - 128]);
        assert_eq!(scope.push(&[0; 128], Scale::Fixed), None);
        assert_eq!(scope.push(&[0; 128], Scale::Fixed), Some(FLAT));
    }

    #[test]
    fn auto_range_preserves_shape_and_lets_quiet_tails_fade() {
        let sweep = |peak: i32| {
            let mut scope = Capture::new();
            scope.push(&[-1], Scale::Auto);
            let wave = core::array::from_fn::<_, WIDTH, _>(|x| [peak, peak / 2, -peak, 0][x % 4]);
            scope.push(&wave, Scale::Auto).expect("one complete sweep")
        };
        assert_eq!(sweep(1 << 17)[..4], [8, 52, 184, CENTER]);
        assert_eq!(sweep(1 << 22), sweep(1 << 17));
        assert_eq!(sweep(1 << 14)[..4], [85, 91, 107, CENTER]);
        assert_eq!(sweep(0), FLAT);
    }

    #[test]
    fn bass_averages_from_each_wrap_across_dma_blocks() {
        let mut scope = Capture::new();
        let expected: Trace = core::array::from_fn(|x| if x % 120 < 60 { 8 } else { 184 });
        let mut completed = Vec::new();
        for block in 0..21 {
            // The first wrap lands two frames before the end of the first block.
            let phase = |i: usize| (block * 128 + i + 600 - 126) % 600;
            let samples: [i32; 128] = core::array::from_fn(|i| {
                let p = phase(i);
                let ring = [3, -1, 2, -2, 3][p % 5] << 17;
                if p < 300 { ring } else { -ring }
            });
            let wrap = (0..128).find(|&i| phase(i) == 0);
            if let Some(trace) = scope.push_bass(&samples, wrap) {
                assert_eq!(trace, expected);
                completed.push(block);
            }
        }
        assert_eq!(completed, [10, 19]);

        // Drop a partial column at a queue gap and wait for a fresh wrap.
        scope.reset();
        assert_eq!(scope.push_bass(&[-1024, 1024], None), None);
        assert_eq!(scope.push_bass(&[0; 128], None), Some(FLAT));
        assert_eq!(scope.push_bass(&[0; 128], None), None);
    }
}
