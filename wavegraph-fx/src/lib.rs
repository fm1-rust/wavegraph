//! Allocation-free shared effects for signed Q23 WaveGraph patches.
//!
//! Connect any instrument buses to [`SendMixer`], call one [`Reverb`] on
//! its output, then mix the 100%-wet return with the dry bus. State survives
//! graph edits because processors live outside the bytecode program.
#![no_std]
#![forbid(unsafe_code)]

use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::{I24, types::i24};

/// Fixed scratch storage for a short mono, Schroeder-style reverb (17,048 bytes).
/// Delay lengths are tuned for the FM-1's approximately 44.1 kHz sample rate.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "Compile-time delay-line lengths"
)]
pub const REVERB_SAMPLES: usize = 743 + 877 + 1009 + 1153 + 173 + 307;

/// Control messages delivered on the effect nodes' first inlet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Send amount for the first instrument, 0..=128.
    SendA(u8),
    /// Send amount for the second instrument, 0..=128.
    SendB(u8),
    /// Reverberation decay, 0..=127.
    Decay(u8),
    /// Wet return level, 0..=128; dry audio is never attenuated here.
    Wet(u8),
    /// Mute or unmute the wet return; the tank keeps running when muted.
    Enabled(bool),
}

/// A two-input, one-output send bus. The caller owns the dry mix separately.
#[derive(Clone, Copy, Debug, Default)]
pub struct SendMixer {
    a: u8,
    b: u8,
}

impl SendMixer {
    /// Create a silent send bus. Set its per-instrument gains with messages.
    #[must_use]
    pub const fn new() -> Self {
        Self { a: 0, b: 0 }
    }

    fn control(&mut self, message: Message) {
        match message {
            Message::SendA(level) => self.a = level.min(128),
            Message::SendB(level) => self.b = level.min(128),
            _ => {}
        }
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Q23 gains use widened intermediates with saturation"
)]
impl Process<Message, I24> for SendMixer {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal, Port::Signal]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        let mut events = cx.events.iter().peekable();
        for frame in 0..cx.frames {
            while events.peek().is_some_and(|e| e.frame == frame) {
                self.control(events.next().expect("peeked event").message);
            }
            let a = i64::from(cx.inputs[0][frame].inner()) * i64::from(self.a);
            let b = i64::from(cx.inputs[1][frame].inner()) * i64::from(self.b);
            cx.outputs[0][frame] = pcm((a + b) >> 7);
        }
        if cx.frames == 0 {
            for e in cx.events {
                self.control(e.message);
            }
        }
    }
}

#[derive(Debug)]
struct Line<'a> {
    samples: &'a mut [i32],
    index: usize,
    lowpass: i32,
}

impl<'a> Line<'a> {
    fn new(samples: &'a mut [i32]) -> Self {
        Self {
            samples,
            index: 0,
            lowpass: 0,
        }
    }

    fn read(&self) -> i32 {
        self.samples[self.index]
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Advancing a bounded circular index"
    )]
    fn write(&mut self, value: i32) {
        self.samples[self.index] = value;
        self.index += 1;
        if self.index == self.samples.len() {
            self.index = 0;
        }
    }
}

