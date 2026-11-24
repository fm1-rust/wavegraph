//! Per-channel TB-303 modulation controls.
use wavegraph_303::Destination;
use wavegraph_lfo::Shape;

const SHAPES: [Shape; 4] = [Shape::Sine, Shape::Triangle, Shape::Saw, Shape::Square];
const TARGETS: [Destination; 8] = [
    Destination::Pitch,
    Destination::Cutoff,
    Destination::Resonance,
    Destination::EnvMod,
    Destination::Decay,
    Destination::Accent,
    Destination::Attack,
    Destination::Overdrive,
];
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

    pub fn edit(&mut self, select: i8, [shape, rate, depth, target]: [i8; 4], toggle: bool) {
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
        self.target = self.target.saturating_add_signed(target).min(7);
    }

    pub fn shape(self) -> Shape {
        SHAPES[usize::from(self.shape)]
    }
    pub fn target(self) -> Destination {
        TARGETS[usize::from(self.target)]
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
