#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod envelope;
mod voice;
use envelope::Timing;
use voice::{Frame, Modulation, Tone, Voice};
use wavegraph_core::{Event, Port, Process, ProcessContext, midi::Note, polyphony::Keyboard};
pub use wavegraph_core::{Tuning, polyphony::VoiceMode};
use wavegraph_fx::{Chorus, ChorusControls};
use wavegraph_sample::FromSample;

/// Number of simultaneous oscillator and envelope pairs.
pub const POLYPHONY: usize = 4;

/// Parameters use the Alpha's 0..127 panel scale. Segment times and filter response are approximations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Patch {
    /// Times from gate-on to L1, L2 and L3, then from key-up to silence.
    pub times: [u8; 4],
    /// Independent targets for the first three stages. L3 holds until key-up.
    pub levels: [u8; 3],
    /// Alpha pulse waveform selector (0 off, 1 square, 2 narrow, 3 PWM).
    pub pulse_wave: u8,
    /// Alpha saw waveform selector (0 off, 1 plain, 2–5 chopped).
    pub saw_wave: u8,
    /// Alpha sub waveform selector (0–3 one octave down, 4–5 two down).
    pub sub_wave: u8,
    /// Sub oscillator level (0–3).
    pub sub_level: u8,
    /// Four-pole low-pass feedback (0–127).
    pub resonance: u8,
    /// Pre-filter bass boost, flat, low cut or higher cut (0–3).
    pub hpf: u8,
    /// Low-pass envelope depth (0–127).
    pub filter_env: u8,
    /// Pulse-width modulation speed. Zero selects static pulse width.
    pub pwm_rate: u8,
    /// Noise level (0–3).
    pub noise: u8,
    /// Base low-pass cutoff.
    pub cutoff: u8,
    /// Positive pitch sweep depth. At full depth the attack starts two octaves low.
    pub sweep: u8,
    /// Envelope level at the played pitch (64–127). Higher levels overshoot the key.
    pub pitch_reference: u8,
    /// Common pulse and chopped-saw PWM depth.
    pub pwm: u8,
    /// Ensemble speed. Zero bypasses both wet paths.
    pub chorus: u8,
}
impl Patch {
    /// Fast-playing interpretation of the factory What the tone.
    pub const HOOVER: Self = Self {
        times: [10, 22, 38, 23],
        levels: [68, 127, 127],
        pulse_wave: 3,
        saw_wave: 3,
        sub_wave: 5,
        sub_level: 3,
        resonance: 0,
        hpf: 0,
        filter_env: 75,
        pwm_rate: 102,
        noise: 1,
        cutoff: 77,
        sweep: 127,
        pitch_reference: 127,
        pwm: 127,
        chorus: 92,
    };
    /// Rave lead with a low wind-up, a fast upward sweep and a long key-up glide.
    pub const DOMINATOR: Self = Self {
        times: [20, 29, 49, 40],
        levels: [24, 127, 87],
        pitch_reference: 87,
        pulse_wave: 0,
        cutoff: 72,
        ..Self::HOOVER
    };
    /// Dominator, Fast Hoover, the factory envelope comparison and a two-peak poly pad.
    pub const PRESETS: [(&'static str, Self); 4] = [
        ("DOMINATOR", Self::DOMINATOR),
        ("FAST HOOVER", Self::HOOVER),
        (
            "WHAT THE",
            Self {
                times: [99, 78, 75, 92],
                ..Self::HOOVER
            },
        ),
        (
            "TWIN PEAK",
            Self {
                times: [25, 51, 64, 55],
                levels: [117, 32, 103],
                sweep: 18,
                pitch_reference: 127,
                pwm: 91,
                chorus: 54,
                cutoff: 90,
                pulse_wave: 1,
                saw_wave: 1,
                sub_wave: 0,
                sub_level: 1,
                resonance: 45,
                hpf: 1,
                filter_env: 100,
                noise: 0,
                pwm_rate: 42,
            },
        ),
    ];

    fn chorus_controls(self) -> ChorusControls {
        ChorusControls {
            rate: self.chorus,
            enabled: self.chorus != 0,
            ..ChorusControls::MKS50
        }
    }
}

/// Sample-offset controls for inlet zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    /// Start a note, or release it at velocity zero.
    NoteOn {
        /// MIDI note number.
        note: u8,
        /// MIDI strike velocity.
        velocity: u8,
    },
    /// Release this note from its current envelope level.
    NoteOff(u8),
    /// Apply a patch. Segment durations and targets latch on segment entry.
    Patch(Patch),
    /// Select polyphonic, unison or chord allocation.
    VoiceMode(VoiceMode),
}

