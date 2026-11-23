#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod voices;

use core::ops::Range;
pub use voices::{BassDrum, HiHat, Metal, SnareDrum};
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::FromSample;

/// The four drum voices in signal-outlet order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Voice {
    /// Bass drum.
    Kick,
    /// Snare drum.
    Snare,
    /// Closed hi-hat.
    ClosedHat,
    /// Open hi-hat.
    OpenHat,
}

impl Voice {
    /// All voices, in dry-stem order.
    pub const ALL: [Self; 4] = [Self::Kick, Self::Snare, Self::ClosedHat, Self::OpenHat];

    /// Translate one of the four kit notes.
    #[must_use]
    pub const fn from_midi_note(note: u8) -> Option<Self> {
        Some(match note {
            36 => Self::Kick,
            38 => Self::Snare,
            42 => Self::ClosedHat,
            46 => Self::OpenHat,
            _ => return None,
        })
    }

    /// Dry signal outlet (the kit mix is outlet zero).
    #[must_use]
    pub const fn outlet(self) -> usize {
        (self as usize).saturating_add(1)
    }
}

/// Bass and snare panel settings, each in the range 0 to 127.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Controls {
    /// Kick transient brightness.
    pub kick_tone: u8,
    /// Kick body decay.
    pub kick_decay: u8,
    /// Balance between the two snare resonances.
    pub snare_tone: u8,
    /// Snare wire level.
    pub snappy: u8,
}

impl Controls {
    /// Factory kit settings.
    pub const INITIAL: Self = Self {
        kick_tone: 78,
        kick_decay: 57,
        snare_tone: 58,
        snappy: 91,
    };
}

/// Frame-stamped kit commands, received on inlet zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Strike a voice with a MIDI velocity. Zero is a no-op.
    Strike(Voice, u8),
    /// Shared excitation boost for subsequent strikes (0 to 127).
    Accent(u8),
    /// Change the timbre of the bass drum and snare.
    Controls(Controls),
    /// Set a child's output level (127 is unity).
    Level(Voice, u8),
    /// Silence all four voices immediately.
    Stop,
}

impl Message {
    /// Translate a MIDI note-on. Unmapped notes and zero velocity are ignored.
    #[must_use]
    pub fn note_on(note: u8, velocity: u8) -> Option<Self> {
        Voice::from_midi_note(note)
            .filter(|_| velocity != 0)
            .map(|v| Self::Strike(v, velocity))
    }
}

/// Four playable children with a shared metal source, mono sum and four dry stems.
#[derive(Clone, Copy, Debug)]
pub struct Tr808 {
    kick: BassDrum,
    snare: SnareDrum,
    closed: HiHat,
    open: HiHat,
    metal: Metal,
    silent_metal_frames: u32,
    closed_this_frame: bool,
    opened_this_frame: bool,
    accent: f32,
}

