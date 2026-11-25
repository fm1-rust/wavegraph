//! Per-note amplitude and four-pole resonant low-pass filter.

use core::f32::consts::PI;
use micromath::F32;
use wavegraph_adsr::{Envelope, Shape, Timing};
use wavegraph_core::midi::Note;

/// A chromatic sampler program's two ADSRs and filter routings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timbre {
    /// Base cutoff in Hz. Use `OPEN` for the full-bandwidth setting.
    pub cutoff_hz: u16,
    /// Four-pole low-pass resonance, from 0 to 127.
    pub resonance: u8,
    /// Filter envelope depth in semitones. Negative values invert the sweep.
    pub env_semitones: i8,
    /// How far a soft strike closes the filter, in semitones.
    pub velocity_semitones: u8,
    /// Percentage of keyboard pitch change applied to cutoff, relative to the root.
    pub key_tracking: u8,
    /// Amplitude envelope.
    pub amp: Shape,
    /// Filter envelope, independent of the amplitude envelope.
    pub filter: Shape,
}

impl Timbre {
    /// At full bandwidth, neutral filter controls bypass the filter entirely.
    pub const OPEN: u16 = u16::MAX;
    /// Transparent playback for drums and already-shaped recordings.
    pub const DRY: Self = Self {
        cutoff_hz: Self::OPEN,
        resonance: 0,
        env_semitones: 0,
        velocity_semitones: 0,
        key_tracking: 0,
        amp: Shape::new(0, 0, 100, 0),
        filter: Shape::new(0, 0, 100, 0),
    };
}

#[derive(Clone, Debug)]
pub(super) struct VoiceTone {
    program: Timbre,
    amp: Envelope,
    filter: Envelope,
    amp_time: Timing,
    filter_time: Timing,
    period: f32,
    key: i16,
    velocity: u8,
    base_cycles: f32,
    bypass: bool,
    dry_gate: bool,
    state: [f32; 4],
    g: f32,
    feedback: f32,
    reciprocal: f32,
    phase: u8,
}

impl VoiceTone {
    pub(super) fn new(period: f32) -> Self {
        let time = Timbre::DRY.amp.adsr().at_sample_period(period);
        Self {
            program: Timbre::DRY,
            amp: Envelope::IDLE,
            filter: Envelope::IDLE,
            amp_time: time,
            filter_time: time,
            period,
            key: 0,
            velocity: 127,
            base_cycles: 0.0,
            bypass: true,
            dry_gate: false,
            state: [0.0; 4],
            g: 0.0,
            feedback: 0.0,
            reciprocal: 1.0,
            phase: 0,
        }
    }

    pub(super) fn strike(&mut self, program: Timbre, note: Note, root: u8) {
        self.key = i16::from(note.number).saturating_sub(i16::from(root));
        self.velocity = note.velocity.min(127);
        self.amp = Envelope::IDLE;
        self.filter = Envelope::IDLE;
        self.state = [0.0; 4];
        self.configure(program);
        self.amp.gate_on(&self.amp_time);
        self.filter.gate_on(&self.filter_time);
    }

    pub(super) fn configure(&mut self, p: Timbre) {
        if self.program.amp != p.amp {
            let next = p.amp.adsr().at_sample_period(self.period);
            self.amp.retime(&self.amp_time, &next);
            self.amp_time = next;
        }
        if self.program.filter != p.filter {
            let next = p.filter.adsr().at_sample_period(self.period);
            self.filter.retime(&self.filter_time, &next);
            self.filter_time = next;
        }
        self.program = p;
        self.dry_gate = false;
        let key = f32::from(self.key) * f32::from(p.key_tracking.min(100)) / 100.0;
        let velocity = (127.0 - f32::from(self.velocity)) * f32::from(p.velocity_semitones) / 127.0;
        let cycles = if p.cutoff_hz == Timbre::OPEN {
            0.45
        } else {
            f32::from(p.cutoff_hz.max(1)) * self.period
        };
        self.bypass = p.cutoff_hz == Timbre::OPEN
            && p.resonance == 0
            && p.env_semitones == 0
            && key == velocity;
        if !self.bypass {
            self.base_cycles = cycles * F32((key - velocity) / 12.0).exp2().0;
            self.feedback = f32::from(p.resonance.min(127)) * (3.6 / 127.0);
        }
        self.phase = 0;
        if self.bypass {
            self.state = [0.0; 4];
        }
    }

    /// The original playback release still provides de-clicking for a dry gate.
    pub(super) fn uses_playback_release(&self) -> bool {
        self.program.amp == Timbre::DRY.amp
    }

    pub(super) fn release(&mut self) {
        self.dry_gate = false;
        self.filter.gate_off(&self.filter_time);
        if !self.uses_playback_release() {
            self.amp.gate_off(&self.amp_time);
        }
    }

    pub(super) fn finished(&self) -> bool {
        self.amp.is_idle()
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "The final float-to-PCM16 cast saturates resonance peaks."
    )]
    pub(super) fn process(&mut self, sample: i16) -> i16 {
        if self.dry_gate {
            return sample;
        }
        // Keep both contours in time even when the player opens the filter mid-note.
        let amp = self.amp.advance(&self.amp_time);
        let env = self.filter.advance(&self.filter_time);
        if self.bypass && amp == 1.0 {
            // After a flat gate reaches its plateau, the break can stream straight from PCM.
            self.dry_gate = self.program.amp == Timbre::DRY.amp
                && self.program.filter == Timbre::DRY.filter
                && env == 1.0
                && self.amp.is_gated()
                && self.filter.is_gated();
            return sample;
        }
        let mut x = f32::from(sample);
        if !self.bypass {
            // Prewarp at control rate. A static cutoff needs only one coefficient update.
            if self.phase == 0 {
                let octaves = env * f32::from(self.program.env_semitones) / 12.0;
                let cycles = self.base_cycles * F32(octaves).exp2().0;
                let warp = F32(PI * cycles.clamp(0.00001, 0.45)).tan().0;
                self.g = warp / (1.0 + warp);
                let square = self.g * self.g;
                self.reciprocal = 1.0 / (1.0 + self.feedback * square * square);
            }
            self.phase = if self.program.env_semitones == 0 {
                1
            } else {
                self.phase.wrapping_add(1) & 15
            };
            // Solve the four trapezoidal one-poles' feedback loop at the current sample.
            let g = self.g;
            let [a, b, c, d] = self.state;
            let memory = (1.0 - g) * (((a * g + b) * g + c) * g + d);
            x = (x - self.feedback * memory) * self.reciprocal;
            for stage in &mut self.state {
                let delta = (x - *stage) * g;
                x = *stage + delta;
                *stage = x + delta;
            }
        }
        (x * amp) as i16
    }
}