/// Four DCO voices and a shared stereo chorus.
#[derive(Debug)]
pub struct Hoover<'a> {
    tuning: &'a Tuning,
    keyboard: Keyboard,
    voices: [Voice; POLYPHONY],
    timing: Timing,
    modulation: Modulation,
    tone: Tone,
    chorus: Chorus,
}
impl<'a> Hoover<'a> {
    /// Start silent with the Dominator patch selected.
    #[must_use]
    pub fn new(tuning: &'a Tuning) -> Self {
        let patch = Patch::DOMINATOR;
        let period = tuning.sample_period();
        let mut chorus = Chorus::mks50(period);
        chorus.set_controls(patch.chorus_controls());
        Self {
            tuning,
            keyboard: Keyboard::EMPTY,
            voices: [Voice::EMPTY; POLYPHONY],
            timing: Timing::new(patch.times, patch.levels, period),
            modulation: Modulation::RESET,
            tone: Tone::new(patch, period),
            chorus,
        }
    }
    fn set_patch(&mut self, p: Patch) {
        let period = self.tuning.sample_period();
        self.timing = Timing::new(p.times, p.levels, period);
        self.tone = Tone::new(p, period);
        self.chorus.set_controls(p.chorus_controls());
    }
    fn strike(&mut self, i: usize, note: Note, step: u32) {
        self.voices[i] = Voice::strike(note, step, &self.timing, self.keyboard.mode);
    }
    fn revoice(&mut self) {
        for i in 0..POLYPHONY {
            if let Some(note) = self.keyboard.voice(i) {
                let step = self.keyboard.step(i, self.tuning.phase_step(note.number));
                if step != 0 {
                    self.strike(i, note, step);
                    continue;
                }
            }
            self.voices[i].env.release(&self.timing);
        }
    }

    /// Apply one control at the current frame.
    pub fn apply(&mut self, message: Message) {
        match message {
            Message::Patch(patch) => self.set_patch(patch),
            Message::VoiceMode(mode) => {
                if self.keyboard.mode != mode {
                    self.keyboard.mode = mode;
                    self.revoice();
                }
            }
            Message::NoteOff(note) | Message::NoteOn { note, velocity: 0 } => {
                let newest = self.keyboard.release(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    for v in &mut self.voices {
                        if v.note.number == note {
                            v.env.release(&self.timing);
                        }
                    }
                } else if newest {
                    self.revoice();
                }
            }
            Message::NoteOn { note, velocity } => {
                let step = self.tuning.phase_step(note);
                if step == 0 {
                    return;
                }
                let note = Note {
                    number: note,
                    velocity: velocity.min(127),
                };
                self.keyboard.press(note);
                if self.keyboard.mode == VoiceMode::Poly {
                    let slot = self
                        .voices
                        .iter()
                        .position(|v| !v.env.idle() && v.note.number == note.number)
                        .or_else(|| self.voices.iter().position(|v| v.env.idle()))
                        .or_else(|| self.voices.iter().rposition(|v| v.env.releasing()))
                        .unwrap_or(POLYPHONY - 1);
                    self.voices[..=slot].rotate_right(1);
                    self.strike(0, note, step);
                } else {
                    self.revoice();
                }
            }
        }
    }

