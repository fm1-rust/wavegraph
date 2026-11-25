#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub use wavegraph_adsr::Adsr;
pub use wavegraph_core::polyphony::{Chord, VoiceMode};
use wavegraph_core::{midi::Note, polyphony::Keyboard};

use wavegraph_adsr::{Envelope, Timing};
use wavegraph_core::{Port, Process, ProcessContext, sine_q15};
use wavegraph_sample::FromSample;

/// Maximum simultaneous notes, including voices in their release tail.
pub const POLYPHONY: usize = 4;

/// VCA settings for the dry house patch.
pub const DEFAULT_ENVELOPE: Adsr = Adsr {
    attack_us: 2_000,
    decay_us: 100_000,
    sustain_percent: 100,
    release_us: Params::INITIAL.release_ms as u32 * 1_000,
};

// Q16.16 key transient with Q15 peak amplitude.
const FULL: u32 = 32767 << 16;

/// Two oscillator recipes based on independent M1 Organ 2 recreation techniques.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Voicing {
    /// Additive 1:3:6 harmonic ratio, a common FM/VA house-organ recipe.
    #[default]
    House,
    /// Triangle/saw bottom, triangle two octaves up, and a square three up.
    Analog,
}

/// Controls for a dry, M1-inspired house organ. This is *not* the M1 PCM ROM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Params {
    /// Oscillator harmonic blend.
    pub voicing: Voicing,
    /// Amount of the bright upper harmonic, 0..=127.
    pub tone: u8,
    /// Short, high-frequency key transient, 0..=127.
    pub percussion: u8,
    /// Time for the transient to fade, in milliseconds (5..=500).
    pub decay_ms: u16,
    /// VCA release in whole milliseconds, saturating at `u16::MAX`.
    /// [`Organ::vca_envelope`] reports the exact time in microseconds.
    pub release_ms: u16,
}

impl Params {
    /// Initial dry organ patch, tuned for short house stabs and bass riffs.
    pub const INITIAL: Self = Self {
        voicing: Voicing::House,
        tone: 80,
        percussion: 44,
        decay_ms: 70,
        release_ms: 55,
    };
}

impl Default for Params {
    fn default() -> Self {
        Self::INITIAL
    }
}

/// One message inlet accepting note and panel changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Change voice allocation, immediately revoicing any held keys.
    VoiceMode(VoiceMode),
    /// Note-on; zero velocity means note-off, as with MIDI.
    NoteOn {
        /// MIDI note number in 0..=127.
        note: u8,
        /// Strike velocity (0 is note-off, other values clamp to 127).
        velocity: u8,
    },
    /// Release only the matching held note; stale note-offs are ignored.
    NoteOff(u8),
    /// Switch between two modeled harmonic recipes.
    Voicing(Voicing),
    /// Upper-partial balance (0..=127).
    Tone(u8),
    /// Percussive transient level (0..=127).
    Percussion(u8),
    /// Transient decay time in milliseconds, clamped to 5..=500.
    DecayMs(u16),
    /// Set the amplitude envelope for all voices, including held notes and tails.
    VcaEnvelope(Adsr),
    /// Set only VCA release in milliseconds, clamped to 0..=1000.
    ReleaseMs(u16),
    /// Release every held key while preserving the release tails.
    AllNotesOff,
    /// Silence all voices immediately, retaining panel settings.
    Reset,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    note: Option<u8>,
    age: u32,
    velocity: u8,
    phase: u32,
    step: u32,
    envelope: Envelope,
    transient: u32,
}

impl Voice {
    const SILENT: Self = Self {
        note: None,
        age: 0,
        velocity: 0,
        phase: 0,
        step: 0,
        envelope: Envelope::IDLE,
        transient: 0,
    };

    fn release(&mut self, timing: &Timing) {
        self.envelope.gate_off(timing);
        if self.envelope.is_idle() {
            self.note = None;
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "Gain is in 0..=1 and the Q15 mixer intermediates fit i32."
    )]
    fn next(&mut self, params: Params, timing: &Timing, decay_step: u32) -> i32 {
        if self.note.is_none() {
            return 0;
        }
        let gain = self.envelope.advance(timing);
        if self.envelope.is_idle() {
            self.note = None;
            return 0;
        }

        // The M1 uses an organ multisample, not drawbars. Offer two modeled
        // alternatives to audition rather than claiming either is exact:
        // a 1:3:6 additive spectrum and MusicRadar's 1:4:8 oscillator recipe.
        // Omit upper partials beyond Nyquist instead of wrapping their phases.
        let low = match params.voicing {
            Voicing::House => triangle(self.phase),
            Voicing::Analog => {
                let saw = i32::from((self.phase >> 16) as u16) - 32768;
                (3 * triangle(self.phase) + saw) >> 2
            }
        };
        let (mid, high) = match params.voicing {
            Voicing::House => (
                if self.step < 715_827_882 {
                    sine_q15(self.phase.wrapping_mul(3))
                } else {
                    0
                },
                if self.step < 357_913_941 {
                    sine_q15(self.phase.wrapping_mul(6))
                } else {
                    0
                },
            ),
            Voicing::Analog => (
                if self.step < 536_870_912 {
                    triangle(self.phase.wrapping_mul(4))
                } else {
                    0
                },
                if self.step < 268_435_456 {
                    if self.phase.wrapping_mul(8) < 0x8000_0000 {
                        32767
                    } else {
                        -32767
                    }
                } else {
                    0
                },
            ),
        };
        let body = (low * 18 + mid * 8 + (high * i32::from(params.tone) * 7 / 127)) >> 5;

        // No random generator or PCM sample required: the short click is a
        // high-frequency pulse whose amplitude falls with the key transient.
        let pulse = if self.phase.wrapping_mul(16) < 0x8000_0000 {
            32767_i32
        } else {
            -32767_i32
        };
        let click_gain = (i32::from(params.percussion) * (self.transient >> 16) as i32) >> 7;
        let click = (pulse * click_gain) >> 18;
        self.transient = self.transient.saturating_sub(decay_step);
        self.phase = self.phase.wrapping_add(self.step);

        let struck = ((body + click) * i32::from(self.velocity)) >> 7;
        (struck * (gain * 32767.0) as i32) >> 15
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "16-bit ramp intermediates fit i32"
)]
fn triangle(phase: u32) -> i32 {
    let ramp = i32::from((phase >> 16) as u16);
    32767 - (2 * ramp - 65535).abs()
}

