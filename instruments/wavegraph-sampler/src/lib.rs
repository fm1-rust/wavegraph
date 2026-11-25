#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod pitched;

pub use pitched::{
    Clip, FULL_SLICES, POLYPHONY, PitchedMessage, PitchedSampler, Playback, PlaybackMode, Slice,
    SliceTrims, Timbre,
};

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

/// One mono voice. Copies share the immutable clip and play independently.
#[derive(Clone, Copy, Debug)]
pub struct Sampler<'a>(Storage<'a>);

#[derive(Clone, Copy, Debug)]
enum Storage<'a> {
    Raw {
        pcm: &'a [[u8; 2]],
        remaining: &'a [[u8; 2]],
    },
    #[cfg(feature = "packed-pcm")]
    Packed {
        clip: fm1_assets::pcm::PackedPcm16<'a>,
        cursor: Option<fm1_assets::pcm::Samples<'a>>,
    },
}

impl<'a> Sampler<'a> {
    /// Borrow raw signed 16-bit little-endian PCM at the playback sample rate.
    /// A new voice is silent until triggered. An empty clip is valid.
    /// Returns `None` if the last frame is missing a byte.
    #[must_use]
    pub const fn from_pcm16le(bytes: &'a [u8]) -> Option<Self> {
        let (pcm, trailing) = bytes.as_chunks::<2>();
        if trailing.is_empty() {
            Some(Self(Storage::Raw {
                pcm,
                remaining: &[],
            }))
        } else {
            None
        }
    }

    /// Borrow a validated packed PCM16 clip. Triggering starts a new cursor.
    #[cfg(feature = "packed-pcm")]
    #[must_use]
    pub const fn from_packed(clip: fm1_assets::pcm::PackedPcm16<'a>) -> Self {
        Self(Storage::Packed { clip, cursor: None })
    }

    /// Play from the beginning on the next frame.
    pub fn trigger(&mut self) {
        match &mut self.0 {
            Storage::Raw { pcm, remaining } => *remaining = pcm,
            #[cfg(feature = "packed-pcm")]
            Storage::Packed { clip, cursor } => *cursor = Some(clip.samples()),
        }
    }

    /// Silence this voice immediately.
    pub fn stop(&mut self) {
        match &mut self.0 {
            Storage::Raw { remaining, .. } => *remaining = &[],
            #[cfg(feature = "packed-pcm")]
            Storage::Packed { cursor, .. } => *cursor = None,
        }
    }

    /// Whether this one-shot still has frames to play.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        match &self.0 {
            Storage::Raw { remaining, .. } => !remaining.is_empty(),
            #[cfg(feature = "packed-pcm")]
            Storage::Packed { cursor, .. } => matches!(cursor, Some(c) if c.remaining() != 0),
        }
    }

    /// Read one mono frame for a containing instrument. An idle voice yields zero.
    pub fn next_frame(&mut self) -> i16 {
        match &mut self.0 {
            Storage::Raw { remaining, .. } => {
                let Some((&bytes, tail)) = remaining.split_first() else {
                    return 0;
                };
                *remaining = tail;
                i16::from_le_bytes(bytes)
            }
            #[cfg(feature = "packed-pcm")]
            Storage::Packed { cursor, .. } => cursor.as_mut().and_then(Iterator::next).unwrap_or(0),
        }
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
