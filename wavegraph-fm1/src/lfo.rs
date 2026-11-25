//! Per-channel modulation routing. The oscillator emits one bipolar float per audio block.
use crate::instrument::{Part, Track};
use wavegraph_303::Destination;
use wavegraph_lfo::{Lfo, Shape};

const SHAPES: [Shape; 4] = [Shape::Sine, Shape::Triangle, Shape::Saw, Shape::Square];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Route {
    Bass(Destination),
    Knob { index: u8, span: u8 },
}

#[derive(Clone, Copy)]
pub(super) struct Target {
    pub(super) name: &'static str,
    pub(super) route: Route,
}

const fn knob(name: &'static str, index: u8, span: u8) -> Target {
    Target {
        name,
        route: Route::Knob { index, span },
    }
}

const fn bass(name: &'static str, target: Destination) -> Target {
    Target {
        name,
        route: Route::Bass(target),
    }
}

const BASS: &[Target] = &[
    bass("PITCH", Destination::Pitch),
    bass("CUTOFF", Destination::Cutoff),
    bass("RESONANCE", Destination::Resonance),
    bass("ENV MOD", Destination::EnvMod),
    bass("DECAY", Destination::Decay),
    bass("ACCENT", Destination::Accent),
    bass("ATTACK", Destination::Attack),
    bass("OVERDRIVE", Destination::Overdrive),
];
const MONO: &[Target] = &[knob("CUTOFF", 0, 18)];
const ORGAN: &[Target] = &[
    knob("TONE", 0, 64),
    knob("PERCUSSION", 1, 64),
    knob("CLICK DECAY", 2, 40),
    knob("RELEASE", 3, 64),
];
const TR808: &[Target] = &[
    knob("KICK TONE", 0, 64),
    knob("KICK DECAY", 1, 64),
    knob("SNARE TONE", 2, 64),
    knob("SNAPPY", 3, 64),
];
const TR909: &[Target] = &[
    knob("KICK DECAY", 0, 50),
    knob("SNARE DECAY", 1, 50),
    knob("CLAP DECAY", 2, 50),
];
const SUPERSAW: &[Target] = &[
    knob("CUTOFF", 0, 18),
    knob("RESONANCE", 1, 64),
    knob("DETUNE", 2, 64),
    knob("SAW MIX", 3, 64),
];
const WAVETABLE: &[Target] = &[
    knob("SCAN START", 0, 64),
    knob("SCAN END", 1, 64),
    knob("TIME / ATTACK", 2, 16),
    knob("SCAN MODE", 3, 2),
];
const HOOVER: &[Target] = &[
    knob("CUTOFF", 0, 64),
    knob("PITCH SWEEP", 1, 64),
    knob("PWM DEPTH", 2, 64),
    knob("CHORUS RATE", 3, 64),
];
const FM: &[Target] = &[
    knob("MOD INDEX", 1, 64),
    knob("FEEDBACK", 3, 64),
    knob("RATIO", 0, 3),
    knob("MOD DECAY", 2, 8),
];
const SAMPLER: &[Target] = &[
    knob("VCF CUTOFF", 0, 18),
    knob("RESONANCE", 1, 64),
    knob("ENV AMOUNT", 2, 24),
    knob("VELOCITY", 3, 24),
];

impl Part {
    pub(super) const fn lfo_targets(self) -> &'static [Target] {
        match self {
            Self::Bass => BASS,
            Self::Mono => MONO,
            Self::Organ => ORGAN,
            Self::Tr808 => TR808,
            Self::Tr909 => TR909,
            Self::Supersaw => SUPERSAW,
            Self::Wavetable => WAVETABLE,
            Self::Hoover => HOOVER,
            Self::Fm => FM,
            Self::Sampler => SAMPLER,
        }
    }
}

// Tenths of a Hz; finer steps at slow rates.
const RATES: [u8; 21] = [
    1, 2, 3, 4, 5, 7, 10, 15, 20, 25, 30, 35, 40, 45, 50, 60, 70, 80, 100, 150, 200,
];
// Number of sixteenth notes per LFO cycle, spanning two bars to one sixteenth.
const LENGTHS: [u8; 6] = [32, 16, 8, 4, 2, 1];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    pub enabled: bool,
    pub depth: u8,
    pub sync: bool,
    shape: u8,
    target: u8,
    rate: u8,
    division: u8,
}

impl Controls {
    pub const INITIAL: Self = Self {
        enabled: false,
        depth: 32,
        sync: false,
        shape: 0,
        target: 0,
        rate: 14,
        division: 3,
    };

    pub fn edit(
        &mut self,
        part: Part,
        select: i8,
        [shape, rate, depth, target]: [i8; 4],
        toggle: bool,
    ) {
        if select != 0 {
            self.sync = select > 0;
        }
        self.enabled ^= toggle;
        self.shape = self.shape.saturating_add_signed(shape).min(3);
        if self.sync {
            self.division = self.division.saturating_add_signed(rate).min(5);
        } else {
            self.rate = self.rate.saturating_add_signed(rate).min(20);
        }
        self.depth = self.depth.saturating_add_signed(depth).min(127);
        self.target = self.target.saturating_add_signed(target).min(
            u8::try_from(part.lfo_targets().len().saturating_sub(1)).expect("small target bank"),
        );
    }

    pub fn shape(self) -> Shape {
        SHAPES[usize::from(self.shape)]
    }
    pub fn target(self, part: Part) -> Target {
        part.lfo_targets()[usize::from(self.target)]
    }

    pub fn tick(self, clock: &mut Lfo, bpm: u16, restart: bool, frames: usize) -> Option<f32> {
        if !self.enabled {
            return None;
        }
        if restart {
            clock.reset();
        }
        clock.set_frequency(self.frequency(bpm));
        clock.shape = self.shape();
        clock
            .tick(frames)
            .map(|e| e.message * (f32::from(self.depth) / 127.0))
    }
    pub fn tenths_hz(self) -> u8 {
        RATES[usize::from(self.rate)]
    }
    pub fn frequency(self, bpm: u16) -> f32 {
        if self.sync {
            f32::from(bpm) / (15.0 * f32::from(LENGTHS[usize::from(self.division)]))
        } else {
            f32::from(self.tenths_hz()) * 0.1
        }
    }

    #[cfg(target_os = "none")]
    pub fn division_name(self) -> &'static str {
        ["2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16"][usize::from(self.division)]
    }
}

/// Render-time offset in the same units as the physical encoder. The track's
/// stored program is always the center, even if edited while the LFO runs.
#[derive(Clone, Copy, Default)]
pub(super) struct KnobOffset {
    index: u8,
    detents: i8,
}

impl KnobOffset {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Bipolar signal times a bounded encoder span fits i8."
    )]
    pub(super) fn new(index: u8, span: u8, amount: f32) -> Self {
        Self {
            index,
            detents: (amount * f32::from(span) + 0.5_f32.copysign(amount)) as i8,
        }
    }

    pub(super) fn apply(self, track: &mut Track) {
        if self.detents != 0 {
            let mut knobs = [0; 4];
            knobs[usize::from(self.index)] = self.detents;
            track.turn_knobs(knobs);
        }
    }
}
