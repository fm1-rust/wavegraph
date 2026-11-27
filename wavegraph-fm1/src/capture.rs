//! A triggered sweep, assembled from consecutive blocks accepted by the DAC.

pub(super) const WIDTH: usize = 240;
// 1,200 audio frames span 27.2 ms, or two cycles of the factory bass line's low D.
const BASS_SAMPLES_PER_COLUMN: u8 = 5;
const LAST_ROW: u8 = 191;
pub(super) const HEIGHT: usize = LAST_ROW as usize + 1;
const CENTER: u8 = LAST_ROW / 2 + 1;
pub(super) type Trace = [u8; WIDTH];
pub(super) const FLAT: Trace = [CENTER; WIDTH];
pub(super) const SLICER_SILENCE: Trace = [LAST_ROW; WIDTH];

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

    /// A rolling, unipolar peak envelope. The end phase is the position *after*
    /// the final DAC frame. Every 15 columns cover one of the 16 Slicer steps.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The phase is modular Q0.32, WIDTH bounds column arithmetic and row height is clamped."
    )]
    pub(super) fn push_slicer(
        &mut self,
        samples: &[i32],
        end_phase: u32,
        increment: u32,
    ) -> Option<Trace> {
        if samples.is_empty() {
            return None;
        }
        // Filled is zero after reset. Otherwise it records the last bin plus
        // one. A new cycle replaces each bin as its first frame actually plays.
        let sweep = &mut self.primary;
        let mut publish = sweep.filled == 0;
        if publish {
            sweep.samples.fill(0);
        }
        let mut phase = end_phase.wrapping_sub(increment.wrapping_mul(samples.len() as u32));
        for &sample in samples {
            let column = ((u64::from(phase) * WIDTH as u64) >> 32) as usize;
            if sweep.filled != column + 1 {
                sweep.samples[column] = 0;
                sweep.filled = column + 1;
                publish |= column.is_multiple_of(WIDTH / 16);
            }
            let peak = (sample.unsigned_abs() >> 8).min(i16::MAX as u32) as i16;
            sweep.samples[column] = sweep.samples[column].max(peak);
            phase = phase.wrapping_add(increment);
        }
        publish.then(|| {
            sweep.samples.map(|peak| {
                // Same fixed channel gain as the bipolar sweep, across its full height.
                let height = ((i32::from(peak) * 42) >> 9).min(i32::from(LAST_ROW));
                (i32::from(LAST_ROW) - height) as u8
            })
        })
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
    fn slicer_peaks_follow_the_played_steps_across_blocks_and_replace_old_audio() {
        let mut scope = Capture::new();
        let increment = 1 << 24; // 256 audio frames per cycle, 16 per step.
        let frames: [i32; 256] = core::array::from_fn(|i| match i / 16 % 4 {
            0 => 0,
            1 => -(1 << 17),
            2 => 1 << 18,
            _ => 1 << 19,
        });
        let mut end = 0_u32;
        let mut latest = None;
        for block in [
            &frames[..5],
            &frames[5..127],
            &frames[127..223],
            &frames[223..],
        ] {
            end =
                end.wrapping_add(increment * u32::try_from(block.len()).expect("short test block"));
            latest = scope.push_slicer(block, end, increment).or(latest);
        }
        let trace = latest.expect("full cycle");
        for (step, bins) in trace.as_chunks::<15>().0.iter().enumerate() {
            assert_eq!(*bins, [[191, 149, 107, 23][step % 4]; 15]);
        }
        assert_eq!(
            scope.push_slicer(&[0; 256], 0, increment),
            Some(SLICER_SILENCE)
        );
    }

    #[test]
    fn slicer_keeps_peak_within_one_bin_and_clears_missing_history_after_reset() {
        let mut scope = Capture::new();
        let increment = 1 << 20;
        // Start precisely on step four. Only two frames are observed there.
        let start = 4 << 28;
        let end = start + 2 * increment;
        let trace = scope
            .push_slicer(&[-(1 << 19), 0], end, increment)
            .expect("first observed bin");
        assert_eq!(trace[60], 23);
        assert!(
            trace[..60]
                .iter()
                .chain(&trace[61..])
                .all(|&y| y == LAST_ROW)
        );
        assert_eq!(scope.push_slicer(&[0], end + increment, increment), None);
        // A new bank/channel, jump in the clock, or missed DMA starts fresh at
        // its current phase. Unobserved steps stay at the silent baseline.
        scope.reset();
        let start = 11 << 28;
        let trace = scope
            .push_slicer(&[1 << 18], start + increment, increment)
            .expect("new timeline after a reset");
        assert_eq!(trace[165], 107);
        assert_eq!(trace[60], LAST_ROW);
        assert_eq!(scope.push_slicer(&[], start + increment, increment), None);
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
