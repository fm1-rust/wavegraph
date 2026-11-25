//! Four-voice keyboard playback of a bank of immutable PCM clips.

#[cfg(feature = "ima-adpcm")]
mod ima;
mod pcm;
mod tone;

pub use tone::Timbre;
use tone::VoiceTone;

use pcm::{Cursor, Pcm};
use wavegraph_core::{Port, Process, ProcessContext, midi::Note};
use wavegraph_sample::FromSample;

/// Independent sample voices per chromatic player.
pub const POLYPHONY: usize = 4;

const FRAC_BITS: u32 = 15;
const ONE: u32 = 1 << FRAC_BITS;
const SEMITONES: [u32; 12] = [
    32768, 34716, 36781, 38968, 41285, 43740, 46341, 49097, 52016, 55109, 58386, 61858,
];
const FULL_GAIN: u32 = 1 << 24;

/// Mono PCM16 with its recording rate and the MIDI key that plays it at native speed.
#[derive(Clone, Copy, Debug)]
pub struct Clip<'a> {
    pcm: Pcm<'a>,
    source_hz: u32,
    root: u8,
    native_bpm: u16,
}

impl<'a> Clip<'a> {
    /// Borrow the data chunk of a mono IMA ADPCM WAV with 64-byte blocks.
    /// `frames` excludes encoder padding in the final block.
    #[cfg(feature = "ima-adpcm")]
    #[must_use]
    pub const fn from_ima_adpcm(
        bytes: &'a [u8],
        frames: u32,
        source_hz: u32,
        root: u8,
    ) -> Option<Self> {
        let (blocks, tail) = bytes.as_chunks::<64>();
        if !tail.is_empty() || blocks.len() as u64 != frames.div_ceil(ima::FRAMES) as u64 {
            return None;
        }
        let mut remaining = blocks;
        while let [block, rest @ ..] = remaining {
            if block[2] > 88 || block[3] != 0 {
                return None;
            }
            remaining = rest;
        }
        Self::new(Pcm::Ima { blocks, frames }, source_hz, root)
    }

    /// Borrow signed little-endian PCM. Reject incomplete frames, zero Hz or a non-MIDI root.
    #[must_use]
    pub const fn from_pcm16le(bytes: &'a [u8], source_hz: u32, root: u8) -> Option<Self> {
        let (pcm, tail) = bytes.as_chunks::<2>();
        if !tail.is_empty() {
            return None;
        }
        Self::new(Pcm::Raw(pcm), source_hz, root)
    }

    /// Borrow losslessly bit-packed PCM1, validating all groups once. Can be used in a static bank.
    #[cfg(feature = "packed-pcm")]
    #[must_use]
    pub const fn from_pcm1(bytes: &'a [u8], source_hz: u32, root: u8) -> Option<Self> {
        Self::packed(bytes, source_hz, root, false)
    }

    /// Lossless second differences stored in PCM1. The sample and its first difference
    /// both start at zero every 128 frames. All predictor arithmetic wraps at 16 bits.
    #[cfg(feature = "packed-pcm")]
    #[must_use]
    pub const fn from_delta_pcm1(bytes: &'a [u8], source_hz: u32, root: u8) -> Option<Self> {
        Self::packed(bytes, source_hz, root, true)
    }

    #[cfg(feature = "packed-pcm")]
    const fn packed(bytes: &'a [u8], source_hz: u32, root: u8, predictive: bool) -> Option<Self> {
        let Some(clip) = fm1_assets::pcm::PackedPcm16::parse(bytes) else {
            return None;
        };
        let (_, groups) = bytes.split_at(8);
        Self::new(
            Pcm::Packed {
                groups,
                frames: clip.frames(),
                predictive,
            },
            source_hz,
            root,
        )
    }

    const fn new(pcm: Pcm<'a>, source_hz: u32, root: u8) -> Option<Self> {
        if source_hz == 0 || root > 127 {
            return None;
        }
        Some(Self {
            pcm,
            source_hz,
            root,
            native_bpm: 0,
        })
    }

    /// Follow the transport by varying playback speed. Zero leaves a clip at its own tempo.
    #[must_use]
    pub const fn with_native_tempo(mut self, bpm: u16) -> Self {
        self.native_bpm = bpm;
        self
    }

