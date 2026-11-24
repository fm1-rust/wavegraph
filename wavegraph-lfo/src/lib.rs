#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

use wavegraph_core::{Event, Port, Process, ProcessContext};

/// Shape of the bipolar control signal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Shape {
    /// Smooth sine approximation, starting at zero and rising.
    #[default]
    Sine,
    /// Linear rise and fall, starting at zero and rising.
    Triangle,
    /// Rising ramp from -1 to +1.
    Saw,
    /// Alternating +1 and -1 with equal duty.
    Square,
}

/// Control-rate oscillator with one float message outlet.
#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    /// Waveform sent to the control outlet.
    pub shape: Shape,
    phase: u32,
    increment: u32,
    units_per_hz: f32,
}

impl Lfo {
    /// Construct a stopped oscillator for a positive, finite audio sample rate.
    #[must_use]
    pub fn new(sample_rate: f32) -> Option<Self> {
        let units_per_hz = 4_294_967_296.0 / sample_rate;
        (sample_rate.is_finite() && units_per_hz.is_finite() && units_per_hz > 0.0).then_some(
            Self {
                shape: Shape::Sine,
                phase: 0,
                increment: 0,
                units_per_hz,
            },
        )
    }

    /// Set frequency without resetting phase. Negative or NaN rates stop the
    /// oscillator. Rates above half the audio sample rate saturate at Nyquist.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Bounded fractional phase increments round to the nearest Q32 unit."
    )]
    pub fn set_frequency(&mut self, hz: f32) {
        self.increment = ((hz * self.units_per_hz).clamp(0.0, 2_147_483_648.0) + 0.5) as u32;
    }

    /// Restart the waveform at phase zero.
    pub fn reset(&mut self) {
        self.phase = 0;
    }

    /// Advance without generating intermediate messages, including across a missed block.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Only the low 32 bits contribute to modular phase."
    )]
    pub fn advance(&mut self, frames: usize) {
        self.phase = self
            .phase
            .wrapping_add(self.increment.wrapping_mul(frames as u32));
    }

    /// Sample the current phase and advance by the supplied audio frame count.
    /// Empty calls produce no message.
    pub fn tick(&mut self, frames: usize) -> Option<Event<f32>> {
        if frames == 0 {
            return None;
        }
        let p = f32::from((self.phase >> 16) as u16) / 65536.0;
        let message = match self.shape {
            Shape::Saw => 2.0 * p - 1.0,
            Shape::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Shape::Sine | Shape::Triangle => {
                let t = if p < 0.75 {
                    1.0 - (4.0 * p - 1.0).abs()
                } else {
                    4.0 * p - 4.0
                };
                if self.shape == Shape::Triangle {
                    t
                } else {
                    let y = t * (2.0 - t.abs());
                    y + 0.225 * (y * y.abs() - y)
                }
            }
        };
        self.advance(frames);
        Some(Event {
            frame: 0,
            port: 0,
            message,
        })
    }
}

impl<S> Process<f32, S> for Lfo {
    fn inlets(&self) -> &[Port] {
        &[]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, f32, S>) {
        if let Some(event) = self.tick(cx.frames) {
            let _ = (cx.emit)(event);
        }
    }
}
