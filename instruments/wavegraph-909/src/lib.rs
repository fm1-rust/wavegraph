#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod analog;
mod clap;
mod pattern;

pub use analog::{BassDrum, SnareDrum};
pub use clap::HandClap;
use core::ops::Range;
pub use pattern::DEMO_PATTERN;
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::FromSample;
use wavegraph_sampler::Sampler;

/// Send a voice to the message inlet to strike it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Voice {
    /// Bass drum.
    Kick,
    /// Snare drum.
    Snare,
    /// Hand clap.
    Clap,
    /// Closed hi-hat.
    ClosedHat,
    /// Open hi-hat.
    OpenHat,
    /// Ride cymbal.
    Ride,
}

impl Voice {
    /// The six voices in dry-outlet order.
    pub const ALL: [Self; 6] = [
        Self::Kick,
        Self::Snare,
        Self::Clap,
        Self::ClosedHat,
        Self::OpenHat,
        Self::Ride,
    ];

    /// Look up a supported voice using the TR-909's MIDI key assignments.
    #[must_use]
    pub const fn from_midi_note(note: u8) -> Option<Self> {
        Some(match note {
            35 | 36 => Self::Kick,
            38 | 40 => Self::Snare,
            39 => Self::Clap,
            42 | 44 => Self::ClosedHat,
            46 => Self::OpenHat,
            51 => Self::Ride,
            _ => return None,
        })
    }

    /// This voice's dry signal outlet. Outlet zero carries the kit mix.
    #[must_use]
    pub const fn outlet(self) -> usize {
        (self as usize).saturating_add(1)
    }
}

/// Mono s16le clips at the graph's playback rate. Empty clips are silent.
#[derive(Clone, Copy, Debug)]
pub struct Samples<'a> {
    /// Closed hi-hat recording.
    pub closed_hat: &'a [u8],
    /// Open hi-hat recording.
    pub open_hat: &'a [u8],
    /// Ride cymbal recording.
    pub ride: &'a [u8],
}

impl Samples<'static> {
    /// Fugue State Audio's CC0 `x0xproc1` kit at 44.1 kHz (148,124 bytes of PCM).
    pub const X0XPROC1: Self = Self {
        closed_hat: include_bytes!("../samples/closed-hat.s16le"),
        open_hat: include_bytes!("../samples/open-hat.s16le"),
        ride: include_bytes!("../samples/ride.s16le"),
    };
}

/// A hybrid drum kit with one mix and six independent voice outputs.
#[derive(Clone, Copy, Debug)]
pub struct Tr909<'a> {
    closed_hat: Sampler<'a>,
    open_hat: Sampler<'a>,
    ride: Sampler<'a>,
    clap: HandClap,
    kick: BassDrum,
    snare: SnareDrum,
    // An open strike wins over a closed strike until audio time advances.
    opened_this_frame: bool,
}

impl<'a> Tr909<'a> {
    /// Borrow the three cymbal recordings and run the drum models at 44.1 kHz.
    /// Returns `None` for an incomplete PCM frame.
    #[must_use]
    pub fn new(samples: Samples<'a>) -> Option<Self> {
        Self::with_sample_rate(samples, 44_100, 1)
    }

    /// Use a graph clock of `numerator / denominator` frames per second.
    /// Supply PCM at that rate. A zero clock or incomplete PCM frame returns `None`.
    #[must_use]
    pub fn with_sample_rate(
        samples: Samples<'a>,
        numerator: u32,
        denominator: u32,
    ) -> Option<Self> {
        Self::with_decay(samples, numerator, denominator, 100)
    }

    /// Construct at a rational sample rate with shorter modeled drum tails.
    /// `decay_percent` scales their amplitude decay times (100 is stock,
    /// larger values select 100). Pitch sweeps, clap burst timing and sampled
    /// cymbals keep their original timing.
    #[must_use]
    pub fn with_decay(
        samples: Samples<'a>,
        numerator: u32,
        denominator: u32,
        decay_percent: u8,
    ) -> Option<Self> {
        Self::with_cymbals(
            [
                Sampler::from_pcm16le(samples.closed_hat)?,
                Sampler::from_pcm16le(samples.open_hat)?,
                Sampler::from_pcm16le(samples.ride)?,
            ],
            numerator,
            denominator,
            decay_percent,
        )
    }