    /// MIDI key at which the clip plays at its original speed.
    #[must_use]
    pub const fn root_note(self) -> u8 {
        self.root
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A 7-bit key difference spans at most eleven octaves. Both scaled clock terms and rounding fit u64."
    )]
    fn step(self, note: u8, output_hz: u32) -> u32 {
        let interval = i32::from(note) - i32::from(self.root);
        let octave = interval.div_euclid(12);
        let semitone = usize::try_from(interval.rem_euclid(12)).expect("one octave");
        let numerator =
            (u64::from(self.source_hz) * u64::from(SEMITONES[semitone])) << octave.max(0);
        let denominator = u64::from(output_hz) << (-octave).max(0);
        u32::try_from((numerator + denominator / 2) / denominator)
            .unwrap_or(u32::MAX)
            .max(1)
    }
}

/// How a strike plays its selected region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackMode {
    /// Play the region once, including after key release.
    OneShot,
    /// Play the region once and fade out on key release.
    Gate,
    /// Repeat the region forwards until key release, then fade out.
    Loop,
}

/// Playback settings latched on the next strike.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Playback {
    /// Key and loop behavior.
    pub mode: PlaybackMode,
    /// Inclusive start of the region, in percent (0–99).
    pub start: u8,
    /// Exclusive end of the region, in percent (1–100, greater than start).
    pub end: u8,
    /// Linear fade for the default flat amplitude gate. Zero selects a 64-frame de-click.
    /// A shaped amplitude envelope uses its own ADSR release.
    pub release_ms: u8,
}

impl Playback {
    /// Play the full sample with a gated release.
    pub const INITIAL: Self = Self {
        mode: PlaybackMode::Gate,
        start: 0,
        end: 100,
        release_ms: 0,
    };
}

/// Frame-timed controls for a chromatic sample player.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PitchedMessage {
    /// Choose the clip for the next strike. Out-of-range indices are ignored.
    Select(u8),
    /// Configure the next strike. Invalid trim ranges are ignored.
    Playback(Playback),
    /// Set the selected clip's tone. Edits also affect its sounding voices.
    Timbre(Timbre),
    /// Transport BPM for tempo-tagged clips. Zero selects the recording's own tempo.
    Tempo(u16),
    /// Restart the selected clip at this pitch and strike velocity.
    NoteOn(Note),
    /// Release the matching gated or looped note.
    NoteOff(u8),
    /// Silence all voices immediately, including one-shot tails.
    Stop,
}

/// Four chromatic voices share the bank and stream independently into one mono outlet.
#[derive(Clone, Debug)]
pub struct PitchedSampler<'a> {
    bank: &'a [Clip<'a>],
    selected: &'a Clip<'a>,
    program: Timbre,
    playback: Playback,
    bpm: u16,
    output_hz: u32,
    period: f32,
    voices: [Voice<'a>; POLYPHONY],
    // Oldest strike first. Rotate indices so live decoders and filters stay in place.
    order: [u8; POLYPHONY],
}

#[derive(Clone, Debug)]
struct Voice<'a> {
    clip: &'a Clip<'a>,
    tone: VoiceTone,
    active: Playback,
    start: Cursor<'a>,
    cursor: Cursor<'a>,
    region_frames: u32,
    remaining: u32,
    current: Option<i16>,
    next: Option<i16>,
    step_whole: u32,
    step_fraction: u32,
    fraction: u32,
    gain: i32,
    note: u8,
    held: bool,
    envelope: u32,
    release_step: u32,
}

