//! Dirty-cell bookkeeping adapted from fm1-graphics 0.2.0-rc.0 (MIT OR Apache-2.0).
use super::style::RowColors;
use core::ops::Range;
use fm1_graphics::embedded_graphics::pixelcolor::raw::ToBytes;
use fm1_graphics::{Rgb565, palette};
use fm1_rticx::bsp::display::{Display, Error, SIZE};
use fm1_rticx::time::Clock;

const COLS: usize = TextGrid::COLUMNS;
const ROWS: usize = TextGrid::ROWS;
const CELL_W: u16 = 12;
const CELL_H: u16 = 16;
type CellPixels = [[[u8; 2]; CELL_W as usize]; CELL_H as usize];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    ch: u8,
    ink: Rgb565,
}
const BLANK: Cell = Cell {
    ch: b' ',
    ink: palette::MUTED,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Highlight {
    row: usize,
    mask: u32,
    colors: RowColors,
}

/// Fixed dashboard cells with native-size panel lettering.
#[derive(Debug)]
pub struct TextGrid {
    cells: [[Cell; COLS]; ROWS],
    dirty: [u32; ROWS],
    background: Rgb565,
    highlight: Option<Highlight>,
}

impl TextGrid {
    /// Number of character columns on the display.
    pub const COLUMNS: usize = (SIZE / CELL_W) as usize;
    /// Number of character rows on the display.
    pub const ROWS: usize = (SIZE / CELL_H) as usize;

    /// Start with a blank, clean grid over an existing background of this color.
    #[must_use]
    pub const fn with_background(background: Rgb565) -> Self {
        Self {
            cells: [[BLANK; COLS]; ROWS],
            dirty: [0; ROWS],
            background,
            highlight: None,
        }
    }

    /// Update a row of ASCII text, clipping at the right edge. Coordinates
    /// outside the grid are ignored.
    pub fn text(&mut self, x: usize, y: usize, text: impl AsRef<[u8]>, ink: Rgb565) {
        let Some(cells) = self.cells.get_mut(y).and_then(|row| row.get_mut(x..)) else {
            return;
        };
        for (column, (cell, &ch)) in (x..).zip(cells.iter_mut().zip(text.as_ref())) {
            let next = Cell {
                ch: ch.to_ascii_uppercase(),
                ink,
            };
            if *cell != next {
                *cell = next;
                self.dirty[y] |= 1 << column;
            }
        }
    }

    /// Move the selected field, repainting both the old and new span of cells.
    pub(super) fn highlight(&mut self, row: usize, columns: Range<usize>, colors: RowColors) {
        debug_assert!(row < ROWS && columns.start <= columns.end && columns.end <= COLS);
        let mask = (1_u32 << columns.end) - (1_u32 << columns.start);
        let next = Highlight { row, mask, colors };
        if self.highlight != Some(next) {
            if let Some(old) = self.highlight.replace(next) {
                self.dirty[old.row] |= old.mask;
            }
            self.dirty[row] |= mask;
        }
    }

    /// Write a right-aligned unsigned decimal field, padding with spaces.
    /// Width is capped at the number of columns. Overflow fills the field with stars.
    pub fn number(&mut self, x: usize, y: usize, width: usize, value: u32, ink: Rgb565) {
        self.decimal(x, y, width, value, None, ink);
    }

    /// Write a right-aligned decimal field with an explicit `+` or `-` sign.
    /// Padding and overflow follow [`Self::number`].
    pub fn signed_number(&mut self, x: usize, y: usize, width: usize, value: i32, ink: Rgb565) {
        self.decimal(
            x,
            y,
            width,
            value.unsigned_abs(),
            Some(if value < 0 { b'-' } else { b'+' }),
            ink,
        );
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Each ASCII digit is in b'0'..=b'9'."
    )]
    fn decimal(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        mut n: u32,
        sign: Option<u8>,
        ink: Rgb565,
    ) {
        let mut buf = [b' '; COLS];
        let field = &mut buf[..width.min(COLS)];
        let mut slots = field.iter_mut().rev();
        loop {
            let Some(slot) = slots.next() else {
                field.fill(b'*');
                break;
            };
            *slot = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                if let Some(sign) = sign {
                    if let Some(slot) = slots.next() {
                        *slot = sign;
                    } else {
                        field.fill(b'*');
                    }
                }
                break;
            }
        }
        self.text(x, y, field, ink);
    }

    // An error or cancellation leaves the dirty cells pending.
    pub async fn paint_async(&mut self, lcd: &mut Display, delay: &mut Clock) -> Result<(), Error> {
        if self.dirty == [0; ROWS] {
            return Ok(());
        }
        let mut lcd = super::PanelDisplay::new(lcd, delay, super::Viewport::Full)?;
        self.paint_panel(&mut lcd).await
    }

    pub(super) async fn paint_panel(
        &mut self,
        lcd: &mut super::PanelDisplay<'_>,
    ) -> Result<(), Error> {
        for (x, y, ch, colors) in self.dirty_cells() {
            let mask = super::lettering::GRID.glyph(ch);
            let mut pixels: CellPixels =
                [[colors.fill.to_be_bytes(); CELL_W as usize]; CELL_H as usize];
            for (row, line) in pixels.iter_mut().enumerate() {
                for (col, pixel) in line.iter_mut().enumerate() {
                    super::lettering::blend(pixel, colors.ink, mask.coverage(col, row));
                }
            }
            lcd.write_rect(x, y, CELL_W, CELL_H, pixels.as_flattened_mut())
                .await?;
        }
        lcd.flush().await?;
        self.dirty.fill(0);
        Ok(())
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::arithmetic_side_effects,
        reason = "The 20 by 15 grid fits the 240-pixel panel on both axes."
    )]
    fn dirty_cells(&self) -> impl Iterator<Item = (u16, u16, u8, RowColors)> {
        self.dirty.iter().enumerate().flat_map(move |(row, &mask)| {
            let mut pending = mask;
            core::iter::from_fn(move || {
                if pending == 0 {
                    return None;
                }
                let col = pending.trailing_zeros() as usize;
                pending &= !(1 << col);
                let cell = self.cells[row][col];
                let colors = self
                    .highlight
                    .filter(|h| h.row == row && h.mask & (1 << col) != 0)
                    .map_or(RowColors::new(self.background, cell.ink), |h| h.colors);
                Some((col as u16 * CELL_W, row as u16 * CELL_H, cell.ch, colors))
            })
        })
    }
}
