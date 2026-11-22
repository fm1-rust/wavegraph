#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::FromSample;

/// Frame-timed controls for a one-shot voice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Start at the first sample, including when already playing.
    Trigger,
    /// Silence the voice from this frame onward.
    Stop,
}

/// One mono voice. Both slices borrow the same PCM storage. A copy of this
/// voice has its own playback position.
#[derive(Clone, Copy, Debug)]
pub struct Sampler<'a> {
    pcm: &'a [[u8; 2]],
    remaining: &'a [[u8; 2]],
}

impl<'a> Sampler<'a> {
    /// Borrow raw signed 16-bit little-endian PCM at the playback sample rate.
    /// A new voice is silent until triggered. An empty clip is valid.
    /// Returns `None` if the last frame is missing a byte.
    #[must_use]
    pub const fn from_pcm16le(bytes: &'a [u8]) -> Option<Self> {
        let (pcm, trailing) = bytes.as_chunks::<2>();
        if trailing.is_empty() {
            Some(Self {
                pcm,
                remaining: &[],
            })
        } else {
            None
        }
    }

    /// Play from the beginning on the next frame.
    pub fn trigger(&mut self) {
        self.remaining = self.pcm;
    }

    /// Silence this voice immediately.
    pub fn stop(&mut self) {
        self.remaining = &[];
    }

    /// Read one mono frame for a containing instrument. An idle voice yields zero.
    pub fn next_frame(&mut self) -> i16 {
        let Some((&bytes, tail)) = self.remaining.split_first() else {
            return 0;
        };
        self.remaining = tail;
        i16::from_le_bytes(bytes)
    }

    fn render<S: FromSample<i16>>(&mut self, output: &mut [S]) {
        for sample in output {
            *sample = S::from_sample_(self.next_frame());
        }
    }
}

impl<S: FromSample<i16>> Process<Message, S> for Sampler<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, S>) {
        let output = &mut cx.outputs[0];
        let mut start = 0;
        for event in cx.events {
            self.render(&mut output[start..event.frame]);
            match event.message {
                Message::Trigger => self.trigger(),
                Message::Stop => self.stop(),
            }
            start = event.frame;
        }
        self.render(&mut output[start..]);
    }
}