    /// Render one stereo frame, including chorus tails after all keys are up.
    pub fn stereo_frame(&mut self) -> [f32; 2] {
        let frame = self.modulation.frame(&self.tone);
        let mut dry = 0.0;
        for v in &mut self.voices {
            v.mix(
                core::slice::from_ref(&frame),
                core::slice::from_mut(&mut dry),
                &self.tone,
                &self.timing,
            );
        }
        self.chorus.stereo_frame(dry)
    }

    /// Render the mono-compatible sum and the stereo difference of the ensemble.
    /// Add the side to the mid for the left return and subtract it for the right.
    ///
    /// # Panics
    /// The two outputs must have equal lengths. Events must be ordered by frame
    /// and fall within the block (including its end).
    pub fn process_mid_side<S: FromSample<f32>>(
        &mut self,
        mid: &mut [S],
        side: &mut [S],
        events: &[Event<Message>],
    ) {
        assert_eq!(mid.len(), side.len());
        self.render_events(mid, Some(side), events);
    }

    fn render_events<S: FromSample<f32>>(
        &mut self,
        mid: &mut [S],
        mut side: Option<&mut [S]>,
        events: &[Event<Message>],
    ) {
        let mut start = 0;
        for event in events {
            let range = start..event.frame;
            self.render(
                &mut mid[range.clone()],
                side.as_deref_mut().map(|s| &mut s[range]),
            );
            self.apply(event.message);
            start = event.frame;
        }
        self.render(&mut mid[start..], side.map(|s| &mut s[start..]));
    }

