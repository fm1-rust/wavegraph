//! A resonant filter whose envelope can be triggered by an external rhythm.

use super::{blend, pcm};
use core::f32::consts::{LN_10, PI, SQRT_2};
use wavegraph_sample::I24;

/// Filter output for the two-pole state-variable filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterMode {
    /// Pass frequencies below the cutoff.
    LowPass,
    /// Pass a band around the cutoff.
    BandPass,
}

/// Controls for a rhythmically retriggered filter envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PatternFilterControls {
    /// Low-pass or band-pass output.
    pub mode: FilterMode,
    /// Resting cutoff in Hz. Both sweep endpoints are limited to 40% of the sample rate.
    pub cutoff_hz: u16,
    /// Resonance, 0..=128 (Q of approximately 0.7 to 4).
    pub resonance: u8,
    /// Envelope sweep, 0..=128 (zero to five octaves above the resting cutoff).
    pub amount: u8,
    /// Time for a full-strength envelope to decay to 1%, in milliseconds.
    pub decay_ms: u16,
    /// Enable with a short wet/dry crossfade.
    pub enabled: bool,
}

impl PatternFilterControls {
    /// A low-pass pulse suitable for a held saw or a drum loop.
    pub const INITIAL: Self = Self {
        mode: FilterMode::LowPass,
        cutoff_hz: 320,
        resonance: 72,
        amount: 96,
        decay_ms: 240,
        enabled: true,
    };
}

/// A sixteen-step rhythm. Zero is a rest; 127 is a full-strength attack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilterPattern {
    /// Short picker label.
    pub name: &'static str,
    /// Envelope trigger strength at each step.
    pub accents: [u8; 16],
}

/// Original rhythms for the pattern-controlled filter, in picker order.
pub const FILTER_PATTERNS: [FilterPattern; 8] = [
    FilterPattern {
        name: "QUARTERS",
        accents: [127, 0, 0, 0, 127, 0, 0, 0, 127, 0, 0, 0, 127, 0, 0, 0],
    },
    FilterPattern {
        name: "OFFBEAT",
        accents: [0, 0, 127, 0, 0, 0, 127, 0, 0, 0, 127, 0, 0, 0, 127, 0],
    },
    FilterPattern {
        name: "EIGHTHS",
        accents: [127, 0, 80, 0, 112, 0, 80, 0, 127, 0, 80, 0, 112, 0, 80, 0],
    },
    FilterPattern {
        name: "SIXTEENTHS",
        accents: [
            127, 56, 88, 64, 112, 56, 88, 64, 127, 56, 88, 64, 112, 56, 88, 80,
        ],
    },
    FilterPattern {
        name: "BACKBEAT",
        accents: [0, 0, 0, 0, 127, 0, 0, 0, 0, 0, 0, 0, 127, 0, 0, 0],
    },
    FilterPattern {
        name: "TRESILLO",
        accents: [127, 0, 0, 0, 0, 0, 112, 0, 0, 0, 0, 0, 112, 0, 0, 0],
    },
    FilterPattern {
        name: "GALLOP",
        accents: [
            127, 0, 72, 90, 112, 0, 72, 90, 127, 0, 72, 90, 112, 0, 72, 90,
        ],
    },
    FilterPattern {
        name: "PICKUP",
        accents: [127, 0, 0, 72, 0, 0, 0, 96, 0, 0, 112, 0, 0, 0, 72, 96],
    },
];