    /// Supply closed hat, open hat and ride players in that order. The clock and
    /// modeled drum decay have the same meaning as in [`Self::with_decay`].
    /// Returns `None` for a zero clock.
    #[must_use]
    pub fn with_cymbals(
        [mut closed_hat, mut open_hat, mut ride]: [Sampler<'a>; 3],
        numerator: u32,
        denominator: u32,
        decay_percent: u8,
    ) -> Option<Self> {
        let clock = analog::Clock::new(numerator, denominator)?;
        closed_hat.stop();
        open_hat.stop();
        ride.stop();
        let decay = f32::from(decay_percent.min(100)) / 100.0;
        Some(Self {
            clap: HandClap::from_clock(clock, decay),
            kick: BassDrum::from_clock(clock, decay),
            snare: SnareDrum::from_clock(clock, decay),
            closed_hat,
            open_hat,
            ride,
            opened_this_frame: false,
        })
    }

    /// Strike a voice at the current audio frame.
    pub fn trigger(&mut self, voice: Voice) {
        match voice {
            Voice::Clap => self.clap.trigger(),
            Voice::Kick => self.kick.trigger(),
            Voice::Snare => self.snare.trigger(),
            Voice::OpenHat => {
                self.opened_this_frame = true;
                self.closed_hat.stop();
                self.open_hat.trigger();
            }
            Voice::ClosedHat => {
                if !self.opened_this_frame {
                    self.open_hat.stop();
                    self.closed_hat.trigger();
                }
            }
            Voice::Ride => self.ride.trigger(),
        }
    }

    /// Silence all voices immediately, including ringing cymbals.
    pub fn stop(&mut self) {
        self.closed_hat.stop();
        self.open_hat.stop();
        self.ride.stop();
        self.kick.stop();
        self.snare.stop();
        self.clap.stop();
        self.opened_this_frame = false;
    }

    /// Whether the voice's envelope or sample is still running. Choking a hat
    /// clears it immediately, even when another voice continues to ring.
    #[must_use]
    pub fn voice_active(&self, voice: Voice) -> bool {
        match voice {
            Voice::Kick => self.kick.is_active(),
            Voice::Snare => self.snare.is_active(),
            Voice::Clap => self.clap.is_active(),
            Voice::ClosedHat => self.closed_hat.is_active(),
            Voice::OpenHat => self.open_hat.is_active(),
            Voice::Ride => self.ride.is_active(),
        }
    }

    /// Advance all six voices one frame and return dry PCM in [`Voice::ALL`] order.
    #[inline]
    pub fn next_voices(&mut self) -> [i16; 6] {
        self.opened_this_frame = false;
        [
            self.kick.next_frame(),
            self.snare.next_frame(),
            self.clap.next_frame(),
            self.closed_hat.next_frame(),
            self.open_hat.next_frame(),
            self.ride.next_frame(),
        ]
    }

    /// Render only the mono mix, using the same gain as signal outlet zero.
    #[inline]
    pub fn next_mix_frame<S: FromSample<i32>>(&mut self) -> S {
        mix(&self.next_voices())
    }

    fn render<S: FromSample<i32>>(&mut self, outputs: &mut [&mut [S]], frames: Range<usize>) {
        let (mixed, stems) = outputs.split_at_mut(1);
        for frame in frames {
            let voices = self.next_voices();
            mixed[0][frame] = mix(&voices);
            for (pcm, stem) in voices.into_iter().zip(stems.iter_mut()) {
                stem[frame] = S::from_sample_(i32::from(pcm) << 16);
            }
        }
    }
}

#[inline]
fn mix<S: FromSample<i32>>(voices: &[i16; 6]) -> S {
    let sum: i32 = voices.iter().copied().map(i32::from).sum();
    S::from_sample_(sum.saturating_mul(1 << 16))
}

impl<S: FromSample<i32>> Process<Voice, S> for Tr909<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal; 7]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Voice, S>) {
        let mut start = 0;
        for event in cx.events {
            self.render(cx.outputs, start..event.frame);
            self.trigger(event.message);
            start = event.frame;
        }
        self.render(cx.outputs, start..cx.frames);
    }
}

// Each modeled voice can also occupy a graph node on its own. A unit message
// strikes it; the kit calls the same trigger and render methods for its stems.
macro_rules! impl_drum_process {
    ($($voice:ty),+) => {$(
        impl<S: FromSample<i16>> Process<(), S> for $voice {
            fn inlets(&self) -> &[Port] { &[Port::Message] }
            fn outlets(&self) -> &[Port] { &[Port::Signal] }

            fn process(&mut self, cx: &mut ProcessContext<'_, '_, (), S>) {
                let output = &mut cx.outputs[0];
                let mut start = 0;
                for event in cx.events {
                    for sample in &mut output[start..event.frame] {
                        *sample = S::from_sample_(self.next_frame());
                    }
                    self.trigger();
                    start = event.frame;
                }
                for sample in &mut output[start..cx.frames] {
                    *sample = S::from_sample_(self.next_frame());
                }
            }
        }
    )+};
}
impl_drum_process!(HandClap, BassDrum, SnareDrum);