    fn render<S: FromSample<f32>>(&mut self, out: &mut [S], side: Option<&mut [S]>) {
        if out.is_empty() {
            return;
        }
        if self.voices.iter().all(|v| v.env.idle()) {
            self.modulation.advance(out.len(), &self.tone);
            if let Some(side) = side {
                self.chorus.mid_side_silence(out, side);
            } else {
                self.chorus.mono_silence(out);
            }
            return;
        }
        // Render each voice across a short span with its DCO and filter state hot.
        // All four voices see exactly the same free-running PWM and noise.
        const SPAN: usize = 32;
        let mut frames = [Frame::ZERO; SPAN];
        let mut dry = [0.0; SPAN];
        let mut side_chunks = side.map(|s| s.chunks_mut(SPAN));
        for dst in out.chunks_mut(SPAN) {
            let frames = &mut frames[..dst.len()];
            let dry = &mut dry[..dst.len()];
            for frame in &mut *frames {
                *frame = self.modulation.frame(&self.tone);
            }
            dry.fill(0.0);
            for v in &mut self.voices {
                v.mix(frames, dry, &self.tone, &self.timing);
            }
            if let Some(side) = side_chunks.as_mut().and_then(Iterator::next) {
                self.chorus.mid_side_block(dry, dst, side);
            } else {
                self.chorus.mono_block(dry, dst);
            }
        }
    }
}
impl<S: FromSample<f32>> Process<Message, S> for Hoover<'_> {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, S>) {
        self.render_events(cx.outputs[0], None, cx.events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fm1_pcm_outlet_folds_the_stereo_chorus_at_the_event_offsets() {
        use wavegraph_core::Event;
        use wavegraph_sample::I24;

        let tuning = Tuning::new(24_000_000, 544).expect("FM-1 sample clock");
        let mut mono = Hoover::new(&tuning);
        let mut stereo = Hoover::new(&tuning);
        let at = |frame, message| Event {
            frame,
            port: 0,
            message,
        };
        let events = [
            at(
                17,
                Message::NoteOn {
                    note: 55,
                    velocity: 112,
                },
            ),
            at(383, Message::NoteOff(55)),
            at(
                640,
                Message::Patch(Patch {
                    chorus: 0,
                    ..Patch::HOOVER
                }),
            ),
        ];
        let mut pcm = [I24::default(); 768];
        mono.process(&mut ProcessContext {
            frames: pcm.len(),
            inputs: &[],
            outputs: &mut [&mut pcm],
            events: &events,
            emit: &mut Err,
        });
        for (i, sample) in pcm.into_iter().enumerate() {
            for event in events.iter().filter(|e| e.frame == i) {
                stereo.apply(event.message);
            }
            let [left, right] = stereo.stereo_frame();
            if i >= 640 {
                assert_eq!(left, right);
            }
            assert_eq!(sample, I24::from_sample_((left + right) * 0.5));
        }
        assert!(pcm[..17].iter().all(|s| s.inner() == 0));
        assert!(pcm[383..640].iter().any(|s| s.inner() != 0));
    }

    #[test]
    fn mid_side_returns_follow_the_same_stereo_patch_and_events() {
        let tuning = Tuning::new(24_000_000, 544).expect("FM-1 sample clock");
        let mut block = Hoover::new(&tuning);
        let mut scalar = Hoover::new(&tuning);
        let at = |frame, message| Event {
            frame,
            port: 0,
            message,
        };
        let events = [
            at(
                0,
                Message::NoteOn {
                    note: 62,
                    velocity: 120,
                },
            ),
            at(301, Message::NoteOff(62)),
            at(
                523,
                Message::Patch(Patch {
                    chorus: 0,
                    ..Patch::DOMINATOR
                }),
            ),
            at(587, Message::Patch(Patch::DOMINATOR)),
        ];
        let mut mid = [0.0; 800];
        let mut side = [0.0; 800];
        block.process_mid_side(&mut mid, &mut side, &events);
        for i in 0..mid.len() {
            for event in events.iter().filter(|e| e.frame == i) {
                scalar.apply(event.message);
            }
            let [l, r] = scalar.stereo_frame();
            assert_eq!(mid[i], (l + r) * 0.5);
            assert_eq!(side[i], (l - r) * 0.5);
        }
        assert!(side[300..523].iter().any(|&x| x != 0.0));
        assert!(side[523..587].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn silent_spans_preserve_free_running_pwm_noise_and_chorus_on_restrike() {
        let tuning = Tuning::new(24_000_000, 544).expect("FM-1 sample clock");
        let mut block = Hoover::new(&tuning);
        let mut scalar = Hoover::new(&tuning);
        let note = Message::NoteOn {
            note: 58,
            velocity: 117,
        };
        let mut compare = |event: Option<Message>, n: usize| {
            if let Some(message) = event {
                block.apply(message);
                scalar.apply(message);
            }
            let mut pcm = [0.0_f32; 257];
            block.render(&mut pcm[..n], None);
            for &sample in &pcm[..n] {
                let [l, r] = scalar.stereo_frame();
                assert_eq!(sample, (l + r) * 0.5);
            }
        };
        let patch = Patch {
            times: [3, 7, 11, 4],
            ..Patch::HOOVER
        };
        compare(Some(Message::Patch(patch)), 73);
        compare(Some(note), 255);
        compare(Some(Message::NoteOff(58)), 7);
        for n in [31, 32, 33, 127, 128, 129, 256, 257, 0] {
            compare(None, n);
        }
        compare(Some(note), 257);
    }

    #[test]
    fn four_note_chord_has_stereo_motion_and_releases_to_silence() {
        let tuning = Tuning::new(44_118, 1).expect("valid clock");
        let mut synth = Hoover::new(&tuning);
        for note in [48, 55, 58, 63] {
            synth.apply(Message::NoteOn {
                note,
                velocity: 127,
            });
        }
        let mut spread = 0.0;
        for _ in 0..8192 {
            let [l, r] = synth.stereo_frame();
            assert!(l.abs() < 1.0 && r.abs() < 1.0);
            spread += (l - r).abs();
        }
        assert!(spread > 1.0);
        for note in [48, 55, 58, 63] {
            synth.apply(Message::NoteOff(note));
        }
        for _ in 0..4096 {
            synth.stereo_frame();
        }
        assert!((0..128).any(|_| synth.stereo_frame() != [0.0; 2]));
        for _ in 0..8192 {
            synth.stereo_frame();
        }
        assert_eq!(synth.stereo_frame(), [0.0; 2]);
    }
}