/// A fixed four-voice, dry house organ with one message inlet and one signal
/// outlet.
/// Voices are reused on re-strike, then allocated from silence, then from
/// oldest released voices, and finally stolen from the oldest held note.
#[derive(Clone, Debug)]
#[allow(
    missing_copy_implementations,
    reason = "Avoid accidental copies of mutable instrument state"
)]
pub struct Organ {
    steps: [u32; 128],
    keyboard: Keyboard,
    voices: [Voice; POLYPHONY],
    params: Params,
    voice_level: u8,
    serial: u32,
    rate_num: u32,
    rate_den: u32,
    decay_step: u32,
    sample_period: f32,
    vca_envelope: Adsr,
    timing: Timing,
}

impl Organ {
    /// Construct for `numerator / denominator` frames/s (e.g. 24 MHz / 544).
    /// Returns `None` for a zero clock. Notes at or above Nyquist are ignored.
    #[must_use]
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_lossless,
        clippy::cast_precision_loss,
        reason = "The positive phase increments are bounded by Nyquist."
    )]
    pub const fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        let mut steps = [0; 128];
        let mut step =
            8.175_798_915_643_707 * 4_294_967_296.0 * denominator as f64 / numerator as f64;
        let mut i = 0;
        while i < steps.len() {
            if step < 2_147_483_648.0 {
                steps[i] = (step + 0.5) as u32;
            }
            step *= 1.059_463_094_359_295_3;
            i += 1;
        }
        let params = Params::INITIAL;
        let sample_period = (denominator as f64 / numerator as f64) as f32;
        let mut organ = Self {
            steps,
            keyboard: Keyboard::EMPTY,
            voices: [Voice::SILENT; POLYPHONY],
            params,
            voice_level: 8,
            serial: 0,
            rate_num: numerator,
            rate_den: denominator,
            decay_step: FULL,
            sample_period,
            vca_envelope: DEFAULT_ENVELOPE,
            timing: DEFAULT_ENVELOPE.at_sample_period(sample_period),
        };
        organ.decay_step = organ.ramp_step(params.decay_ms);
        Some(organ)
    }

    /// Set the per-voice gain in units of 1/128. The default is 8 (about
    /// −24 dB). Levels above 128 select unity. The polyphonic sum saturates
    /// at the output rails.
    #[must_use]
    pub const fn with_voice_level(mut self, level: u8) -> Self {
        self.voice_level = if level > 128 { 128 } else { level };
        self
    }

    /// The current panel settings.
    #[must_use]
    pub const fn params(&self) -> Params {
        self.params
    }

    /// The VCA settings shared by all four voices.
    #[must_use]
    pub const fn vca_envelope(&self) -> Adsr {
        self.vca_envelope
    }

    /// The number of occupied voices, including held keys at zero sustain and release tails.
    #[must_use]
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.note.is_some()).count()
    }

    /// The held note in each of the four voice slots (release tails are `None`).
    #[must_use]
    pub fn held_notes(&self) -> [Option<u8>; POLYPHONY] {
        self.voices
            .each_ref()
            .map(|v| if v.envelope.is_gated() { v.note } else { None })
    }

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_lossless,
        clippy::cast_possible_truncation,
        reason = "Products fit u64; the frame count is clamped to u32."
    )]
    const fn frames(&self, ms: u16) -> u32 {
        if ms == 0 {
            return 1;
        }
        let n = self.rate_num as u64 * ms as u64;
        let d = self.rate_den as u64 * 1000;
        let frames = n.div_ceil(d);
        if frames == 0 {
            1
        } else if frames > u32::MAX as u64 {
            u32::MAX
        } else {
            frames as u32
        }
    }

    #[allow(clippy::arithmetic_side_effects, reason = "Nonzero frame count")]
    const fn ramp_step(&self, ms: u16) -> u32 {
        let frames = self.frames(ms);
        let step = FULL / frames;
        if step == 0 { 1 } else { step }
    }

    fn allocate(&self, note: u8) -> usize {
        if let Some(i) = self.voices.iter().position(|v| v.note == Some(note)) {
            return i;
        }
        if let Some(i) = self.voices.iter().position(|v| v.note.is_none()) {
            return i;
        }
        let released = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.envelope.is_gated())
            .max_by_key(|(_, v)| self.serial.wrapping_sub(v.age));
        if let Some((i, _)) = released {
            return i;
        }
        self.voices
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| self.serial.wrapping_sub(v.age))
            .map_or(0, |(i, _)| i)
    }

    fn set_vca_envelope(&mut self, adsr: Adsr) {
        let adsr = Adsr {
            sustain_percent: adsr.sustain_percent.min(100),
            ..adsr
        };
        let timing = adsr.at_sample_period(self.sample_period);
        for voice in &mut self.voices {
            voice.envelope.retime(&self.timing, &timing);
        }
        self.timing = timing;
        self.vca_envelope = adsr;
        self.params.release_ms = u16::try_from(adsr.release_us / 1_000).unwrap_or(u16::MAX);
    }

    fn step(&self, note: u8) -> u32 {
        self.steps.get(usize::from(note)).copied().unwrap_or(0)
    }

    fn release_slot(&mut self, i: usize) {
        self.voices[i].release(&self.timing);
    }

    fn strike(&mut self, i: usize, note: Note, step: u32) {
        self.serial = self.serial.wrapping_add(1);
        self.voices[i] = Voice {
            note: Some(note.number),
            age: self.serial,
            velocity: note.velocity.min(127),
            step,
            transient: FULL,
            ..Voice::SILENT
        };
        self.voices[i].envelope.gate_on(&self.timing);
    }

    fn revoice(&mut self) {
        // Oldest first preserves the organ's age order and slot zero is the newest key.
        for i in (0..POLYPHONY).rev() {
            if let Some(note) = self.keyboard.voice(i) {
                let step = self.keyboard.step(i, self.step(note.number));
                if step != 0 {
                    self.strike(i, note, step);
                    continue;
                }
            }
            if self.voices[i].envelope.is_gated() {
                self.release_slot(i);
            }
        }
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::VoiceMode(mode) => {
                if self.keyboard.mode != mode {
                    self.keyboard.mode = mode;
                    self.revoice();
                }
            }
            Message::NoteOff(note) | Message::NoteOn { note, velocity: 0 } => {
                let newest = self.keyboard.release(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    for i in 0..POLYPHONY {
                        if self.voices[i].note == Some(note) && self.voices[i].envelope.is_gated() {
                            self.release_slot(i);
                        }
                    }
                } else if newest {
                    self.revoice();
                }
            }
            Message::NoteOn { note, velocity } => {
                let step = self.step(note);
                if step == 0 {
                    return;
                }
                let note = Note {
                    number: note,
                    velocity,
                };
                self.keyboard.press(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    self.strike(self.allocate(note.number), note, step);
                } else {
                    self.revoice();
                }
            }
            Message::Voicing(voicing) => self.params.voicing = voicing,
            Message::Tone(tone) => self.params.tone = tone.min(127),
            Message::Percussion(punch) => self.params.percussion = punch.min(127),
            Message::DecayMs(ms) => {
                self.params.decay_ms = ms.clamp(5, 500);
                self.decay_step = self.ramp_step(self.params.decay_ms);
            }
            Message::VcaEnvelope(adsr) => self.set_vca_envelope(adsr),
            Message::ReleaseMs(ms) => self.set_vca_envelope(Adsr {
                release_us: u32::from(ms.min(1000)) * 1_000,
                ..self.vca_envelope
            }),
            Message::AllNotesOff => {
                self.keyboard.clear();
                for voice in &mut self.voices {
                    voice.release(&self.timing);
                }
            }
            Message::Reset => {
                self.keyboard.clear();
                self.voices = [Voice::SILENT; POLYPHONY];
            }
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Four bounded 16-bit voices in Q31 cannot overflow i32."
    )]
    fn render<S: FromSample<i32>>(&mut self, output: &mut [S]) {
        if self.voices.iter().all(|voice| voice.note.is_none()) {
            output.fill_with(|| S::from_sample_(0));
            return;
        }
        // Convert the Q15 voice sum to Q31 and apply the voice level in one multiply.
        let scale = i32::from(self.voice_level) << 9;
        for sample in output {
            let sum: i32 = self
                .voices
                .iter_mut()
                .map(|v| v.next(self.params, &self.timing, self.decay_step))
                .sum();
            *sample = S::from_sample_(sum.saturating_mul(scale));
        }
    }
}

impl<S: FromSample<i32>> Process<Message, S> for Organ {
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
            let frame = event.frame.min(output.len());
            if frame < start {
                continue;
            }
            self.render(&mut output[start..frame]);
            self.apply(event.message);
            start = frame;
        }
        self.render(&mut output[start..]);
    }
}
