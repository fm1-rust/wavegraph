//! Temporary channel number in the upper-left corner of the LCD.

use super::mixer::Band;
use crate::panel::Channel;
use embedded_graphics::{
    Drawable,
    mono_font::{MonoTextStyle, ascii::FONT_9X15_BOLD},
    pixelcolor::{IntoStorage, RgbColor, raw::ToBytes},
    prelude::Point,
    text::{Baseline, Text},
};
use fm1_graphics::{Rgb565, TextGrid, palette};
use fm1_rticx::bsp::display::{Display, Error, SIZE};

// Two 12 × 16 text cells in each direction, anchored at LCD pixel (0, 0).
const W: usize = 24;
const H: usize = 32;
type Pixel = [u8; 2];
type Glyph = [[Pixel; W / 2]; H / 2];

pub(super) struct ChannelBadge {
    channel: Option<Channel>,
    glyph: Glyph,
    underneath: [[Pixel; W]; H],
}

impl ChannelBadge {
    pub(super) const fn new() -> Self {
        Self {
            channel: None,
            glyph: [[[0; 2]; W / 2]; H / 2],
            underneath: [[[0; 2]; W]; H],
        }
    }

    pub(super) fn select(
        &mut self,
        lcd: &mut Display,
        strip: &mut [Pixel],
        grid: &mut TextGrid,
        channel: Option<Channel>,
    ) -> Result<(), Error> {
        if self.channel == channel {
            return Ok(());
        }
        self.channel = channel;
        if let Some(channel) = channel {
            let mut glyph = Band::new(
                self.glyph.as_flattened_mut(),
                0,
                0,
                (W / 2) as u16,
                (H / 2) as u16,
                Rgb565::BLACK,
            );
            Text::with_baseline(
                ["1", "2", "3", "4"][channel.index()],
                Point::new(2, 1),
                MonoTextStyle::new(&FONT_9X15_BOLD, Rgb565::WHITE),
                Baseline::Top,
            )
            .draw(&mut glyph)
            .expect("infallible tile");
            let pixels = &mut strip[..W * H];
            for (y, row) in pixels.chunks_exact_mut(W).enumerate() {
                self.draw_row(0, y, row);
            }
            lcd.write_rect_buffered(0, 0, W as u16, H as u16, pixels.as_flattened())?;
        } else {
            // Invalidate the covered text so TextGrid rewrites the current header.
            for row in 0..2 {
                grid.text(0, row, b"  ", palette::MUTED);
            }
            lcd.write_rect_buffered(
                0,
                0,
                W as u16,
                H as u16,
                self.underneath.as_flattened().as_flattened(),
            )?;
        }
        Ok(())
    }

    pub(super) fn change_view(&mut self, background: Rgb565) {
        self.underneath
            .as_flattened_mut()
            .fill(background.to_be_bytes());
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The covered rows are inside the fill rectangle"
    )]
    pub(super) fn fill_rows(
        &mut self,
        lcd: &mut Display,
        y: u16,
        height: u16,
        color: Rgb565,
    ) -> Result<(), Error> {
        let start = usize::from(y).min(H);
        let end = usize::from(y.saturating_add(height)).min(H);
        self.underneath[start..end]
            .as_flattened_mut()
            .fill(color.to_be_bytes());
        let covered = (end - start) as u16;
        let ink = color.into_storage();
        if covered == 0 || self.channel.is_none() {
            return lcd.fill_rect(0, y, SIZE, height, ink);
        }
        lcd.fill_rect(W as u16, y, SIZE - W as u16, covered, ink)?;
        if covered < height {
            lcd.fill_rect(0, y + covered, SIZE, height - covered, ink)?;
        }
        Ok(())
    }

    // TextGrid writes straight to the LCD, so skip the two cells under the badge.
    pub(super) fn header_text(&self, grid: &mut TextGrid, row: usize, label: &[u8], ink: Rgb565) {
        if self.channel.is_some() {
            grid.text(2, row, label.get(2..).unwrap_or_default(), ink);
        } else {
            grid.text(0, row, label, ink);
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Both rectangles fit the LCD"
    )]
    pub(super) fn compose(&mut self, x: usize, y: usize, width: usize, pixels: &mut [Pixel]) {
        if x >= W || y >= H {
            return;
        }
        let count = width.min(W - x);
        for (dy, row) in pixels.chunks_exact_mut(width).take(H - y).enumerate() {
            let covered = &mut row[..count];
            self.underneath[y + dy][x..x + count].copy_from_slice(covered);
            if self.channel.is_some() {
                self.draw_row(x, y + dy, covered);
            }
        }
    }

    fn draw_row(&self, x: usize, y: usize, pixels: &mut [Pixel]) {
        let source = &self.glyph[y / 2];
        for (x, pixel) in (x..).zip(pixels) {
            *pixel = source[x / 2];
        }
    }
}
