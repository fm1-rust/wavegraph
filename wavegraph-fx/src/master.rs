//! Three-band EQ, feed-forward compression and sample-peak limiting on the dry/wet sum.

use crate::{
    eq::{Bands, GainRamp, band_gain},
    pcm,
};
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::I24;

/// Master compressor and final limiter controls, in whole dB except for the ratio.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dynamics {
    /// Threshold in dBFS, clamped to −36..=0.
    pub threshold_db: i8,
    /// Ratio in half steps, from 2 (1:1) through 16 (8:1).
    pub ratio_x2: u8,
    /// Gain after compression and before limiting, clamped to 0..=12 dB.
    pub makeup_db: u8,
    /// Sample-peak ceiling in dBFS, clamped to −12..=0.
    pub ceiling_db: i8,
}

impl Dynamics {
    /// Gentle 2:1 bus compression with +4 dB of makeup and 1 dB of peak margin.
    pub const GLUE: Self = Self {
        threshold_db: -12,
        ratio_x2: 4,
        makeup_db: 4,
        ceiling_db: -1,
    };
}

/// An atomic edit of one master insert. Bypass preserves its panel settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MasterControls {
    /// Low, mid and high gains in dB (−12..=12). `None` gives a flat response.
    Eq(Option<[i8; 3]>),
    /// Compressor and limiter settings. `None` bypasses both stages.
    Dynamics(Option<Dynamics>),
}

const FULL: f32 = 8_388_608.0;
const WINDOW: u8 = if cfg!(feature = "short-detector-probe") {
    4
} else {
    16
};

fn gain(db: i8) -> f32 {
    libm::powf(10.0, f32::from(db) / 20.0)
}

