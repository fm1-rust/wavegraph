//! Signal models for the bass drum and snare circuits.

/// A rational graph clock converted once, before rendering.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Clock(f32);

impl Clock {
    #[allow(
        clippy::cast_precision_loss,
        reason = "DSP timing coefficients use f32."
    )]
    pub(crate) fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        Some(Self(numerator as f32 / denominator as f32))
    }

    pub(crate) fn hz(self) -> f32 {
        self.0
    }

    pub(crate) fn decay(self, seconds: f32) -> f32 {
        1.0 - 1.0 / (1.0 + self.0 * seconds)
    }

    pub(crate) fn filter_step(self, hz: f32) -> f32 {
        1.0 / (1.0 + self.0 / (core::f32::consts::TAU * hz))
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Round positive durations to frames."
    )]
    pub(crate) fn frames(self, seconds: f32) -> u32 {
        ((self.0 * seconds + 0.5) as u32).max(1)
    }
}

/// A bipolar bit from a maximal 32-bit shift register.
#[inline]
pub(crate) fn noise(state: &mut u32) -> f32 {
    let bit = *state & 1;
    *state = (*state >> 1) ^ (bit.wrapping_neg() & 0x8020_0003);
    if bit == 0 { -1.0 } else { 1.0 }
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "The final conversion saturates to signed PCM."
)]
pub(crate) fn pcm(signal: f32) -> i16 {
    signal as i16
}

#[derive(Clone, Copy, Debug)]
struct Oscillator {
    phase: f32,
    step: f32,
}

impl Oscillator {
    fn new(clock: Clock, hz: f32) -> Self {
        Self {
            phase: 0.25,
            step: hz / clock.hz(),
        }
    }

    fn reset(&mut self) {
        self.phase = 0.25;
    }

    #[inline]
    fn next(&mut self, pitch: f32) -> f32 {
        self.phase += (self.step * pitch).min(0.5);
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let triangle = 1.0 - 4.0 * (self.phase - 0.5).abs();
        // Smooth the triangle's tips to approximate the circuit's diode shaper.
        triangle * (1.5 - 0.5 * triangle * triangle)
    }
}

/// Pitch-swept, waveshaped kick with a separate short noise attack.
#[derive(Clone, Copy, Debug)]
pub struct BassDrum {
    oscillator: Oscillator,
    noise: u32,
    body: f32,
    bend: f32,
    click: f32,
    hold: u32,
    hold_frames: u32,
    body_decay: f32,
    bend_decay: f32,
    click_decay: f32,
}

impl BassDrum {
    /// Construct for a rational graph clock, returning `None` for a zero clock.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self::from_clock(Clock::new(numerator, denominator)?, 1.0))
    }

    pub(crate) fn from_clock(clock: Clock, decay: f32) -> Self {
        Self {
            oscillator: Oscillator::new(clock, 49.5),
            noise: 0x8469_f235,
            body: 0.0,
            bend: 0.0,
            click: 0.0,
            hold: 0,
            hold_frames: clock.frames(0.049),
            body_decay: clock.decay(0.070 * decay),
            bend_decay: clock.decay(0.0107),
            click_decay: clock.decay(0.0014),
        }
    }

    /// Retrigger the oscillator and its pitch, attack and volume envelopes.
    pub fn trigger(&mut self) {
        self.oscillator.reset();
        self.body = 1.0;
        self.bend = 5.35;
        self.click = 0.17;
        self.hold = self.hold_frames;
    }

    pub(crate) fn stop(&mut self) {
        self.body = 0.0;
    }

    /// Render the next mono PCM frame.
    #[inline]
    pub fn next_frame(&mut self) -> i16 {
        if self.body <= 0.06 {
            return 0;
        }
        let tone = self.oscillator.next(1.0 + self.bend);
        let signal = tone * (self.body - 0.06) + self.click * noise(&mut self.noise);
        if self.hold == 0 {
            self.body *= self.body_decay;
        } else {
            self.hold = self.hold.saturating_sub(1);
        }
        self.bend *= self.bend_decay;
        self.click *= self.click_decay;
        pcm(signal * 12_000.0)
    }
}

/// Two tuned drum oscillators with separate low- and high-pass snare noise.
#[derive(Clone, Copy, Debug)]
pub struct SnareDrum {
    shells: [Oscillator; 2],
    noise: u32,
    low: f32,
    high_cut: f32,
    low_step: f32,
    high_step: f32,
    body: f32,
    snap: f32,
    bend: f32,
    body_decay: f32,
    snap_decay: f32,
    bend_decay: f32,
}

impl SnareDrum {
    /// Construct for a rational graph clock, returning `None` for a zero clock.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self::from_clock(Clock::new(numerator, denominator)?, 1.0))
    }

    pub(crate) fn from_clock(clock: Clock, decay: f32) -> Self {
        Self {
            shells: [172.0, 272.0].map(|hz| Oscillator::new(clock, hz)),
            noise: 0x9352_6a7d,
            low: 0.0,
            high_cut: 0.0,
            low_step: clock.filter_step(1_100.0),
            high_step: clock.filter_step(3_900.0),
            body: 0.0,
            snap: 0.0,
            bend: 0.0,
            body_decay: clock.decay(0.036 * decay),
            snap_decay: clock.decay(0.048 * decay),
            bend_decay: clock.decay(0.007),
        }
    }

    /// Retrigger the shells and snare-wire noise envelopes.
    pub fn trigger(&mut self) {
        for shell in &mut self.shells {
            shell.reset();
        }
        self.body = 1.0;
        self.snap = 1.0;
        self.bend = 0.55;
    }

    pub(crate) fn stop(&mut self) {
        self.snap = 0.0;
    }

    /// Render the next mono PCM frame.
    #[inline]
    pub fn next_frame(&mut self) -> i16 {
        if self.snap < 0.00002 {
            return 0;
        }
        let raw = noise(&mut self.noise);
        self.low += self.low_step * (raw - self.low);
        self.high_cut += self.high_step * (raw - self.high_cut);
        let wires = 0.65 * (raw - self.high_cut) + 0.35 * self.low;
        let pitch = 1.0 + self.bend;
        let shell = self.shells[0].next(pitch) + 0.57 * self.body * self.shells[1].next(pitch);
        let signal = 0.56 * self.body * shell + 0.48 * self.snap * wires;
        self.body *= self.body_decay;
        self.snap *= self.snap_decay;
        self.bend *= self.bend_decay;
        pcm(signal * 10_400.0)
    }
}
