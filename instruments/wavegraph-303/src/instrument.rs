use crate::{Voice, VoiceParams};
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::{I24, Sample, types::i24};

/// Oscillator shapes on the 303's waveform switch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Waveform {
    /// Rising sawtooth.
    Sawtooth,
    /// Pulse wave using the duty cycle in [`VoiceParams::pulse_width`].
    Square,
}

/// Destination of a bipolar modulation message. The stored panel settings remain
/// the center of the sweep, including when automation changes them mid-note.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Destination {
    /// Oscillator pitch, up to two semitones either side of the tuned pitch.
    Pitch,
    /// Filter cutoff, up to three octaves either side of the base cutoff.
    Cutoff,
    /// Filter resonance.
    Resonance,
    /// Filter envelope depth.
    EnvMod,
    /// Filter envelope decay time.
    Decay,
    /// VCA attack time.
    Attack,
    /// Accent amount.
    Accent,
    /// Drive into the ladder filter.
    Overdrive,
}

impl Destination {
    // Called at control boundaries. Keep destination mapping out of the
    // FM-1 SRAM section reserved for the sample renderer.
    #[inline(never)]
    fn apply(self, p: &mut VoiceParams, amount: f32) {
        match self {
            Self::Pitch => p.tuning_st += 2.0 * amount,
            Self::Cutoff => {
                p.cutoff_hz =
                    (p.cutoff_hz * crate::dsp::fast_exp2(3.0 * amount)).clamp(62.0, 8000.0)
            }
            Self::Resonance => p.resonance = (p.resonance + amount).clamp(0.0, 1.0),
            Self::EnvMod => p.env_oct = (p.env_oct + 6.0 * amount).clamp(0.0, 6.0),
            Self::Decay => p.decay_s = (p.decay_s + 1.95 * amount).clamp(0.05, 2.0),
            Self::Attack => p.attack_s = (p.attack_s + 0.060 * amount).clamp(0.0, 0.060),
            Self::Accent => p.accent = (p.accent + amount).clamp(0.0, 1.0),
            Self::Overdrive => p.overdrive_db = (p.overdrive_db + 36.0 * amount).clamp(0.0, 36.0),
        }
    }
}

/// Notes and panel changes for the 303's message inlet.
/// Out-of-range notes and controls are ignored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Message {
    /// Open the gate on a MIDI note in 0..=127.
    NoteOn {
        /// MIDI note number.
        note: u8,
        /// Trigger the accent circuit.
        accent: bool,
        /// Glide from an open gate, carrying the envelopes across the change.
        slide: bool,
    },
    /// Release the gate if the held note has this MIDI note number.
    NoteOff(u8),
    /// Set the filter cutoff in Hz (positive and finite).
    CutoffHz(f32),
    /// Set the filter resonance in 0..=1.
    Resonance(f32),
    /// Set the filter envelope depth in octaves (finite).
    EnvOctaves(f32),
    /// Set the unaccented envelope decay in seconds (nonnegative and finite).
    DecaySeconds(f32),
    /// Set the VCA attack time constant in seconds (nonnegative and finite).
    AttackSeconds(f32),
    /// Set the oscillator overdrive into the resonant filter, in dB (0..=36).
    OverdriveDb(f32),
    /// Set the accent amount in 0..=1.
    Accent(f32),
    /// Set the tuning offset in semitones (finite).
    TuningSemitones(f32),
    /// Replace the current modulation with a bipolar offset in -1..=1.
    /// Zero bypasses modulation on all destinations.
    Modulate(Destination, f32),
    /// Select the oscillator shape using the voice's existing crossfade.
    Waveform(Waveform),
    /// Reset the voice at a transport discontinuity, retaining the panel settings.
    Reset,
}

/// A monophonic 303 instrument with one message inlet and one signal outlet.
/// The outlet supports native `f32` and signed 24-bit PCM ([`I24`]).
#[derive(Debug)]
pub struct Tb303 {
    /// Panel settings and voice calibration. Send [`Message`]s to move panel
    /// controls at a particular frame during playback.
    pub params: VoiceParams,
    modulation: (Destination, f32),
    voice: Voice,
    held_note: Option<u8>,
    last_cycle_start: Option<usize>,
    suspend_idle: bool,
    output_gain: f32,
}

