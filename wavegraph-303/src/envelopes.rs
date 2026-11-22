//! Control signals shared by the audio voice and silent envelope previews.

use crate::{
    VoiceParams,
    dsp::{clamp, fast_exp2, tau_to_coef},
};

/// Control signals at one instant in the 303 voice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeLevels {
    /// Ladder cutoff target in Hz, including the main envelope and accent sweep.
    pub cutoff_hz: f32,
    /// Contribution of the accent sweep to the filter cutoff, in octaves.
    pub accent_octaves: f32,
    /// Amplitude envelope multiplied by the smoothed accent gain, before output trim.
    pub vca_gain: f32,
}

/// The 303's filter, accent and amplitude envelopes, independent of the oscillator.
/// A display can run its own instance without touching a sounding voice.
#[derive(Clone, Copy, Debug)]
pub struct Envelopes {
    sample_rate: f32,
    hold: f32,
    release: f32,
    accent_charge: f32,
    accent_glide: f32,
    filter: f32,
    amplitude: f32,
    capacitor: f32,
    accent_gain: f32,
    pub(super) gate: bool,
    attacking: bool,
    accent: bool,
}

pub(super) struct Coefficients {
    filter_decay: f32,
    attack: f32,
    accent_drain: f32,
    accent_gain: f32,
}

impl Envelopes {
    /// Start discharged at the given positive sample rate in Hz.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            hold: tau_to_coef(1.34, sample_rate),
            release: tau_to_coef(0.004, sample_rate),
            accent_charge: tau_to_coef(0.012, sample_rate),
            accent_glide: tau_to_coef(0.002, sample_rate),
            filter: 0.0,
            amplitude: 0.0,
            capacitor: 0.0,
            accent_gain: 1.0,
            gate: false,
            attacking: false,
            accent: false,
        }
    }

    /// Open the gate. A legato slide changes accent without retriggering.
    pub fn note_on(&mut self, accent: bool, slide: bool) {
        self.accent = accent;
        if !(slide && self.gate) {
            self.filter = 1.0;
            self.attacking = true;
        }
        self.gate = true;
    }

    /// Close the VCA gate. The filter envelope and accent sweep continue to decay.
    pub fn note_off(&mut self) {
        self.gate = false;
    }

    /// Advance only the control signals, returning their levels after `frames` samples.
    /// Passing zero reads the current levels.
    #[must_use]
    pub fn advance(&mut self, params: &VoiceParams, frames: usize) -> EnvelopeLevels {
        let coefficients = self.coefficients(params);
        for _ in 0..frames {
            self.step(&coefficients);
        }
        EnvelopeLevels {
            cutoff_hz: self.cutoff_hz(params, params.base_octaves()),
            accent_octaves: self.accent_octaves(params),
            vca_gain: self.amplify(1.0),
        }
    }

    pub(super) fn reset(&mut self) {
        self.capacitor = 0.0;
        self.amplitude = 0.0;
        self.filter = 0.0;
        self.accent = false;
        self.gate = false;
    }

    pub(super) fn sounding(&self) -> bool {
        self.gate || self.amplitude > 0.0001
    }

    pub(super) fn coefficients(&self, p: &VoiceParams) -> Coefficients {
        let decay = if self.accent { 0.2 } else { p.decay_s };
        Coefficients {
            filter_decay: tau_to_coef(decay / 4.6, self.sample_rate),
            attack: tau_to_coef(p.attack_s, self.sample_rate),
            // Resonance slows the drain of the accent sweep capacitor.
            accent_drain: tau_to_coef(0.08 + 0.35 * p.resonance, self.sample_rate),
            accent_gain: if self.accent {
                1.0 + 0.9 * p.accent
            } else {
                1.0
            },
        }
    }

    #[inline]
    pub(super) fn step(&mut self, c: &Coefficients) {
        self.filter -= self.filter * c.filter_decay;
        if self.gate {
            if self.attacking {
                self.amplitude += (1.05 - self.amplitude) * c.attack;
                if self.amplitude >= 1.0 {
                    self.amplitude = 1.0;
                    self.attacking = false;
                }
            } else {
                self.amplitude -= self.amplitude * self.hold;
            }
        } else {
            self.amplitude -= self.amplitude * self.release;
        }
        let input = if self.accent { self.filter } else { 0.0 };
        let rate = if input > self.capacitor {
            self.accent_charge
        } else {
            c.accent_drain
        };
        self.capacitor += (input - self.capacitor) * rate;
        // A slide can switch accent mid-note. Slew its gain to avoid a click.
        self.accent_gain += (c.accent_gain - self.accent_gain) * self.accent_glide;
    }

    fn accent_octaves(&self, p: &VoiceParams) -> f32 {
        3.0 * p.accent * self.capacitor
    }

    pub(super) fn cutoff_hz(&self, p: &VoiceParams, base_octaves: f32) -> f32 {
        let octaves = base_octaves + p.env_oct * self.filter + self.accent_octaves(p);
        // The ladder caps its target to keep the resonant peak below the top of the band.
        clamp(fast_exp2(octaves), 5.0, 16_000.0)
    }

    pub(super) fn amplify(&self, sample: f32) -> f32 {
        sample * self.amplitude * self.accent_gain
    }
}

impl VoiceParams {
    pub(super) fn base_octaves(&self) -> f32 {
        micromath::F32Ext::log2(self.cutoff_hz * 1.6)
    }
}
