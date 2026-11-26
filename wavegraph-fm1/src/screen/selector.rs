//! Picker rows composited over the live scope from flash-backed GRID glyphs.

use super::{InstrumentStyle, lettering::GRID, style::RowColors};
use crate::{
    fx::InsertKind,
    instrument::Part,
    panel::{CHANNELS, Channel},
    scope::Window,
};
use core::ops::Range;
use fm1_graphics::embedded_graphics::pixelcolor::{RgbColor, raw::ToBytes};
use fm1_graphics::{Rgb565, WaveformStyle};
use fm1_rticx::{
    bsp::display::{Display, Error},
    time::Clock,
};

// Leave room for a channel number, nine GRID glyphs (MONOSYNTH or WAVETABLE), and the selection arrow.
const WIDTH: usize = 144;
const ROW_HEIGHT: usize = 18;
const HEIGHT: usize = Part::ALL.len() * ROW_HEIGHT;
pub(super) const LEFT: usize = (crate::capture::WIDTH - WIDTH) / 2;
pub(super) const RIGHT: usize = LEFT + WIDTH;
pub(super) const TOP: usize = (crate::capture::WIDTH - HEIGHT) / 2;
pub(super) const BOTTOM: usize = TOP + HEIGHT;

fn letter_ink(label: &[u8], x: usize, y: usize) -> bool {
    label
        .get(x / GRID.width)
        .is_some_and(|&byte| GRID.glyph(byte).coverage(x % GRID.width, y) >= 2)
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
    labels: [&'static [u8]; Part::ALL.len()],
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
            labels: [&[]; Part::ALL.len()],
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
                self.labels[row] = name.as_bytes();
                if active {
                    self.selected_top = y;
                }
            }
        }
        true
    }

    pub(super) fn visible(&self) -> bool {
        self.menu.is_some()
    }

    pub(super) fn instrument(&self) -> Option<Part> {
        match self.menu {
            Some(Menu::Instruments(selected)) => Some(selected),
            _ => None,
        }
    }

    /// The live instrument draws around this opaque picker. A detent only
    /// changes the previously selected row and the new one.
    pub(super) async fn paint_instruments(
        &self,
        lcd: &mut Display,
        clock: &mut Clock,
        strip: &mut [[u8; 2]],
        previous: Option<Part>,
        selected: Part,
    ) -> Result<(), Error> {
        let mut lcd = lcd.as_async(clock)?;
        for (i, part) in Part::ALL.into_iter().enumerate() {
            if previous.is_some_and(|old| part != old && part != selected) {
                continue;
            }
            let row = Window::new(LEFT, TOP + i * ROW_HEIGHT, WIDTH, ROW_HEIGHT);
            for w in row.strips(strip.len()) {
                let pixels = &mut strip[..w.width * w.height];
                self.overlay(w, pixels);
                lcd.write_rect_buffered(
                    w.x as u16,
                    w.y as u16,
                    w.width as u16,
                    w.height as u16,
                    pixels.as_flattened(),
                )
                .await?;
            }
        }
        lcd.flush().await
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
        let menu = self.menu?;
        let rows = menu.mask_rows();
        if !rows.contains(&y) {
            return None;
        }
        let active = (self.selected_top..self.selected_top + ROW_HEIGHT).contains(&y);
        let row = self.rows[usize::from(active)];
        let local = y - rows.start;
        let index = local / ROW_HEIGHT;
        let text_y = (local % ROW_HEIGHT).wrapping_sub((ROW_HEIGHT - GRID.height) / 2);
        let channels = matches!(menu, Menu::Channels { .. });
        let left = if channels { 24 } else { 8 };
        let marked = text_y < GRID.height
            && (letter_ink(self.labels[index], x.wrapping_sub(left), text_y)
                || (active && letter_ink(b">", x.wrapping_sub(WIDTH - GRID.width), text_y))
                || (channels && letter_ink(&b"1234"[index..=index], x.wrapping_sub(8), text_y)));
        Some(if marked { row.ink } else { row.fill })
    }
}
