//! Persistent voice models. Only initialization and knob changes need division.

use core::f32::consts::TAU;
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::FromSample;

#[derive(Clone, Copy, Debug)]
pub(super) struct Clock(f32);

impl Clock {
    #[allow(
        clippy::cast_precision_loss,
        reason = "Coefficients use single-precision Hz."
    )]
    pub(super) fn new(numerator: u32, denominator: u32) -> Option<Self> {
        let hz = numerator as f32 / denominator as f32;
        (1_000.0..=384_000.0).contains(&hz).then_some(Self(hz))
    }

    fn decay(self, seconds: f32) -> f32 {
        1.0 - 1.0 / (1.0 + self.0 * seconds)
    }

    fn lowpass(self, hz: f32) -> f32 {
        let omega = TAU * hz;
        omega / (self.0 + omega)
    }
}

pub(super) fn unit(value: u8) -> f32 {
    f32::from(value.min(127)) * (1.0 / 127.0)
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "Saturating conversion to output PCM."
)]
fn pcm(value: f32) -> i16 {
    value as i16
}

// A damped quadrature resonator. A new pulse adds energy to the current
// motion, so a second kick can reinforce or cancel part of the ringing body.
#[derive(Clone, Copy, Debug)]
struct Resonator {
    x: f32,
    y: f32,
    step: f32,
    decay: f32,
}

impl Resonator {
    fn new(clock: Clock, hz: f32, seconds: f32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            step: (TAU * hz / clock.0).min(1.75),
            decay: clock.decay(seconds),
        }
    }

    fn excite(&mut self, pulse: f32) {
        self.x = (self.x + pulse).clamp(-1.25, 1.25);
    }

    #[inline]
    fn next(&mut self, bend: f32) -> f32 {
        let step = self.step * bend;
        self.y += step * self.x;
        self.x = (self.x - step * self.y) * self.decay;
        self.y *= self.decay;
        self.y
    }

    fn stop(&mut self) {
        self.x = 0.0;
        self.y = 0.0;
    }
}

/// Pulse-excited bass drum, with separate body decay and transient tone.
#[derive(Clone, Copy, Debug)]
pub struct BassDrum {
    clock: Clock,
    body: Resonator,
    bend: f32,
    click: f32,
    bend_decay: f32,
    click_decay: f32,
    tone: f32,
    level: f32,
    active: bool,
}

impl BassDrum {
    /// Construct at a rational sample rate between 1 and 384 kHz.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self::from_clock(Clock::new(numerator, denominator)?))
    }

    pub(super) fn from_clock(clock: Clock) -> Self {
        let mut drum = Self {
            clock,
            body: Resonator::new(clock, 51.0, 0.13),
            bend: 0.0,
            click: 0.0,
            bend_decay: clock.decay(0.004),
            click_decay: clock.decay(0.0012),
            tone: 0.0,
            level: 1.0,
            active: false,
        };
        drum.set_tone(super::Controls::INITIAL.kick_tone);
        drum.set_decay(super::Controls::INITIAL.kick_decay);
        drum
    }

    /// Adjust the brightness of the attack pulse.
    pub fn set_tone(&mut self, value: u8) {
        self.tone = unit(value);
    }
    /// Adjust the damping of the ringing body.
    pub fn set_decay(&mut self, value: u8) {
        self.body.decay = self.clock.decay(0.042 + 0.245 * unit(value));
    }
    /// Set the dry voice level (127 is unity).
    pub fn set_level(&mut self, value: u8) {
        self.level = unit(value);
    }
    /// Excite the resonator at its current phase with a MIDI velocity.
    pub fn trigger(&mut self, velocity: u8) {
        self.excite(unit(velocity));
    }

    pub(super) fn excite(&mut self, strength: f32) {
        if strength > 0.0 {
            self.body.excite(strength);
            self.bend = strength * 2.6;
            self.click = strength * (0.035 + 0.32 * self.tone);
            self.active = true;
        }
    }

    /// Whether the drum is ringing.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }
    /// Silence the resonator and attack.
    pub fn stop(&mut self) {
        self.body.stop();
        self.bend = 0.0;
        self.click = 0.0;
        self.active = false;
    }
    /// Render one signed PCM16 sample.
    #[inline]
    pub fn next_frame(&mut self) -> i16 {
        if !self.active {
            return 0;
        }
        let sample = self.body.next(1.0 + self.bend) + self.click;
        self.bend *= self.bend_decay;
        self.click *= self.click_decay;
        if self.body.x.abs() + self.body.y.abs() < 0.00006 {
            self.stop();
        }
        pcm(sample * (16_000.0 * self.level))
    }
}