/// A resonant insert for mono Q23 audio. The caller clocks the envelope with
/// [`trigger`](Self::trigger), independently of the instrument's notes.
#[derive(Clone, Copy, Debug)]
pub struct PatternFilter {
    mode: FilterMode,
    enabled: bool,
    sample_rate: f32,
    base: f32,
    span: f32,
    damping: f32,
    decay: f32,
    slew: f32,
    envelope: f32,
    g: f32,
    coefficients: [f32; 3],
    state: [f32; 2],
    phase: u8,
    mix: i32,
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "The sample rate is positive, both cutoff endpoints are below Nyquist, and damping is positive."
)]
impl PatternFilter {
    /// Construct a filter for a given sample rate. Zero is invalid.
    #[must_use]
    #[allow(clippy::cast_precision_loss, reason = "Audio rates fit f32 precision")]
    pub fn new(rate_hz: u32) -> Option<Self> {
        if rate_hz == 0 {
            return None;
        }
        let sample_rate = rate_hz as f32;
        let mut filter = Self {
            mode: PatternFilterControls::INITIAL.mode,
            enabled: PatternFilterControls::INITIAL.enabled,
            sample_rate,
            base: 0.0,
            span: 0.0,
            damping: 0.0,
            decay: 0.0,
            slew: -libm::expm1f(-1000.0 / sample_rate),
            envelope: 0.0,
            g: 0.0,
            coefficients: [0.0; 3],
            state: [0.0; 2],
            phase: 0,
            mix: 0,
        };
        filter.set_controls(PatternFilterControls::INITIAL);
        filter.g = filter.base;
        Some(filter)
    }

    /// Change the filter and envelope settings while preserving filter history.
    pub fn set_controls(&mut self, controls: PatternFilterControls) {
        self.mode = controls.mode;
        self.enabled = controls.enabled;
        let warp = |hz: f32| libm::tanf(PI * (hz / self.sample_rate).min(0.4));
        let cutoff = f32::from(controls.cutoff_hz.max(1));
        let octaves = f32::from(controls.amount.min(128)) * (5.0 / 128.0);
        self.base = warp(cutoff);
        self.span = warp(cutoff * libm::exp2f(octaves)) - self.base;
        self.damping = SQRT_2 - (SQRT_2 - 0.25) * f32::from(controls.resonance.min(128)) / 128.0;
        let samples = self.sample_rate * f32::from(controls.decay_ms.max(1)) / 1000.0;
        self.decay = libm::expf(-2.0 * LN_10 / samples);
        self.phase = 0;
    }

    /// Retrigger at the current sample. Zero leaves the existing decay alone.
    pub fn trigger(&mut self, strength: u8) {
        if strength != 0 {
            self.envelope = f32::from(strength.min(127)) / 127.0;
            self.phase = 0;
        }
    }

    /// Crossfade to the filtered or dry signal without recalculating the filter coefficients.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Fade to dry, then retire the envelope so the next enable starts at the resting cutoff.
    /// A fully dry block is left untouched in constant time.
    pub fn bypass_in_place(&mut self, samples: &mut [I24]) {
        self.enabled = false;
        for sample in samples {
            if self.mix == 0 {
                break;
            }
            *sample = self.tick(*sample);
        }
        if self.mix == 0 {
            self.envelope = 0.0;
            self.g = self.base;
        }
    }

    /// Process one signed Q23 sample.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "Q23 input is exact in f32; the final conversion clips to the output rails."
    )]
    pub fn tick(&mut self, sample: I24) -> I24 {
        self.g += self.slew * (self.base + self.span * self.envelope - self.g);
        self.envelope = if self.envelope < 1e-8 {
            0.0
        } else {
            self.envelope * self.decay
        };
        let mix = blend(&mut self.mix, if self.enabled { 128 } else { 0 });
        if mix == 0 {
            self.state = [0.0; 2];
            self.phase = 0;
            return sample;
        }
        if self.phase == 0 {
            // Trapezoidal state-variable filter. One division per eight
            // samples; the cutoff and envelope themselves slew every sample.
            let g = self.g;
            let a1 = 1.0 / (1.0 + g * (g + self.damping));
            let a2 = g * a1;
            self.coefficients = [a1, a2, g * a2];
        }
        self.phase = self.phase.wrapping_add(1) & 7;
        let x = sample.inner() as f32;
        let [s1, s2] = self.state;
        let [a1, a2, a3] = self.coefficients;
        let v = x - s2;
        let band = a1 * s1 + a2 * v;
        let low = s2 + a2 * s1 + a3 * v;
        self.state = [2.0 * band - s1, 2.0 * low - s2];
        let wet = match self.mode {
            FilterMode::LowPass => low,
            FilterMode::BandPass => self.damping * band,
        };
        pcm((x + (mix as f32 / 128.0) * (wet - x)) as i32)
    }
}
