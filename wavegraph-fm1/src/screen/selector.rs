//! A one-bit ink mask over the live scope.

use super::{InstrumentStyle, lettering::GRID, style::RowColors};
use crate::{
    fx::InsertKind,
    instrument::Part,
    panel::{CHANNELS, Channel},
    scope::Window,
};
use core::ops::Range;
use fm1_graphics::embedded_graphics::{
    Drawable, Pixel,
    framebuffer::{Framebuffer, buffer_size},
    pixelcolor::{
        BinaryColor, RgbColor,
        raw::{BigEndian, RawU1, ToBytes},
    },
    prelude::{DrawTarget, Point},
};
use fm1_graphics::{Rgb565, WaveformStyle};

// Leave room for a channel number, nine GRID glyphs (MONOSYNTH or WAVETABLE), and the selection arrow.
const WIDTH: usize = 144;
const ROW_HEIGHT: usize = 18;
const HEIGHT: usize = Part::ALL.len() * ROW_HEIGHT;
pub(super) const LEFT: usize = (crate::capture::WIDTH - WIDTH) / 2;
pub(super) const RIGHT: usize = LEFT + WIDTH;
pub(super) const TOP: usize = (crate::capture::WIDTH - HEIGHT) / 2;
pub(super) const BOTTOM: usize = TOP + HEIGHT;

type Mask<const H: usize, const BYTES: usize> =
    Framebuffer<BinaryColor, RawU1, BigEndian, WIDTH, H, BYTES>;

fn marked(mask: &[u8], x: usize, y: usize) -> bool {
    let i = y * WIDTH + x;
    mask[i / 8] & (0x80 >> (i % 8)) != 0
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Menu {
    Channels {
        parts: [Part; CHANNELS],
        focus: Channel,
    },
    Instruments(Part),
    Effects(InsertKind),
}

impl Menu {
    fn mask_rows(self) -> Range<usize> {
        let count = match self {
            Self::Channels { .. } => CHANNELS,
            Self::Instruments(_) => Part::ALL.len(),
            Self::Effects(_) => InsertKind::ALL.len(),
        };
        let height = count * ROW_HEIGHT;
        let top = (HEIGHT - height) / 2;
        top..top + height
    }
}

pub(super) struct Selector {
    menu: Option<Menu>,
    mask: Mask<HEIGHT, { buffer_size::<BinaryColor>(WIDTH, HEIGHT) }>,
    rows: [RowColors; 2],
    selected_top: usize,
}

impl Selector {
    const GLOBAL: [RowColors; 2] = [
        RowColors::new(Rgb565::BLACK, Rgb565::WHITE),
        RowColors::new(Rgb565::WHITE, Rgb565::BLACK),
    ];

    pub(super) fn new() -> Self {
        Self {
            menu: None,
            mask: Mask::new(),
            rows: Self::GLOBAL,
            selected_top: 0,
        }
    }

    pub(super) fn select(&mut self, menu: Option<Menu>, theme: Option<&InstrumentStyle>) -> bool {
        let rows = theme.map_or(Self::GLOBAL, |t| [t.row(false), t.selected]);
        if self.menu == menu && (menu.is_none() || self.rows == rows) {
            return false;
        }
        self.menu = menu;
        if let Some(menu) = menu {
            self.rows = rows;
            self.mask
                .clear(BinaryColor::Off)
                .expect("infallible framebuffer");
            for (row, y) in menu.mask_rows().step_by(ROW_HEIGHT).enumerate() {
                let (name, active) = match menu {
                    Menu::Channels { parts, focus } => (parts[row].name(), row == focus.index()),
                    Menu::Instruments(active) => {
                        let part = Part::ALL[row];
                        (part.name(), part == active)
                    }
                    Menu::Effects(active) => {
                        let kind = InsertKind::ALL[row];
                        (kind.name(), kind == active)
                    }
                };
                let text_y = y + (ROW_HEIGHT - GRID.height) / 2;
                if active {
                    self.selected_top = y;
                    self.label(WIDTH - GRID.width, text_y, ">");
                }
                let left = if matches!(menu, Menu::Channels { .. }) {
                    self.label(8, text_y, ["1", "2", "3", "4"][row]);
                    24
                } else {
                    8
                };
                self.label(left, text_y, name);
            }
        }
        true
    }

    fn label(&mut self, x: usize, y: usize, text: &str) {
        for (i, byte) in text.bytes().enumerate() {
            let glyph = GRID.glyph(byte);
            let left = x + i * glyph.width;
            for dy in 0..glyph.height {
                for dx in 0..glyph.width {
                    if glyph.coverage(dx, dy) >= 2 {
                        let point = Point::new((left + dx) as i32, (y + dy) as i32);
                        Pixel(point, BinaryColor::On)
                            .draw(&mut self.mask)
                            .expect("infallible framebuffer");
                    }
                }
            }
        }
    }

    pub(super) fn visible(&self) -> bool {
        self.menu.is_some()
    }

    pub(super) fn shade(&self, color: Rgb565) -> Rgb565 {
        self.shader()(color)
    }

    pub(super) fn shader(&self) -> impl Fn(Rgb565) -> Rgb565 + use<> {
        let dimmed = self.visible();
        move |color| {
            if dimmed {
                Rgb565::new(color.r() / 2, color.g() / 2, color.b() / 2)
            } else {
                color
            }
        }
    }

    pub(super) fn style(&self, style: WaveformStyle) -> WaveformStyle {
        WaveformStyle {
            background: self.shade(style.background),
            grid: self.shade(style.grid),
            center: self.shade(style.center),
            trace: self.shade(style.trace),
            ..style
        }
    }

    pub(super) fn pixel(&self, x: usize, y: usize, background: Rgb565) -> Rgb565 {
        if !self.visible() || !(LEFT..RIGHT).contains(&x) || !(TOP..BOTTOM).contains(&y) {
            return background;
        }
        self.ink(x, y).unwrap_or(background)
    }

    pub(super) fn scope_overhang(&self) -> Option<Window> {
        let menu = self.menu?;
        // The insert rack composites its own rows above and below the plot.
        if matches!(menu, Menu::Effects(_)) {
            return None;
        }
        let rows = menu.mask_rows();
        Some(Window {
            x: LEFT,
            y: TOP + rows.start,
            width: WIDTH,
            height: rows.len(),
        })
    }

    pub(super) fn overlay(&self, w: Window, pixels: &mut [[u8; 2]]) {
        if !self.visible() {
            return;
        }
        for y in TOP.max(w.y)..BOTTOM.min(w.y + w.height) {
            for x in LEFT.max(w.x)..RIGHT.min(w.x + w.width) {
                if let Some(ink) = self.ink(x, y) {
                    pixels[(y - w.y) * w.width + x - w.x] = ink.to_be_bytes();
                }
            }
        }
    }

    fn ink(&self, x: usize, y: usize) -> Option<Rgb565> {
        let (x, y) = (x - LEFT, y - TOP);
        if !self.menu?.mask_rows().contains(&y) {
            return None;
        }
        let active = (self.selected_top..self.selected_top + ROW_HEIGHT).contains(&y);
        let row = self.rows[usize::from(active)];
        Some(if marked(self.mask.data(), x, y) {
            row.ink
        } else {
            row.fill
        })
    }
}