impl<'a> PitchedSampler<'a> {
    /// Start silent with the first clip selected and gate mode enabled.
    /// An empty bank or zero output rate returns `None`.
    #[must_use]
    pub fn new(bank: &'a [Clip<'a>], output_hz: u32) -> Option<Self> {
        #[allow(
            clippy::cast_precision_loss,
            reason = "Audio sample rates are represented in Hz."
        )]
        let rate = output_hz as f32;
        Self::at_sample_rate(bank, rate)
    }

    /// Use the actual output clock in Hz, including its fractional part.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Validated positive clock, rounded for the original PCM playback and release rates."
    )]
    pub fn at_sample_rate(bank: &'a [Clip<'a>], rate: f32) -> Option<Self> {
        let clip = bank.first()?;
        if !rate.is_finite() || rate < 1.0 {
            return None;
        }
        let output_hz = (rate + 0.5) as u32;
        let period = 1.0 / rate;
        Some(Self {
            bank,
            selected: clip,
            program: Timbre::DRY,
            playback: Playback::INITIAL,
            bpm: 0,
            output_hz,
            period,
            voices: core::array::from_fn(|_| Voice::new(clip, period)),
            order: [0, 1, 2, 3],
        })
    }

    fn apply(&mut self, message: PitchedMessage) {
        match message {
            PitchedMessage::Select(index) => {
                if let Some(clip) = self.bank.get(usize::from(index)) {
                    self.selected = clip;
                }
            }
            PitchedMessage::Playback(p) if p.start < p.end && p.end <= 100 => self.playback = p,
            PitchedMessage::Playback(_) => (),
            PitchedMessage::Tempo(bpm) => {
                self.bpm = bpm;
                for voice in &mut self.voices {
                    if voice.current.is_some() && voice.clip.native_bpm != 0 {
                        voice.update_step(self.output_hz, self.period, bpm);
                    }
                }
            }
            PitchedMessage::Timbre(program) => {
                self.program = program;
                for voice in &mut self.voices {
                    if voice.current.is_some() && core::ptr::eq(self.selected, voice.clip) {
                        voice.tone.configure(program, self.period);
                    }
                }
            }
            PitchedMessage::Stop => {
                for voice in &mut self.voices {
                    voice.current = None;
                }
            }
            PitchedMessage::NoteOff(note)
            | PitchedMessage::NoteOn(Note {
                number: note,
                velocity: 0,
            }) => {
                for voice in &mut self.voices {
                    if voice.current.is_some() && voice.note == note && voice.held {
                        voice.release(self.output_hz);
                    }
                }
            }
            PitchedMessage::NoteOn(note) if note.number < 128 => {
                let rank = self
                    .order
                    .iter()
                    .position(|&i| {
                        let v = &self.voices[usize::from(i)];
                        v.current.is_some() && v.note == note.number
                    })
                    .or_else(|| {
                        self.order
                            .iter()
                            .position(|&i| self.voices[usize::from(i)].current.is_none())
                    })
                    .or_else(|| {
                        self.order
                            .iter()
                            .position(|&i| !self.voices[usize::from(i)].held)
                    })
                    .unwrap_or(0);
                let slot = usize::from(self.order[rank]);
                self.order[rank..].rotate_left(1);
                let voice = &mut self.voices[slot];
                voice.strike(
                    self.selected,
                    note,
                    self.playback,
                    self.program,
                    self.period,
                );
                voice.update_step(self.output_hz, self.period, self.bpm);
            }
            PitchedMessage::NoteOn(_) => (),
        }
    }

    fn render<S: FromSample<i16>>(&mut self, output: &mut [S]) {
        // The decoder and filter run only for voices that actually sound in this span.
        let mut active = self.voices.iter_mut().filter(|v| v.current.is_some());
        let Some(first) = active.next() else {
            output.fill_with(|| S::from_sample_(0));
            return;
        };
        if active.next().is_none() {
            for sample in output {
                *sample = S::from_sample_(first.next_frame());
            }
            return;
        }
        for sample in output {
            let sum: i32 = self
                .voices
                .iter_mut()
                .map(|v| i32::from(v.next_frame()))
                .sum();
            #[allow(
                clippy::cast_possible_truncation,
                reason = "Saturate the wide mix once, after summing four PCM16 voices."
            )]
            let mixed = sum.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            *sample = S::from_sample_(mixed);
        }
    }
}

impl<'a> Voice<'a> {
    fn new(clip: &'a Clip<'a>, period: f32) -> Self {
        Self {
            clip,
            tone: VoiceTone::new(period),
            active: Playback::INITIAL,
            start: clip.pcm.cursor(),
            cursor: clip.pcm.cursor(),
            region_frames: 0,
            remaining: 0,
            current: None,
            next: None,
            step_whole: 0,
            step_fraction: 0,
            fraction: 0,
            gain: 0,
            note: 0,
            held: false,
            envelope: FULL_GAIN,
            release_step: 0,
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::arithmetic_side_effects,
        reason = "Playback speed is nonnegative, fixed-point fractions are less than one and MIDI intervals fit eleven octaves."
    )]
    fn update_step(&mut self, output_hz: u32, period: f32, bpm: u16) {
        let clip = self.clip;
        if clip.native_bpm == 0 {
            let step = clip.step(self.note, output_hz);
            self.step_whole = step >> FRAC_BITS;
            self.step_fraction = step << (32 - FRAC_BITS);
        } else {
            let interval = i32::from(self.note) - i32::from(clip.root);
            let index = usize::try_from(interval.rem_euclid(12)).expect("semitone");
            let octave = interval.div_euclid(12);
            let pitch =
                ((SEMITONES[index] << octave.max(0)) as f32) / ((ONE << (-octave).max(0)) as f32);
            let bpm = if bpm == 0 { clip.native_bpm } else { bpm };
            let speed = (clip.source_hz as f32 * period)
                * (f32::from(bpm) / f32::from(clip.native_bpm))
                * pitch;
            self.step_whole = speed as u32;
            self.step_fraction = ((speed - self.step_whole as f32) * 4_294_967_296.0) as u32;
        }
    }

