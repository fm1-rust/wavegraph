//! Picker rows composited over the live scope from flash-backed GRID glyphs.

use super::{
    InstrumentStyle,
    lettering::{GRID, picker_row},
    style::RowColors,
};
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

// Leave room for a channel number, nine GRID glyphs and the selection arrow.
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
        .is_some_and(|&byte| picker_row(byte, y) & (0x8000 >> (x % GRID.width)) != 0)
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Menu {
    Instruments(Part),
    Effects(InsertKind),
}

impl Menu {
    fn mask_rows(self) -> Range<usize> {
        let count = match self {
            Self::Instruments(_) => Part::ALL.len(),
            Self::Effects(_) => InsertKind::ALL.len(),
        };
        let height = count * ROW_HEIGHT;
        let top = (HEIGHT - height) / 2;
        top..top + height
    }
}

pub(super) struct Selector {
    channel_feedback: bool,
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
            channel_feedback: false,
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

    pub(super) fn set_channel_feedback(&mut self, visible: bool) {
        self.channel_feedback = visible;
    }

    pub(super) fn visible(&self) -> bool {
        self.menu.is_some() || self.channel_feedback
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
        if self.menu.is_none() || !(LEFT..RIGHT).contains(&x) || !(TOP..BOTTOM).contains(&y) {
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
        if self.menu.is_none() {
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
        let marked = text_y < GRID.height
            && (letter_ink(self.labels[index], x.wrapping_sub(8), text_y)
                || (active && letter_ink(b">", x.wrapping_sub(WIDTH - GRID.width), text_y)));
        Some(if marked { row.ink } else { row.fill })
    }
}

/// The channel picker uses the same GRID lettering and instrument colors as the other pickers.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct ChannelHud {
    pub(super) focus: Channel,
    pub(super) parts: [Part; CHANNELS],
}

impl ChannelHud {
    pub(super) const AREA: Window = Window::new(
        LEFT,
        (crate::capture::WIDTH - CHANNELS * ROW_HEIGHT) / 2,
        WIDTH,
        CHANNELS * ROW_HEIGHT,
    );

    fn row(self, index: usize) -> (Part, bool, RowColors) {
        let active = index == self.focus.index();
        let theme = InstrumentStyle::for_part(self.parts[self.focus.index()]);
        (self.parts[index], active, theme.row(active))
    }

    pub(super) async fn paint(
        self,
        lcd: &mut Display,
        clock: &mut Clock,
        strip: &mut [[u8; 2]],
        previous: Option<Self>,
    ) -> Result<(), Error> {
        let mut lcd = lcd.as_async(clock)?;
        let margin = (ROW_HEIGHT - GRID.height) / 2;
        for i in 0..CHANNELS {
            let (part, active, colors) = self.row(i);
            if previous.is_some_and(|old| old.row(i) == (part, active, colors)) {
                continue;
            }
            let row = Window::new(LEFT, Self::AREA.y + i * ROW_HEIGHT, WIDTH, ROW_HEIGHT);
            let ink = colors.ink.to_be_bytes();
            for w in row.strips(strip.len()) {
                let pixels = &mut strip[..w.width * w.height];
                pixels.fill(colors.fill.to_be_bytes());
                for (dy, line) in pixels.chunks_exact_mut(WIDTH).enumerate() {
                    let y = w.y - row.y + dy;
                    if (margin..margin + GRID.height).contains(&y) {
                        let glyph_y = y - margin;
                        draw_letters(&mut line[8..], &b"1234"[i..=i], glyph_y, ink);
                        draw_letters(&mut line[24..], part.name().as_bytes(), glyph_y, ink);
                        if active {
                            draw_letters(&mut line[WIDTH - GRID.width..], b">", glyph_y, ink);
                        }
                    }
                }
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
}

/// Set only the ink pixels from the precomputed 12-bit glyph scanlines.
fn draw_letters(line: &mut [[u8; 2]], text: &[u8], y: usize, ink: [u8; 2]) {
    for (cell, &byte) in line.chunks_exact_mut(GRID.width).zip(text) {
        let mut bits = picker_row(byte, y);
        while bits != 0 {
            let x = 15 - bits.trailing_zeros() as usize;
            cell[x] = ink;
            bits &= bits - 1;
        }
    }
}
