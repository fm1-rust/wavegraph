//! Allocation-free shared effects for signed Q23 WaveGraph patches.
//!
//! Route independent post-fader [`ChannelMixer`] sends through a [`Reverb`]
//! and a [`Delay`]. Add their wet-only returns to the dry mix. Each processor
//! keeps its state across graph edits.
#![no_std]
#![forbid(unsafe_code)]

mod delay;
mod ducker;
mod eq;
mod insert;
mod master;
mod mixer;
pub use delay::Delay;
pub use ducker::SidechainDucker;
pub use eq::{EqControls, ThreeBandEq};
pub use insert::{
    FILTER_PATTERNS, FilterMode, FilterPattern, Flanger, ModulationControls,
    ModulationControls as FlangerControls, ModulationControls as PhaserControls, PatternFilter,
    PatternFilterControls, Phaser, Saturator, SaturatorControls,
};
pub use master::{Dynamics, MasterBus, MasterControls};
pub use mixer::{ChannelMixer, MixerMessage};

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
    /// Digital delay time in milliseconds, limited by the caller's delay line.
    TimeMs(u16),
    /// Digital delay feedback in percent, clamped to 95. Zero gives one echo.
    Feedback(u8),
    /// Select a detector signal inlet on a sidechain insert. Inlet 0 keys from the program.
    SidechainKey(u8),
    /// Percentage of the insert's maximum gain reduction, 0..=100.
    SidechainDepth(u8),
    /// Low-shelf corner frequency, clamped to 40..=1000 Hz.
    SidechainHz(u16),
    /// Wet return level, 0..=128; dry audio is never attenuated here.
    Wet(u8),
    /// Saturator controls.
    Saturator(SaturatorControls),
    /// Three-band channel EQ controls.
    Eq(EqControls),
    /// Phaser controls.
    Phaser(PhaserControls),
    /// Flanger controls.
    Flanger(FlangerControls),
    /// Resonant pattern filter and envelope settings.
    PatternFilter(PatternFilterControls),
    /// Trigger the pattern filter envelope at this sample offset, with strength 0..=127.
    FilterTrigger(u8),
    /// Enable the effect. Disabling mutes a return or bypasses an insert.
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
    reason = "Two Q23 samples at max gain 128 sum to a signed i32 before shifting."
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
            let a = cx.inputs[0][frame].inner() * i32::from(self.a);
            let b = cx.inputs[1][frame].inner() * i32::from(self.b);
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

    // Visit the ring in contiguous spans so only a wrap splits the sample loop.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Each span ends within the ring"
    )]
    fn spans(&mut self, frames: usize, mut visit: impl FnMut(usize, &mut [i32], &mut i32)) {
        let mut start = 0;
        while start < frames {
            let count = (frames - start).min(self.samples.len() - self.index);
            let end = self.index + count;
            visit(start, &mut self.samples[self.index..end], &mut self.lowpass);
            self.index = if end == self.samples.len() { 0 } else { end };
            start += count;
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
        clippy::cast_possible_truncation,
        reason = "Four Q23 taps fit in i32. The Q23 × Q15 feedback fits Q23 after shifting."
    )]
    fn render(&mut self, input: &[I24], output: &mut [I24]) {
        // Four Q23 taps need a wider accumulator until their sum is divided by four.
        let mut sum = [0_i32; 64];
        for (input, output) in input.chunks(sum.len()).zip(output.chunks_mut(sum.len())) {
            let sum = &mut sum[..input.len()];
            sum.fill(0);
            for comb in &mut self.combs {
                comb.spans(input.len(), |start, taps, lowpass| {
                    for ((tap, sample), total) in
                        taps.iter_mut().zip(&input[start..]).zip(&mut sum[start..])
                    {
                        let delayed = *tap;
                        *lowpass += (delayed - *lowpass) >> 1;
                        let echo = ((i64::from(*lowpass) * i64::from(self.feedback)) >> 15) as i32;
                        *tap = clamp_pcm(sample.inner() / 8 + echo);
                        *total += delayed;
                    }
                });
            }
            for wet in &mut *sum {
                *wet >>= 2;
            }
            for diffuser in &mut self.diffusion {
                diffuser.spans(input.len(), |start, taps, _| {
                    for (tap, wet) in taps.iter_mut().zip(&mut sum[start..]) {
                        let delayed = *tap;
                        *tap = clamp_pcm(*wet + (delayed >> 1));
                        *wet = clamp_pcm(delayed - (*tap >> 1));
                    }
                });
            }
            if self.enabled {
                for (out, wet) in output.iter_mut().zip(sum) {
                    *out = pcm((*wet * i32::from(self.wet)) >> 7);
                }
            } else {
                output.fill(I24::default());
            }
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

fn clamp_pcm(value: i32) -> i32 {
    value.clamp(i24::MIN.inner(), i24::MAX.inner())
}

fn pcm(value: i32) -> I24 {
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
        if cx.frames == 0 {
            for event in cx.events {
                self.control(event.message);
            }
            return;
        }
        let mut start = 0;
        for event in cx.events {
            self.render(
                &cx.inputs[0][start..event.frame],
                &mut cx.outputs[0][start..event.frame],
            );
            self.control(event.message);
            start = event.frame;
        }
        self.render(
            &cx.inputs[0][start..cx.frames],
            &mut cx.outputs[0][start..cx.frames],
        );
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
    fn full_scale_sends_saturate_after_the_sum() {
        let mut mixer = SendMixer::new();
        let a = [i24::MIN, i24::MAX, i24::MIN, i24::MAX];
        let b = [i24::MIN, i24::MAX, i24::MAX, i24::MIN];
        let mut out = [I24::default(); 4];
        mixer.process(&mut ProcessContext {
            frames: 4,
            inputs: &[&a, &b],
            outputs: &mut [&mut out],
            events: &[Message::SendA(128), Message::SendB(128)].map(|message| Event {
                frame: 0,
                port: 0,
                message,
            }),
            emit: &mut Err,
        });
        assert_eq!(
            out.map(I24::inner),
            [i24::MIN.inner(), i24::MAX.inner(), -1, -1]
        );
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
