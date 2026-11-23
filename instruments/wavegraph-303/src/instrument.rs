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
    voice: Voice,
    held_note: Option<u8>,
    suspend_idle: bool,
    output_gain: f32,
}

impl Tb303 {
    /// Create a silent instrument for the host's physical sample rate in Hz.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            params: VoiceParams::default(),
            voice: Voice::new(sample_rate),
            held_note: None,
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

    fn render<S: OutputSample>(&mut self, output: &mut [S]) {
        if output.is_empty() {
            return;
        }
        if self.suspend_idle && self.voice.skip_silence(&self.params, output.len()) {
            for sample in output {
                S::write(sample, 0.0);
            }
        } else {
            let gain = self.output_gain;
            self.voice
                .render_with(&self.params, output, |sample, value| {
                    S::write(sample, value * gain);
                });
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
        let mut start = 0;
        for event in cx.events {
            self.render(&mut output[start..event.frame]);
            self.apply(event.message);
            start = event.frame;
        }
        self.render(&mut output[start..]);
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