    fn release(&mut self, output_hz: u32) {
        self.held = false;
        if self.active.mode == PlaybackMode::OneShot {
            return;
        }
        self.tone.release();
        if self.tone.uses_playback_release() {
            let frames = match self.active.release_ms {
                0 => 64,
                ms => u64::from(output_hz)
                    .saturating_mul(u64::from(ms))
                    .div_ceil(1000),
            };
            self.release_step = FULL_GAIN.div_ceil(frames.try_into().unwrap_or(u32::MAX));
        } else if self.tone.finished() {
            self.current = None;
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "Valid trim percentages bound both offsets by the u32 source length."
    )]
    fn strike(
        &mut self,
        clip: &'a Clip<'a>,
        note: Note,
        playback: Playback,
        program: Timbre,
        period: f32,
    ) {
        self.clip = clip;
        self.note = note.number;
        self.held = true;
        self.tone.strike(program, note, clip.root, period);
        self.gain = (i32::from(note.velocity.min(127)) << FRAC_BITS) / 127;
        self.release_step = 0;
        self.envelope = FULL_GAIN;
        self.fraction = 0;
        self.active = playback;
        let frames = u64::from(clip.pcm.frames());
        let begin = (frames * u64::from(playback.start) / 100) as u32;
        let end = (frames * u64::from(playback.end)).div_ceil(100) as u32;
        self.region_frames = end - begin;
        self.start = clip.pcm.cursor();
        if begin != 0 {
            self.start.nth(begin - 1);
        }
        self.cursor = self.start.clone();
        self.remaining = self.region_frames;
        self.current = self.read(0);
        self.next = self.read(0);
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A nonempty loop wraps before subtraction; skip then fits the remaining region."
    )]
    fn read(&mut self, mut skip: u32) -> Option<i16> {
        if skip >= self.remaining {
            if self.active.mode != PlaybackMode::Loop || self.region_frames == 0 {
                self.remaining = 0;
                return None;
            }
            skip = (skip - self.remaining) % self.region_frames;
            self.remaining = self.region_frames;
            self.cursor = self.start.clone();
        }
        self.remaining -= skip + 1;
        self.cursor.nth(skip)
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "The Q15 fraction times any PCM16 difference fits i32. Interpolation and both envelope gains stay within PCM16."
    )]
    fn next_frame(&mut self) -> i16 {
        let Some(first) = self.current else { return 0 };
        let a = i32::from(first);
        let b = i32::from(self.next.unwrap_or(0));
        let value = a + (((b - a) * (self.fraction >> (32 - FRAC_BITS)) as i32) >> FRAC_BITS);
        let mut value = (value * self.gain) >> FRAC_BITS;
        let (phase, carry) = self.fraction.overflowing_add(self.step_fraction);
        let advance = self.step_whole.saturating_add(u32::from(carry));
        self.fraction = phase;
        if advance != 0 {
            self.current = if advance == 1 {
                self.next
            } else {
                self.read(advance - 2)
            };
            self.next = self.read(0);
        }
        if self.release_step != 0 {
            value = (value * (self.envelope >> 9) as i32) >> FRAC_BITS;
            self.envelope = self.envelope.saturating_sub(self.release_step);
            if self.envelope == 0 {
                self.current = None;
            }
        }
        let out = self.tone.process(value as i16);
        if self.tone.finished() {
            self.current = None;
        }
        out
    }
}

impl<S: FromSample<i16>> Process<PitchedMessage, S> for PitchedSampler<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, PitchedMessage, S>) {
        let output = &mut cx.outputs[0];
        let mut start = 0;
        for event in cx.events {
            self.render(&mut output[start..event.frame]);
            self.apply(event.message);
            start = event.frame;
        }
        self.render(&mut output[start..]);
    }
}