impl Tb303 {
    /// Create a silent instrument for the host's physical sample rate in Hz.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            params: VoiceParams::default(),
            modulation: (Destination::Pitch, 0.0),
            voice: Voice::new(sample_rate),
            held_note: None,
            last_cycle_start: None,
            suspend_idle: false,
            output_gain: 1.0,
        }
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::NoteOn {
                note,
                accent,
                slide,
            } if note <= 127 => {
                self.voice.note_on(i32::from(note), accent, slide);
                self.held_note = Some(note);
            }
            Message::NoteOff(note) if self.held_note == Some(note) => {
                self.voice.note_off();
                self.held_note = None;
            }
            Message::CutoffHz(hz) if hz > 0.0 && hz.is_finite() => self.params.cutoff_hz = hz,
            Message::Resonance(value) if (0.0..=1.0).contains(&value) => {
                self.params.resonance = value
            }
            Message::EnvOctaves(octaves) if octaves.is_finite() => self.params.env_oct = octaves,
            Message::DecaySeconds(seconds) if seconds >= 0.0 && seconds.is_finite() => {
                self.params.decay_s = seconds
            }
            Message::AttackSeconds(seconds) if seconds >= 0.0 && seconds.is_finite() => {
                self.params.attack_s = seconds
            }
            Message::OverdriveDb(db) if (0.0..=36.0).contains(&db) => self.params.overdrive_db = db,
            Message::Accent(value) if (0.0..=1.0).contains(&value) => self.params.accent = value,
            Message::TuningSemitones(semitones) if semitones.is_finite() => {
                self.params.tuning_st = semitones
            }
            Message::Modulate(target, value) if (-1.0..=1.0).contains(&value) => {
                self.modulation = (target, value);
            }
            Message::Waveform(shape) => self.params.square = shape == Waveform::Square,
            Message::Reset => {
                self.voice.reset();
                self.held_note = None;
            }
            _ => {}
        }
    }

    /// Let the oscillator and ladder sleep once the released VCA is below the Q23 floor.
    /// The accent circuit continues to discharge between notes.
    #[must_use]
    pub fn with_idle_suspension(mut self) -> Self {
        self.suspend_idle = true;
        self
    }

    /// Attenuate the voice after its filter and overdrive, before output conversion.
    ///
    /// The default gain is `1.0`. A smaller gain reserves mixing headroom and
    /// avoids clipping a hot resonant voice at the [`I24`] rails; it leaves the
    /// filter's drive and internal signal unchanged. The same gain applies to
    /// native `f32` output. A gain of `0.0` mutes the output while the voice runs.
    ///
    /// # Panics
    /// Panics if `gain` is not finite or is outside `0.0..=1.0`.
    #[must_use]
    pub fn with_output_gain(mut self, gain: f32) -> Self {
        assert!(
            (0.0..=1.0).contains(&gain),
            "303 output gain must be finite and in 0.0..=1.0"
        );
        self.output_gain = gain;
        self
    }

    /// Zero-based frame of the last oscillator wrap in the most recent process block,
    /// or `None` if the oscillator did not wrap.
    /// A scope can trigger here while displaying the filtered signal, including during slides.
    #[must_use]
    pub fn last_cycle_start(&self) -> Option<usize> {
        self.last_cycle_start
    }

    // SAFETY: the FM-1 linker places .data in executable SRAM and startup copies it before use.
    #[cfg_attr(
        all(target_os = "none", feature = "fm1-ram-code"),
        allow(unsafe_code),
        unsafe(link_section = ".data.wavegraph_audio")
    )]
    fn render<S: OutputSample>(&mut self, output: &mut [S], start: usize) {
        if output.is_empty() {
            return;
        }
        let mut effective;
        let (target, amount) = self.modulation;
        let params = if amount == 0.0 {
            &self.params
        } else {
            effective = self.params;
            target.apply(&mut effective, amount);
            &effective
        };
        if self.suspend_idle && self.voice.skip_silence(params, output.len()) {
            for sample in output {
                S::write(sample, 0.0);
            }
        } else {
            let gain = self.output_gain;
            let cycle = self.voice.render_with(params, output, |sample, value| {
                S::write(sample, value * gain);
            });
            if let Some(frame) = cycle {
                self.last_cycle_start = start.checked_add(frame);
            }
        }
    }
}

impl<S: OutputSample> Process<Message, S> for Tb303 {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, S>) {
        let output = &mut cx.outputs[0];
        self.last_cycle_start = None;
        let mut start = 0;
        for event in cx.events {
            self.render(&mut output[start..event.frame], start);
            self.apply(event.message);
            start = event.frame;
        }
        self.render(&mut output[start..], start);
    }
}

trait OutputSample {
    fn write(&mut self, sample: f32);
}

impl OutputSample for f32 {
    fn write(&mut self, sample: f32) {
        *self = sample;
    }
}

impl OutputSample for I24 {
    fn write(&mut self, sample: f32) {
        // +1.0 is one count beyond the positive Q23 rail.
        *self = sample.clamp(-1.0, i24::MAX.to_sample::<f32>()).to_sample();
    }
}