/// Two pitched snare resonators and an independently enveloped noise path.
#[derive(Clone, Copy, Debug)]
pub struct SnareDrum {
    shells: [Resonator; 2],
    tone: f32,
    snappy: f32,
    level: f32,
    noise: u32,
    low: f32,
    low_step: f32,
    wires: f32,
    decay: f32,
}

impl SnareDrum {
    /// Construct at a rational sample rate between 1 and 384 kHz.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self::from_clock(Clock::new(numerator, denominator)?))
    }

    pub(super) fn from_clock(clock: Clock) -> Self {
        Self {
            shells: [
                Resonator::new(clock, 180.0, 0.044),
                Resonator::new(clock, 330.0, 0.033),
            ],
            tone: unit(super::Controls::INITIAL.snare_tone),
            snappy: unit(super::Controls::INITIAL.snappy),
            level: 1.0,
            noise: 0xb27a_c189,
            low: 0.0,
            low_step: clock.lowpass(1_900.0),
            wires: 0.0,
            decay: clock.decay(0.060),
        }
    }

    /// Balance the lower and upper drum resonances.
    pub fn set_tone(&mut self, value: u8) {
        self.tone = unit(value);
    }
    /// Set the snare-wire noise level independently of the body.
    pub fn set_snappy(&mut self, value: u8) {
        self.snappy = unit(value);
    }
    /// Set the dry voice level (127 is unity).
    pub fn set_level(&mut self, value: u8) {
        self.level = unit(value);
    }
    /// Excite both shells and the wires with a MIDI velocity.
    pub fn trigger(&mut self, velocity: u8) {
        self.excite(unit(velocity));
    }

    pub(super) fn excite(&mut self, strength: f32) {
        if strength > 0.0 {
            for shell in &mut self.shells {
                shell.excite(strength);
            }
            self.wires = strength;
        }
    }

    /// Whether the snare is ringing.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.wires > 0.0
    }
    /// Silence both pitched and noise components.
    pub fn stop(&mut self) {
        for shell in &mut self.shells {
            shell.stop();
        }
        self.wires = 0.0;
        self.low = 0.0;
    }
    /// Render one signed PCM16 sample.
    #[inline]
    pub fn next_frame(&mut self) -> i16 {
        if !self.is_active() {
            return 0;
        }
        let bit = self.noise & 1;
        self.noise = (self.noise >> 1) ^ (bit.wrapping_neg() & 0x8020_0003);
        let noise = if bit == 0 { -1.0 } else { 1.0 };
        self.low += self.low_step * (noise - self.low);
        let low = self.shells[0].next(1.0);
        let high = self.shells[1].next(1.0);
        let shell = low + self.tone * (high - low);
        let wire = (noise - self.low) * self.wires * self.snappy;
        self.wires *= self.decay;
        if self.wires < 0.00004 {
            self.stop();
        }
        pcm((6_700.0 * shell + 7_100.0 * wire) * self.level)
    }
}

/// Free-running bank of six rectangular oscillators for the hat signal inlet.
#[derive(Clone, Copy, Debug)]
pub struct Metal {
    phases: [u32; 6],
    steps: [u32; 6],
}

impl Metal {
    /// Construct at a rational sample rate between 1 and 384 kHz.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self::from_clock(Clock::new(numerator, denominator)?))
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Each positive phase step is less than half a cycle."
    )]
    pub(super) fn from_clock(clock: Clock) -> Self {
        Self {
            phases: [
                0,
                0x35a0_0000,
                0x7130_0000,
                0x89f0_0000,
                0xbe10_0000,
                0xe330_0000,
            ],
            steps: [205.0, 369.6, 304.4, 522.7, 800.0, 540.5]
                .map(|hz| ((hz / clock.0).min(0.49) * 4_294_967_296.0 + 0.5) as u32),
        }
    }

    /// Advance through a silent interval without synthesizing it.
    pub fn advance(&mut self, frames: u32) {
        for (phase, step) in self.phases.iter_mut().zip(self.steps) {
            *phase = phase.wrapping_add(step.wrapping_mul(frames));
        }
    }

    /// Render the metallic excitation shared by both hats.
    #[inline]
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Sum of six individual sign bits."
    )]
    pub fn next_frame(&mut self) -> f32 {
        let mut high = 0_u8;
        for (phase, step) in self.phases.iter_mut().zip(self.steps) {
            *phase = phase.wrapping_add(step);
            high += u8::from(*phase >= 1 << 31);
        }
        f32::from(high) * 0.25 - 0.75
    }
}

