#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod envelope;
use envelope::{Envelope, Timing};
use wavegraph_core::{Port, Process, ProcessContext, midi::Note, polyphony::Keyboard};
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
    /// Positive pitch sweep depth. Full depth spans two octaves.
    pub sweep: u8,
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
        pwm: 127,
        chorus: 92,
    };
    /// Playable rave lead, factory envelope comparison and a two-peak poly pad.
    pub const PRESETS: [(&'static str, Self); 3] = [
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

fn triangle(phase: u32) -> f32 {
    let hi = (phase >> 16) as u16;
    let folded = if hi < 32768 { hi } else { !hi };
    f32::from(folded) / 16384.0 - 1.0
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    note: Note,
    phase: u32,
    step: u32,
    env: Envelope,
    filter: [f32; 4],
    low: f32,
    gain: f32,
}
impl Voice {
    const EMPTY: Self = Self {
        note: Note {
            number: 0,
            velocity: 0,
        },
        phase: 0,
        step: 0,
        env: Envelope::IDLE,
        filter: [0.0; 4],
        low: 0.0,
        gain: 0.0,
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    #[allow(clippy::too_many_arguments)]
    fn sample(
        &mut self,
        t: &Timing,
        width: f32,
        sweep: f32,
        cutoff: f32,
        period: f32,
        noise: f32,
        patch: &Patch,
    ) -> f32 {
        let level = self.env.sample(t);
        // Accumulate the two-octave sub, then derive both DCO waves from it.
        let step = self.step as f32 * (1.0 - sweep * (1.0 - level)) * 0.25;
        self.phase = self.phase.wrapping_add(step as u32);
        let phase = self.phase.wrapping_shl(2);
        let ramp = f32::from((phase >> 16) as u16) / 65536.0;
        let duty = match patch.pulse_wave {
            1 => 0.5,
            2 => 0.25,
            3 => width * 0.5,
            _ => 0.0,
        };
        let pulse = if ramp >= 1.0 - duty {
            1.0 - duty
        } else {
            -duty
        };
        // Saw 3 gates the rising ramp late in each half-cycle. At zero PW
        // its gate stays open and it becomes the ordinary saw (Roland p.21).
        let gate = |shift: u32, duty: f32| {
            f32::from((phase.wrapping_shl(shift) >> 16) as u16) >= 65536.0 * (1.0 - duty)
        };
        let saw = match patch.saw_wave {
            1 => ramp - 0.5,
            2 | 3 => {
                let w = if patch.saw_wave == 2 { 0.5 } else { width };
                (if gate(1, w) { ramp } else { 0.0 }) - w * (3.0 - w) * 0.25
            }
            4 => (if gate(3, 0.5) { ramp } else { 0.0 }) - 0.265625,
            5 => {
                (if gate(1, 0.5) && gate(3, 0.5) {
                    ramp
                } else {
                    0.0
                }) - 0.1640625
            }
            _ => 0.0,
        };
        let sub_phase = if patch.sub_wave < 4 {
            self.phase.wrapping_shl(1)
        } else {
            self.phase
        };
        let (on, mean) = match patch.sub_wave {
            1 | 5 => (sub_phase >> 30 == 3, 0.25),
            2 => (sub_phase & 0xa000_0000 == 0xa000_0000, 0.25),
            3 => (sub_phase & 0x9000_0000 == 0x9000_0000, 0.25),
            _ => (sub_phase >> 31 != 0, 0.5),
        };
        let sub = (if on { 1.0 } else { 0.0 }) - mean;
        let mut x =
            pulse * 0.30 + saw * 0.43 + sub * (f32::from(patch.sub_level.min(3)) / 6.0) + noise;
        let hp = patch.hpf.min(3);
        let pole = if hp == 3 { 3500.0 } else { 850.0 };
        self.low += (pole * period).min(1.0) * (x - self.low);
        x += match hp {
            0 => 0.50 * self.low,
            1 => 0.0,
            _ => -self.low,
        };
        let feedback = f32::from(patch.resonance.min(127)) * (2.0 / 127.0);
        x -= feedback * self.filter[3];
        let env_amount = f32::from(patch.filter_env.min(127)) / 127.0;
        let g = (cutoff * (0.30 + 1.18 * env_amount * level) * period)
            .clamp(0.001, 0.94 - 0.15 * feedback);
        for state in &mut self.filter {
            *state += g * (x - *state);
            x = *state;
        }
        x * level * self.gain
    }
}

/// Four DCO voices and a shared stereo chorus.
#[derive(Debug)]
pub struct Hoover<'a> {
    tuning: &'a Tuning,
    keyboard: Keyboard,
    voices: [Voice; POLYPHONY],
    patch: Patch,
    timing: Timing,
    pwm_phase: u32,
    pwm_step: u32,
    chorus: Chorus,
    noise: u32,
    cutoff: f32,
    sweep: f32,
    pwm: f32,
}
impl<'a> Hoover<'a> {
    /// Start silent with the playable Hoover patch selected.
    #[must_use]
    pub fn new(tuning: &'a Tuning) -> Self {
        let patch = Patch::HOOVER;
        let period = tuning.sample_period();
        let mut synth = Self {
            tuning,
            keyboard: Keyboard::EMPTY,
            voices: [Voice::EMPTY; POLYPHONY],
            patch,
            timing: Timing::new(patch.times, patch.levels, period),
            pwm_phase: 0,
            pwm_step: 0,
            chorus: Chorus::new(period),
            noise: 1,
            cutoff: 0.0,
            sweep: 0.0,
            pwm: 0.0,
        };
        synth.set_patch(patch);
        synth
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn rate(hz: f32, period: f32) -> u32 {
        (hz * period * 4294967296.0) as u32
    }
    fn set_patch(&mut self, p: Patch) {
        self.patch = p;
        let period = self.tuning.sample_period();
        self.timing = Timing::new(p.times, p.levels, period);
        let cutoff = f32::from(p.cutoff.min(127)) / 127.0;
        self.cutoff = 700.0 + 90_000.0 * cutoff * cutoff;
        self.sweep = 0.75 * f32::from(p.sweep.min(127)) / 127.0;
        self.pwm_step = Self::rate(
            0.02 + 0.000355 * f32::from(p.pwm_rate.min(127)) * f32::from(p.pwm_rate.min(127)),
            period,
        );
        self.pwm = 0.94 * f32::from(p.pwm.min(127)) / 127.0;
        self.chorus.set_controls(ChorusControls {
            rate: p.chorus,
            enabled: p.chorus != 0,
            ..ChorusControls::INITIAL
        });
    }
    fn strike(&mut self, i: usize, note: Note, step: u32) {
        let v = &mut self.voices[i];
        *v = Voice {
            note,
            step,
            gain: f32::from(note.velocity) / (127.0 * 4.0),
            ..Voice::EMPTY
        };
        v.env.strike(&self.timing);
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
        let motion = if self.patch.pwm_rate == 0 {
            1.0
        } else {
            0.5 + 0.5 * triangle(self.pwm_phase)
        };
        let width = 1.0 - self.pwm * motion;
        self.pwm_phase = self.pwm_phase.wrapping_add(self.pwm_step);
        self.noise = self.noise.wrapping_mul(1664525).wrapping_add(1013904223);
        let noise = f32::from((self.noise >> 16) as u16) * (0.018 / 65536.0) - 0.009;
        let mut sum = 0.0;
        for v in &mut self.voices {
            if !v.env.idle() {
                sum += v.sample(
                    &self.timing,
                    width,
                    self.sweep,
                    self.cutoff,
                    self.tuning.sample_period(),
                    noise * f32::from(self.patch.noise.min(3)),
                    &self.patch,
                );
            }
        }
        self.chorus.stereo_frame(sum)
    }
    fn render<S: FromSample<f32>>(&mut self, out: &mut [S]) {
        for frame in out {
            let [l, r] = self.stereo_frame();
            *frame = S::from_sample_((l + r) * 0.5);
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
        assert_eq!(synth.stereo_frame(), [0.0; 2]);
    }
}
