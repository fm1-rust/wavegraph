//! Shared faceplate, control and oscilloscope colors for each instrument.
use crate::{
    instrument::Part,
    scope::{GRID, Stripes},
};
use fm1_graphics::{Rgb565, WaveformStyle, embedded_graphics::pixelcolor::RgbColor};

pub(super) struct InstrumentStyle {
    pub face: Rgb565,
    pub surface: Rgb565,
    pub ink: Rgb565,
    pub muted: Rgb565,
    pub accent: Rgb565,
    pub scope: WaveformStyle,
    pub peak: Rgb565,
    pub stripes: Stripes,
}

type Color = Rgb565;
const fn scope(background: Color, grid: Color, center: Color, trace: Color) -> WaveformStyle {
    WaveformStyle {
        background,
        grid,
        center,
        trace,
        grid_x: GRID,
        grid_y: GRID,
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::new(r, g, b)
}

// Dark phosphor CRT.
pub(super) const MONO: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Normal,
    face: rgb(0, 2, 1),
    surface: rgb(2, 12, 4),
    ink: rgb(27, 62, 28),
    muted: rgb(14, 37, 17),
    accent: rgb(10, 56, 18),
    peak: rgb(26, 63, 23),
    scope: scope(rgb(0, 2, 1), rgb(2, 12, 4), rgb(4, 21, 7), rgb(9, 51, 11)),
};
// M1 charcoal, electric lime and a cool readout.
pub(super) const ORGAN: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Fine,
    face: rgb(4, 7, 8),
    surface: rgb(8, 14, 13),
    ink: rgb(28, 61, 29),
    muted: rgb(17, 36, 23),
    accent: rgb(27, 59, 4),
    peak: rgb(30, 62, 11),
    scope: scope(rgb(4, 7, 8), rgb(7, 13, 9), rgb(11, 21, 10), rgb(23, 50, 4)),
};
// Brushed silver and oxide-red legends.
pub(super) const TB303: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Fine,
    face: rgb(27, 54, 27),
    surface: rgb(23, 47, 23),
    ink: rgb(4, 8, 4),
    muted: rgb(9, 19, 10),
    accent: rgb(17, 6, 3),
    peak: rgb(22, 8, 4),
    scope: scope(
        rgb(24, 49, 24),
        rgb(21, 43, 21),
        rgb(17, 35, 17),
        rgb(9, 10, 7),
    ),
};
// The three independent 303 curves share their legend inks with the sequencer.
pub(super) const ACID_LANES: [Color; 3] = [TB303.ink, TB303.accent, rgb(1, 20, 17)];

// Black chassis and the 808's orange step keys.
pub(super) const TR808: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Wide,
    face: rgb(3, 6, 3),
    surface: rgb(7, 12, 5),
    ink: rgb(30, 53, 20),
    muted: rgb(22, 34, 13),
    accent: rgb(31, 29, 2),
    peak: rgb(31, 51, 13),
    scope: scope(rgb(3, 6, 3), rgb(7, 12, 4), rgb(12, 20, 6), rgb(29, 32, 5)),
};
// Ivory chassis, graphite print, burnt orange switches.
pub(super) const TR909: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Wide,
    face: rgb(29, 57, 26),
    surface: rgb(25, 49, 22),
    ink: rgb(5, 12, 7),
    muted: rgb(10, 22, 12),
    accent: rgb(18, 13, 1),
    peak: rgb(20, 14, 0),
    scope: scope(
        rgb(29, 57, 26),
        rgb(25, 50, 23),
        rgb(20, 40, 18),
        rgb(5, 15, 9),
    ),
};
// Midnight blue with amber legends and a cyan trace.
pub(super) const JP8080: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Fine,
    face: rgb(3, 10, 17),
    surface: rgb(5, 17, 22),
    ink: rgb(26, 54, 30),
    muted: rgb(14, 38, 26),
    accent: rgb(31, 44, 7),
    peak: rgb(25, 63, 31),
    scope: scope(
        rgb(1, 6, 10),
        rgb(3, 15, 18),
        rgb(5, 22, 23),
        rgb(7, 54, 30),
    ),
};
// Microwave I's blue-violet panel and red data wheel.
pub(super) const MICROWAVE: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Normal,
    face: rgb(7, 10, 18),
    surface: rgb(11, 16, 23),
    ink: rgb(29, 57, 31),
    muted: rgb(20, 35, 26),
    accent: rgb(31, 23, 17),
    peak: rgb(31, 47, 24),
    scope: scope(
        rgb(3, 4, 9),
        rgb(7, 10, 16),
        rgb(10, 16, 20),
        rgb(30, 23, 18),
    ),
};
// DX-style brown/turquoise membrane panel and inverse green LCD.
pub(super) const FM: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Normal,
    face: rgb(7, 10, 6),
    surface: rgb(6, 20, 13),
    ink: rgb(30, 59, 27),
    muted: rgb(18, 39, 20),
    accent: rgb(12, 54, 22),
    peak: rgb(2, 6, 4),
    scope: scope(
        rgb(20, 45, 13),
        rgb(18, 41, 12),
        rgb(15, 34, 10),
        rgb(3, 10, 4),
    ),
};
// Emulator II's blue panel and pale legends. The sample readout retains its LCD.
pub(super) const SAMPLER: InstrumentStyle = InstrumentStyle {
    stripes: Stripes::Normal,
    face: rgb(6, 21, 29),
    surface: rgb(4, 15, 23),
    ink: rgb(30, 60, 31),
    muted: rgb(22, 43, 29),
    accent: rgb(19, 54, 31),
    peak: rgb(28, 61, 31),
    scope: scope(
        rgb(2, 7, 13),
        rgb(4, 14, 21),
        rgb(7, 22, 26),
        rgb(16, 45, 31),
    ),
};

pub(super) fn mix(a: Color, b: Color) -> Color {
    rgb(
        a.r().midpoint(b.r()),
        a.g().midpoint(b.g()),
        a.b().midpoint(b.b()),
    )
}

impl InstrumentStyle {
    pub(super) fn for_part(part: Part) -> &'static Self {
        match part {
            Part::Mono => &MONO,
            Part::Organ => &ORGAN,
            Part::Bass => &TB303,
            Part::Tr808 => &TR808,
            Part::Tr909 => &TR909,
            Part::Supersaw => &JP8080,
            Part::Wavetable => &MICROWAVE,
            Part::Fm => &FM,
            Part::Sampler => &SAMPLER,
        }
    }

    pub(super) fn panel(&self) -> WaveformStyle {
        scope(
            self.face,
            self.surface,
            mix(self.surface, self.muted),
            self.accent,
        )
    }

    pub(super) fn phosphor(&self, age: usize) -> (Color, Option<Color>) {
        let trace = self.scope.trace;
        match age {
            0..=3 => (self.peak, Some(mix(self.scope.background, self.peak))),
            4..=12 => (mix(self.peak, trace), None),
            _ => (trace, None),
        }
    }
}