impl<M, S: FromSample<f32>> Process<M, S> for Metal {
    fn inlets(&self) -> &[Port] {
        &[]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, M, S>) {
        for out in &mut cx.outputs[0][..cx.frames] {
            *out = S::from_sample_(self.next_frame());
        }
    }
}

/// Hat envelope and filter. Feed the signal inlet from a shared [`Metal`].
#[derive(Clone, Copy, Debug)]
pub struct HiHat {
    decay: f32,
    choke_decay: f32,
    release: f32,
    envelope: f32,
    level: f32,
    lows: [f32; 2],
    low_step: f32,
}

impl HiHat {
    /// Construct a closed hat at a rational sample rate between 1 and 384 kHz.
    #[must_use]
    pub fn closed(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self::from_clock(Clock::new(numerator, denominator)?, false))
    }
    /// Construct an open hat at a rational sample rate between 1 and 384 kHz.
    #[must_use]
    pub fn open(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self::from_clock(Clock::new(numerator, denominator)?, true))
    }

    pub(super) fn from_clock(clock: Clock, open: bool) -> Self {
        let decay = clock.decay(if open { 0.105 } else { 0.019 });
        Self {
            decay,
            choke_decay: clock.decay(0.0025),
            release: decay,
            envelope: 0.0,
            level: 1.0,
            lows: [0.0; 2],
            low_step: clock.lowpass(5_400.0),
        }
    }

    /// Set the dry voice level (127 is unity).
    pub fn set_level(&mut self, value: u8) {
        self.level = unit(value);
    }
    /// Strike the hat at a MIDI velocity, leaving the metal source free-running.
    pub fn trigger(&mut self, velocity: u8) {
        self.excite(unit(velocity));
    }

    pub(super) fn excite(&mut self, strength: f32) {
        if strength > 0.0 {
            self.envelope = strength;
            self.release = self.decay;
        }
    }

    /// Quickly damp the remaining open-hat tail.
    pub fn choke(&mut self) {
        self.release = self.choke_decay;
    }
    /// Whether the envelope is running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.envelope > 0.0
    }
    /// Silence the hat and clear its filter memory.
    pub fn stop(&mut self) {
        self.envelope = 0.0;
        self.lows = [0.0; 2];
    }
    /// Render one signed PCM16 sample from the shared metal input.
    #[inline]
    pub fn next_frame(&mut self, mut metal: f32) -> i16 {
        if !self.is_active() {
            return 0;
        }
        for low in &mut self.lows {
            *low += self.low_step * (metal - *low);
            metal -= *low;
        }
        let sample = pcm(metal * self.envelope * (33_000.0 * self.level));
        self.envelope *= self.release;
        if self.envelope < 0.00005 {
            self.stop();
        }
        sample
    }
}

// Standalone children and the kit share their per-frame voice implementations.
macro_rules! drum_process {
    ($voice:ty, $inlets:expr, |$drum:ident, $cx:ident, $frame:ident| $sample:expr) => {
        impl<S: Copy + FromSample<i16>> Process<u8, S> for $voice
        where
            f32: FromSample<S>,
        {
            fn inlets(&self) -> &[Port] {
                $inlets
            }
            fn outlets(&self) -> &[Port] {
                &[Port::Signal]
            }
            fn process(&mut self, $cx: &mut ProcessContext<'_, '_, u8, S>) {
                let $drum = self;
                let mut start = 0;
                for event in $cx.events {
                    for $frame in start..event.frame {
                        $cx.outputs[0][$frame] = S::from_sample_($sample);
                    }
                    $drum.trigger(event.message);
                    start = event.frame;
                }
                for $frame in start..$cx.frames {
                    $cx.outputs[0][$frame] = S::from_sample_($sample);
                }
            }
        }
    };
}
drum_process!(BassDrum, &[Port::Message], |drum, cx, frame| drum
    .next_frame());
drum_process!(SnareDrum, &[Port::Message], |drum, cx, frame| drum
    .next_frame());
drum_process!(HiHat, &[Port::Message, Port::Signal], |hat, cx, frame| hat
    .next_frame(f32::from_sample_(cx.inputs[0][frame])));
