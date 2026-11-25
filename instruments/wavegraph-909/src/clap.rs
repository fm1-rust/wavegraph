//! The TR-909 hand clap's noise filter and two amplitude envelopes.

use crate::analog::{Clock, noise, pcm};

/// A circuit-inspired hand clap. A message on inlet 0 retriggers both envelopes.
/// The noise and filter keep their state across strikes.
#[derive(Clone, Copy, Debug)]
pub struct HandClap {
    // State-variable band-pass filter (IC26b), plus the darker pseudo-reverb path.
    band_state: f32,
    low_state: f32,
    direct_low: f32,
    tail_filter: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    direct_filter_step: f32,
    tail_filter_step: f32,
    noise: u32,
    direct: f32,
    tail: f32,
    burst_decay: f32,
    body_decay: f32,
    tail_decay: f32,
    burst_spacing: u32,
    until_burst: u32,
    bursts_left: u8,
    hit_frames: u32,
    remaining: u32,
}

impl HandClap {
    /// Set the graph clock to `numerator / denominator` frames per second.
    /// Returns `None` for a zero clock.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self::from_clock(Clock::new(numerator, denominator)?, 1.0))
    }

    pub(crate) fn from_clock(clock: Clock, decay: f32) -> Self {
        let rate = clock.hz();
        // IC26b: C42 = C43 = 4.7 nF, with 47k, 10k and 150k resistors.
        // Bilinear transform of the nominal 963 Hz band-pass (Q = 2.14).
        let g = core::f32::consts::PI * 963.0 / rate;
        let a1 = 1.0 / (1.0 + g * (g + 1.0 / 2.14));
        let a2 = g * a1;
        // One-pole RC decay; coefficients change at control boundaries.
        let mut voice = Self {
            band_state: 0.0,
            low_state: 0.0,
            direct_low: 0.0,
            tail_filter: 0.0,
            a1,
            a2,
            a3: g * a2,
            // Effective voicing of the bright, AC-coupled VCA and darker tail.
            direct_filter_step: clock.filter_step(1_250.0),
            tail_filter_step: clock.filter_step(430.0),
            noise: 0xa924_6d3b,
            direct: 0.0,
            tail: 0.0,
            burst_decay: clock.decay(0.0022),
            body_decay: 0.0,
            tail_decay: 0.0,
            burst_spacing: clock.frames(0.0107),
            until_burst: 0,
            bursts_left: 0,
            hit_frames: 0,
            remaining: 0,
        };
        voice.set_decay(clock, decay);
        voice
    }

    pub(crate) fn set_decay(&mut self, clock: Clock, decay: f32) {
        let elapsed = self.hit_frames.saturating_sub(self.remaining);
        self.body_decay = clock.decay(0.018 * decay);
        self.tail_decay = clock.decay(0.095 * decay);
        self.hit_frames = clock.frames(0.04 + 0.96 * decay).max(4);
        if self.remaining != 0 {
            self.remaining = self.hit_frames.saturating_sub(elapsed);
        }
    }

    /// Start the first of four noise bursts on the next frame.
    pub fn trigger(&mut self) {
        self.direct = 1.0;
        self.tail = 0.32;
        self.bursts_left = 3;
        self.until_burst = self.burst_spacing;
        self.remaining = self.hit_frames;
    }

    pub(crate) fn stop(&mut self) {
        self.remaining = 0;
    }

    pub(crate) fn is_active(&self) -> bool {
        self.remaining != 0
    }

    /// Render one signed 16-bit frame. Returns silence before the first strike
    /// and after both envelopes have decayed below the output resolution.
    #[inline]
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Counters decrement only above zero."
    )]
    pub fn next_frame(&mut self) -> i16 {
        if !self.is_active() {
            return 0;
        }
        self.remaining -= 1;
        if self.bursts_left > 0 {
            if self.until_burst == 0 {
                self.bursts_left -= 1;
                self.direct = if self.bursts_left == 0 { 0.60 } else { 1.0 };
                self.until_burst = self.burst_spacing;
            }
            self.until_burst -= 1;
        }

        let input = noise(&mut self.noise) - self.low_state;
        let band = self.a1 * self.band_state + self.a2 * input;
        let low = self.low_state + self.a2 * self.band_state + self.a3 * input;
        self.band_state = 2.0 * band - self.band_state;
        self.low_state = 2.0 * low - self.low_state;
        self.direct_low += self.direct_filter_step * (band - self.direct_low);
        self.tail_filter += self.tail_filter_step * (band - self.tail_filter);

        let output = self.direct * (band - self.direct_low) + self.tail * self.tail_filter;
        self.direct *= if self.bursts_left == 0 {
            self.body_decay
        } else {
            self.burst_decay
        };
        self.tail *= self.tail_decay;
        pcm(output * 16_000.0)
    }
}