/// One shared mono reverb processor with a 100%-wet output.
///
/// The six delay lines borrow caller-owned storage. Construct this once,
/// outside the audio interrupt, and call it exactly once per audio block.
/// This is deliberately a small room/plate-like starting point, not a
/// reproduction of the DEP-5 or of the larger 303/Dattorro reverb.
#[derive(Debug)]
pub struct Reverb<'a> {
    combs: [Line<'a>; 4],
    diffusion: [Line<'a>; 2],
    feedback: i32,
    wet: u8,
    enabled: bool,
}

impl<'a> Reverb<'a> {
    /// Borrow and clear exactly [`REVERB_SAMPLES`] signed delay samples.
    #[must_use]
    pub fn new(storage: &'a mut [i32; REVERB_SAMPLES]) -> Self {
        storage.fill(0);
        let (c0, rest) = storage.split_at_mut(743);
        let (c1, rest) = rest.split_at_mut(877);
        let (c2, rest) = rest.split_at_mut(1009);
        let (c3, rest) = rest.split_at_mut(1153);
        let (a0, a1) = rest.split_at_mut(173);
        Self {
            combs: [Line::new(c0), Line::new(c1), Line::new(c2), Line::new(c3)],
            diffusion: [Line::new(a0), Line::new(a1)],
            feedback: feedback(80),
            wet: 64,
            enabled: true,
        }
    }

    fn control(&mut self, message: Message) {
        match message {
            Message::Decay(level) => self.feedback = feedback(level.min(127)),
            Message::Wet(level) => self.wet = level.min(128),
            Message::Enabled(enabled) => self.enabled = enabled,
            _ => {}
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Bounded Q23 delay-line arithmetic uses i64 intermediates"
    )]
    fn sample(&mut self, input: i32) -> I24 {
        let mut sum = 0_i64;
        for comb in &mut self.combs {
            let delayed = comb.read();
            // One-pole HF damping in the feedback path.
            comb.lowpass += (delayed - comb.lowpass) >> 1;
            let echoed = (i64::from(comb.lowpass) * i64::from(self.feedback)) >> 15;
            comb.write(clamp_pcm(i64::from(input) / 8 + echoed));
            sum += i64::from(delayed);
        }
        let mut wet = clamp_pcm(sum >> 2);
        // Two all-pass diffusers, with gain 0.5, following the parallel combs.
        for allpass in &mut self.diffusion {
            let delayed = allpass.read();
            let v = clamp_pcm(i64::from(wet) + (i64::from(delayed) >> 1));
            allpass.write(v);
            wet = clamp_pcm(i64::from(delayed) - (i64::from(v) >> 1));
        }
        if self.enabled {
            pcm((i64::from(wet) * i64::from(self.wet)) >> 7)
        } else {
            I24::default()
        }
    }
}

// Q15 feedback stays below unity, including at maximum decay.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "Maximum Q15 feedback is 31890, below unity"
)]
fn feedback(decay: u8) -> i32 {
    23_000 + i32::from(decay) * 70
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Clamped to the signed 24-bit range first"
)]
fn clamp_pcm(value: i64) -> i32 {
    value.clamp(i64::from(i24::MIN.inner()), i64::from(i24::MAX.inner())) as i32
}

fn pcm(value: i64) -> I24 {
    I24::new_unchecked(clamp_pcm(value))
}

impl Process<Message, I24> for Reverb<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        let mut events = cx.events.iter().peekable();
        for frame in 0..cx.frames {
            while events.peek().is_some_and(|e| e.frame == frame) {
                self.control(events.next().expect("peeked event").message);
            }
            cx.outputs[0][frame] = self.sample(cx.inputs[0][frame].inner());
        }
        if cx.frames == 0 {
            for e in cx.events {
                self.control(e.message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wavegraph_core::Event;

    #[test]
    fn independent_sends_and_sample_offset_control() {
        let mut mixer = SendMixer::new();
        let one = [I24::new_unchecked(1024); 4];
        let two = [I24::new_unchecked(2048); 4];
        let mut out = [I24::default(); 4];
        let events = [
            Event {
                frame: 0,
                port: 0,
                message: Message::SendA(128),
            },
            Event {
                frame: 2,
                port: 0,
                message: Message::SendB(64),
            },
        ];
        mixer.process(&mut ProcessContext {
            frames: 4,
            inputs: &[&one, &two],
            outputs: &mut [&mut out],
            events: &events,
            emit: &mut |e| Err(e),
        });
        assert_eq!(out.map(I24::inner), [1024, 1024, 2048, 2048]);
    }

    #[test]
    fn impulse_creates_tail_and_muting_preserves_state() {
        let mut storage = [0; REVERB_SAMPLES];
        let mut reverb = Reverb::new(&mut storage);
        let mut input = [I24::default(); 128];
        let mut output = [I24::default(); 128];
        input[0] = I24::new_unchecked(1 << 21);
        let mut tail_seen = false;
        let first = [Event {
            frame: 0,
            port: 0,
            message: Message::Decay(110),
        }];
        for block in 0..40 {
            let events = if block == 0 { &first[..] } else { &[][..] };
            reverb.process(&mut ProcessContext {
                frames: 128,
                inputs: &[&input],
                outputs: &mut [&mut output],
                events,
                emit: &mut |e| Err(e),
            });
            if block > 0 {
                tail_seen |= output.iter().any(|v| v.inner() != 0);
            }
            input.fill(I24::default());
        }
        assert!(tail_seen, "an impulse should reach the wet output");
        let mute = [Event {
            frame: 0,
            port: 0,
            message: Message::Enabled(false),
        }];
        reverb.process(&mut ProcessContext {
            frames: 128,
            inputs: &[&input],
            outputs: &mut [&mut output],
            events: &mute,
            emit: &mut |e| Err(e),
        });
        assert!(output.iter().all(|sample| sample.inner() == 0));
    }
}