/// Mono master insert. EQ, compression, makeup and limiting all operate
/// before the final Q23 conversion. Use [`Self::process_sample`] to pass a
/// wide bus sum directly without first clipping it to a signal port.
/// The compressor observes consecutive 16-sample peaks (four in the detector probe). The independent
/// zero-lookahead limiter acts on every sample, including the first transient.
#[derive(Clone, Copy, Debug)]
pub struct MasterBus {
    bands: Bands,
    gains: GainRamp<4>,
    dynamics: bool,
    inv_threshold: f32,
    curve: [f32; 2],
    ceiling: f32,
    attack: f32,
    release: f32,
    limit_release: f32,
    peak: f32,
    pending: u8,
    target: f32,
    comp_gain: f32,
    comp_rate: f32,
    limit_gain: f32,
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Bounded floating-point audio and gain coefficients"
)]
impl MasterBus {
    /// Start with flat EQ and bypassed dynamics. Returns `None` for a zero sample rate.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "Audio clock fits the precision of the filters"
    )]
    pub fn new(rate_hz: u32) -> Option<Self> {
        if rate_hz == 0 {
            return None;
        }
        let rate = rate_hz as f32;
        let ms = |t: f32| -libm::expm1f(-1000.0 / (t * rate));
        Some(Self {
            bands: Bands::new(rate),
            gains: GainRamp::UNITY,
            dynamics: false,
            inv_threshold: 1.0 / FULL,
            curve: [1.0; 2],
            ceiling: FULL,
            attack: ms(15.0),
            release: ms(150.0),
            limit_release: ms(80.0),
            peak: 0.0,
            pending: WINDOW,
            target: 1.0,
            comp_gain: 1.0,
            comp_rate: 0.0, // Both gains start at unity until the first detector window.
            limit_gain: 1.0,
        })
    }

    /// Apply one insert edit. Filter and detector state survive parameter edits.
    #[cold]
    pub fn edit(&mut self, control: MasterControls) {
        let mut gains = self.gains.target;
        match control {
            MasterControls::Eq(eq) => {
                gains[..3].copy_from_slice(&eq.unwrap_or([0; 3]).map(band_gain));
            }
            MasterControls::Dynamics(Some(d)) => {
                self.dynamics = true;
                self.inv_threshold = 1.0 / (FULL * gain(d.threshold_db.clamp(-36, 0)));
                let slope = 2.0 / f32::from(d.ratio_x2.clamp(2, 16)) - 1.0;
                self.curve = [libm::powf(1.5, slope), libm::exp2f(slope)];
                gains[3] = gain(d.makeup_db.min(12).cast_signed());
                self.ceiling = FULL * gain(d.ceiling_db.clamp(-12, 0));
            }
            MasterControls::Dynamics(None) => {
                self.dynamics = false;
                gains[3] = 1.0;
                self.peak = 0.0;
                self.pending = WINDOW;
                self.target = 1.0;
                self.comp_gain = 1.0;
                self.comp_rate = self.release;
                self.limit_gain = 1.0;
            }
        }
        self.gains.set(gains);
    }

    // Piecewise-linear approximation to over^(1/ratio - 1), with knots at
    // 1, 1.5 and 2 in each octave above threshold. No pow/log in the sample loop.
    fn reduction(&self, mut over: f32) -> f32 {
        if over <= 1.0 {
            return 1.0;
        }
        let [mid, end] = self.curve;
        let mut octave_gain = 1.0;
        while over >= 2.0 {
            over *= 0.5;
            octave_gain *= end;
        }
        octave_gain
            * if over < 1.5 {
                1.0 + (mid - 1.0) * (2.0 * (over - 1.0))
            } else {
                mid + (end - mid) * (2.0 * (over - 1.5))
            }
    }

    fn compress(&mut self, x: f32) -> f32 {
        // Samples come from a bounded integer mix and finite EQ gains.
        let magnitude = x.abs();
        if magnitude > self.peak {
            self.peak = magnitude;
        }
        self.pending -= 1;
        if self.pending == 0 {
            self.target = self.reduction(self.peak * self.inv_threshold);
            // Between detector updates the gain approaches a fixed target monotonically.
            self.comp_rate = if self.target < self.comp_gain {
                self.attack
            } else {
                self.release
            };
            self.pending = WINDOW;
            self.peak = 0.0;
        }
        self.comp_gain += self.comp_rate * (self.target - self.comp_gain);
        let boosted = x * self.comp_gain * self.gains.current[3];
        let out = if self.limit_gain == 1.0 {
            boosted
        } else {
            self.limit_gain += self.limit_release * (1.0 - self.limit_gain);
            boosted * self.limit_gain
        };
        if out.abs() > self.ceiling {
            self.limit_gain = self.ceiling / boosted.abs();
            self.ceiling.copysign(out)
        } else {
            out
        }
    }

    /// Current compressor and limiter gain, excluding makeup. Unity means no gain reduction.
    #[must_use]
    pub fn dynamics_gain(&self) -> f32 {
        self.comp_gain * self.limit_gain
    }

    /// Process a signed sample in Q23 units. An i32 accumulator leaves headroom
    /// for boosted tracks and returns before EQ and dynamics.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "Q23 samples are exact in f32; the widened mix is quantized on output"
    )]
    pub fn process_sample(&mut self, sum: i32) -> I24 {
        let x = sum as f32;
        self.gains.advance();
        let [lo, mid, hi, _] = self.gains.current;
        let eq = self.bands.tick(x, [lo, mid, hi]);
        let y = if self.dynamics { self.compress(eq) } else { eq };
        pcm(y as i32)
    }
}

impl Process<MasterControls, I24> for MasterBus {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, MasterControls, I24>) {
        let mut start = 0;
        for event in cx.events {
            for (x, y) in cx.inputs[0][start..event.frame]
                .iter()
                .zip(&mut cx.outputs[0][start..event.frame])
            {
                *y = self.process_sample(x.inner());
            }
            self.edit(event.message);
            start = event.frame;
        }
        for (x, y) in cx.inputs[0][start..cx.frames]
            .iter()
            .zip(&mut cx.outputs[0][start..cx.frames])
        {
            *y = self.process_sample(x.inner());
        }
    }
}
