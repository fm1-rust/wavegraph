use super::Message;
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::I24;

/// Mono digital delay with a wet-only Q23 output and a caller-owned PCM16 line.
/// A time change crossfades two taps over 64 frames. Faster edits are coalesced
/// into the next fade. Changing the wet level or muting never clears the line.
#[derive(Debug)]
pub struct Delay<'a> {
    line: &'a mut [i16],
    rate_hz: u32,
    write: usize,
    // Upper bound on the number of zero writes needed to empty the ring.
    tail: usize,
    time: usize,
    target: usize,
    fade_to: usize,
    fade_frame: u8,
    feedback: u8,
    wet: u8,
    enabled: bool,
}

impl<'a> Delay<'a> {
    /// Initial time in milliseconds, subject to the available storage.
    pub const DEFAULT_TIME_MS: u16 = 375;
    /// Initial feedback, as a percentage of the previous echo.
    pub const DEFAULT_FEEDBACK: u8 = 35;
    /// The feedback cap keeps repeats decaying at every setting.
    pub const MAX_FEEDBACK: u8 = 95;

    /// Clear the line and enable the return at half level. Each sample of
    /// storage supports one frame of delay. Returns `None` for an empty line
    /// or a zero sample rate.
    #[must_use]
    pub fn new(line: &'a mut [i16], rate_hz: u32) -> Option<Self> {
        if line.is_empty() || rate_hz == 0 {
            return None;
        }
        line.fill(0);
        let mut delay = Self {
            line,
            rate_hz,
            write: 0,
            tail: 0,
            time: 0,
            target: 0,
            fade_to: 0,
            fade_frame: 0,
            feedback: Self::DEFAULT_FEEDBACK,
            wet: 64,
            enabled: true,
        };
        delay.control(Message::TimeMs(Self::DEFAULT_TIME_MS));
        Some(delay)
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "u32 Hz times u16 ms fits u64"
    )]
    fn control(&mut self, message: Message) {
        match message {
            Message::TimeMs(ms) => {
                let frames = (u64::from(self.rate_hz) * u64::from(ms) + 500) / 1000;
                self.target = usize::try_from(frames)
                    .unwrap_or(usize::MAX)
                    .clamp(1, self.line.len());
                if self.tail == 0 {
                    self.time = self.target;
                    self.fade_frame = 0;
                }
            }
            Message::Feedback(percent) => self.feedback = percent.min(Self::MAX_FEEDBACK),
            Message::Wet(level) => self.wet = level.min(128),
            Message::Enabled(enabled) => self.enabled = enabled,
            _ => {}
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Both indices are bounded by the line length"
    )]
    fn tap(&self, frames: usize) -> i32 {
        let index = if self.write >= frames {
            self.write - frames
        } else {
            self.line.len() - (frames - self.write)
        };
        i32::from(self.line[index])
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "PCM16 tap differences, gains and sums fit i32. Clamp before narrowing."
    )]
    fn sample(&mut self, input: I24) -> I24 {
        const FADE: u8 = 64;
        let mut echo = self.tap(self.time);
        if self.fade_frame != 0 || self.time != self.target {
            if self.fade_frame == 0 {
                self.fade_to = self.target;
            }
            self.fade_frame += 1;
            echo += (self.tap(self.fade_to) - echo) * i32::from(self.fade_frame) / i32::from(FADE);
            if self.fade_frame == FADE {
                self.time = self.fade_to;
                self.fade_frame = 0;
            }
        }

        // Truncate toward zero so even the quietest negative repeats decay to silence.
        let sample = input.inner() / 256 + echo * i32::from(self.feedback) / 100;
        self.line[self.write] = sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        if sample != 0 {
            self.tail = self.line.len();
        } else {
            self.tail = self.tail.saturating_sub(1);
            if self.tail == 0 {
                self.time = self.target;
                self.fade_frame = 0;
            }
        }
        self.write += 1;
        if self.write == self.line.len() {
            self.write = 0;
        }
        if self.enabled {
            // PCM16 to Q23 is x256, and the return gain is wet/128.
            I24::new_unchecked(echo * 2 * i32::from(self.wet))
        } else {
            I24::default()
        }
    }

    fn render(&mut self, input: &[I24], output: &mut [I24]) {
        if self.tail == 0 && input.iter().all(|s| s.inner() == 0) {
            output.fill(I24::default());
        } else {
            for (out, &sample) in output.iter_mut().zip(input) {
                *out = self.sample(sample);
            }
        }
    }
}

impl Process<Message, I24> for Delay<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        if cx.frames == 0 {
            for event in cx.events {
                self.control(event.message);
            }
            return;
        }
        let input = cx.inputs[0];
        let output = &mut cx.outputs[0];
        let mut start = 0;
        for event in cx.events {
            self.render(&input[start..event.frame], &mut output[start..event.frame]);
            self.control(event.message);
            start = event.frame;
        }
        self.render(&input[start..cx.frames], &mut output[start..cx.frames]);
    }
}
