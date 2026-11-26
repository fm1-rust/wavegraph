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
        send: impl FnOnce(bool, bool) -> bool,
    ) {
        let changed = self.last_labels != Some(labels);
        if visible || changed {
            let sent = send(self.gap, changed);
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

// Two half-sized sample arrays fit in the space previously used for one i32 sweep.
// Q15 is ample for a 192-row trace. Triggering still observes the original Q23.
struct SweepBuffer {
    samples: [i16; WIDTH],
    filled: usize,
    sum: i32,
    count: u8,
    active: bool,
}

impl SweepBuffer {
    const fn new() -> Self {
        Self {
            samples: [0; WIDTH],
            filled: 0,
            sum: 0,
            count: 0,
            active: false,
        }
    }

    fn reset(&mut self) {
        self.filled = 0;
        self.sum = 0;
        self.count = 0;
        self.active = false;
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The sum of five shifted Q23 samples fits i32 and the column count resets at WIDTH."
    )]
    fn bass_sample(&mut self, sample: i32) -> Option<Trace> {
        if !self.active {
            return None;
        }
        self.sum += sample >> 7;
        self.count += 1;
        if self.count != BASS_SAMPLES_PER_COLUMN {
            return None;
        }
        let mean = self.sum / i32::from(BASS_SAMPLES_PER_COLUMN);
        self.sum = 0;
        self.count = 0;
        self.column(mean >> 1, Scale::Auto)
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The column count resets at WIDTH."
    )]
    fn column(&mut self, value: i32, scale: Scale) -> Option<Trace> {
        self.samples[self.filled] = i16::try_from(value).expect("Q23 tap reduced to Q15");
        self.filled += 1;
        if self.filled != WIDTH {
            return None;
        }
        self.filled = 0;
        self.active = false;
        Some(self.trace(scale))
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The peak bounds the normalized offset. Rows are clamped before conversion."
    )]
    fn trace(&self, scale: Scale) -> Trace {
        let (gain, shift) = match scale {
            Scale::Fixed => (21, 9),
            Scale::Auto => {
                let peak = self
                    .samples
                    .iter()
                    .map(|&s| i32::from(s).abs())
                    .fold(512, i32::max);
                ((88 << 16) / peak, 16)
            }
        };
        self.samples.map(|s| {
            let offset = (i32::from(s) * gain) >> shift;
            (i32::from(CENTER) - offset).clamp(0, i32::from(LAST_ROW)) as u8
        })
    }
}

pub(super) struct Capture {
    primary: SweepBuffer,
    overlap: SweepBuffer,
    previous: Option<i32>,
    silence_reported: bool,
}

impl Capture {
    pub(super) const fn new() -> Self {
        Self {
            primary: SweepBuffer::new(),
            overlap: SweepBuffer::new(),
            previous: None,
            silence_reported: false,
        }
    }

    pub(super) fn reset(&mut self) {
        self.primary.reset();
        self.overlap.reset();
        self.previous = None;
        self.silence_reported = false;
    }

    /// Rearm on the next rising crossing. A flat input publishes once, even when
    /// the DAC continues to accept hundreds of silent blocks per second.
    pub(super) fn push(&mut self, samples: &[i32], scale: Scale) -> Option<Trace> {
        if samples.is_empty() {
            return None;
        }
        let silent = samples.iter().all(|&s| s == 0);
        if silent && self.silence_reported {
            return None;
        }
        let mut onset = !silent && self.silence_reported;
        if !silent {
            self.silence_reported = false;
        }
        let mut latest = None;
        for &sample in samples {
            let previous = self.previous.replace(sample);
            if !self.primary.active {
                if !silent
                    && !(onset && sample != 0 || previous.is_some_and(|p| p < 0 && sample >= 0))
                {
                    continue;
                }
                self.primary.active = true;
                onset = false;
            }
            if let Some(trace) = self.primary.column(sample >> 8, scale) {
                self.silence_reported = silent && trace == FLAT;
                latest = Some(trace);
                if self.silence_reported {
                    break;
                }
            }
        }
        latest
    }

    /// Start a second wrap-locked sweep once the first is at least half full.
    /// Both retain the same 1,200 consecutive frames and the same slide time scale.
    pub(super) fn push_bass(&mut self, samples: &[i32], cycle: Option<usize>) -> Option<Trace> {
        if !self.primary.active && !self.overlap.active && cycle.is_none() {
            if !self.silence_reported && !samples.is_empty() && samples.iter().all(|&s| s == 0) {
                self.silence_reported = true;
                return Some(FLAT);
            }
            return None;
        }
        let mut latest = None;
        for (i, &sample) in samples.iter().enumerate() {
            if cycle == Some(i) {
                self.silence_reported = false;
                if !self.primary.active
                    && (!self.overlap.active || self.overlap.filled >= WIDTH / 2)
                {
                    self.primary.active = true;
                } else if !self.overlap.active && self.primary.filled >= WIDTH / 2 {
                    self.overlap.active = true;
                }
            }
            let a = self.primary.bass_sample(sample);
            let b = self.overlap.bass_sample(sample);
            if let Some(trace) = a.or(b) {
                latest = Some(trace);
            }
        }
        latest
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
            updates.publish((revision, gain), visible, |gap, changed| {
                offered = Some((gap, changed));
                room
            });
            offered
        };
        assert_eq!(offer(0, false, true, true), Some((false, true)));
        assert_eq!(offer(1, false, false, false), Some((false, true))); // queue full on exit
        assert_eq!(offer(1, false, false, true), Some((true, true))); // retry
        assert_eq!(offer(1, false, false, true), None);
        assert_eq!(offer(1, true, false, true), Some((true, true))); // graph edit only
        assert_eq!(offer(2, true, true, true), Some((true, true))); // scope reopens
        assert_eq!(offer(2, true, true, true), Some((false, false)));
        assert_eq!(offer(3, true, false, false), Some((false, true)));
        assert_eq!(offer(4, true, true, true), Some((true, true))); // hidden page never delivered
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
    fn silence_sleeps_until_the_first_attack_even_without_a_zero_crossing() {
        let mut scope = Capture::new();
        assert_eq!(scope.push(&[0; WIDTH], Scale::Fixed), Some(FLAT));
        assert_eq!(scope.push(&[0; WIDTH], Scale::Fixed), None);
        assert_eq!(scope.push(&[0, 0, 1 << 18], Scale::Fixed), None);
        let trace = scope
            .push(&[1 << 18; WIDTH - 1], Scale::Fixed)
            .expect("complete attack");
        assert_ne!(trace, FLAT);
        assert!(trace.iter().all(|y| *y == trace[0]));
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
        assert_eq!(completed, [10, 15, 19]);

        // Drop a partial column at a queue gap and wait for a fresh wrap.
        scope.reset();
        assert_eq!(scope.push_bass(&[-1024, 1024], None), None);
        assert_eq!(scope.push_bass(&[0; 128], None), Some(FLAT));
        assert_eq!(scope.push_bass(&[0; 128], None), None);
    }
}