impl Tr808 {
    /// Construct at a rational sample rate between 1 and 384 kHz.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        let clock = voices::Clock::new(numerator, denominator)?;
        Some(Self {
            kick: BassDrum::from_clock(clock),
            snare: SnareDrum::from_clock(clock),
            closed: HiHat::from_clock(clock, false),
            open: HiHat::from_clock(clock, true),
            metal: Metal::from_clock(clock),
            silent_metal_frames: 0,
            closed_this_frame: false,
            opened_this_frame: false,
            accent: 1.0,
        })
    }

    /// Apply a message immediately before the next sample.
    pub fn send(&mut self, message: Message) {
        match message {
            Message::Strike(voice, velocity) => self.trigger(voice, velocity),
            Message::Accent(value) => self.accent = 1.0 + 0.25 * voices::unit(value),
            Message::Controls(c) => {
                self.kick.set_tone(c.kick_tone);
                self.kick.set_decay(c.kick_decay);
                self.snare.set_tone(c.snare_tone);
                self.snare.set_snappy(c.snappy);
            }
            Message::Level(voice, level) => match voice {
                Voice::Kick => self.kick.set_level(level),
                Voice::Snare => self.snare.set_level(level),
                Voice::ClosedHat => self.closed.set_level(level),
                Voice::OpenHat => self.open.set_level(level),
            },
            Message::Stop => {
                self.kick.stop();
                self.snare.stop();
                self.closed.stop();
                self.open.stop();
                self.closed_this_frame = false;
                self.opened_this_frame = false;
            }
        }
    }

    /// Strike a child at the current frame.
    pub fn trigger(&mut self, voice: Voice, velocity: u8) {
        if velocity == 0 {
            return;
        }
        let strength = voices::unit(velocity) * self.accent;
        match voice {
            Voice::Kick => self.kick.excite(strength),
            Voice::Snare => self.snare.excite(strength),
            Voice::ClosedHat => {
                self.closed_this_frame = true;
                self.open.choke();
                if !self.opened_this_frame {
                    self.closed.excite(strength);
                }
            }
            Voice::OpenHat => {
                self.opened_this_frame = true;
                self.closed.stop();
                self.open.excite(strength);
                if self.closed_this_frame {
                    self.open.choke();
                }
            }
        }
    }

    /// Whether a child is still ringing.
    #[must_use]
    pub fn voice_active(&self, voice: Voice) -> bool {
        match voice {
            Voice::Kick => self.kick.is_active(),
            Voice::Snare => self.snare.is_active(),
            Voice::ClosedHat => self.closed.is_active(),
            Voice::OpenHat => self.open.is_active(),
        }
    }

    /// Advance once and return the four PCM16 stems in [`Voice::ALL`] order.
    #[inline]
    pub fn next_voices(&mut self) -> [i16; 4] {
        self.closed_this_frame = false;
        self.opened_this_frame = false;
        let (closed, open) = if self.closed.is_active() || self.open.is_active() {
            if self.silent_metal_frames != 0 {
                self.metal
                    .advance(core::mem::take(&mut self.silent_metal_frames));
            }
            let metal = self.metal.next_frame();
            (self.closed.next_frame(metal), self.open.next_frame(metal))
        } else {
            self.silent_metal_frames = self.silent_metal_frames.wrapping_add(1);
            (0, 0)
        };
        [
            self.kick.next_frame(),
            self.snare.next_frame(),
            closed,
            open,
        ]
    }

    /// Render the mono sum and isolated kick together, e.g. for a sidechain detector.
    ///
    /// # Panics
    /// Panics if the two buffers have different lengths.
    pub fn render_mix_kick<S: FromSample<i16> + Copy>(&mut self, mix: &mut [S], kick: &mut [S]) {
        let frames = mix.len();
        assert_eq!(frames, kick.len());
        self.render(&mut [mix, kick], 0..frames);
    }

    fn render<S: FromSample<i16> + Copy>(
        &mut self,
        outputs: &mut [&mut [S]],
        frames: Range<usize>,
    ) {
        if frames.is_empty() {
            return;
        }
        if !Voice::ALL.into_iter().any(|v| self.voice_active(v)) {
            // Phase arithmetic is modulo 2^32, including on 64-bit hosts.
            #[allow(clippy::cast_possible_truncation)]
            let advance = frames.len() as u32;
            self.silent_metal_frames = self.silent_metal_frames.wrapping_add(advance);
            for out in outputs {
                out[frames.clone()].fill(S::from_sample_(0));
            }
            return;
        }
        let (mix, stems) = outputs.split_at_mut(1);
        for frame in frames {
            let voices = self.next_voices();
            let sum: i32 = voices.into_iter().map(i32::from).sum();
            let pcm = sum.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
            mix[0][frame] = S::from_sample_(i16::try_from(pcm).expect("clamped PCM16"));
            for (out, sample) in stems.iter_mut().zip(voices) {
                out[frame] = S::from_sample_(sample);
            }
        }
    }
}

impl<S: FromSample<i16> + Copy> Process<Message, S> for Tr808 {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal; 5]
    }
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, S>) {
        let mut start = 0;
        for event in cx.events {
            self.render(cx.outputs, start..event.frame);
            self.send(event.message);
            start = event.frame;
        }
        self.render(cx.outputs, start..cx.frames);
    }
}
