//! Temporary channel number in the upper-left corner of the LCD.

use super::TextGrid;
use crate::panel::Channel;
use core::ops::Range;
use embedded_graphics::pixelcolor::{IntoStorage, RgbColor, raw::ToBytes};
use fm1_graphics::{Rgb565, palette};
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Display, Error, SIZE},
    time::Clock,
};

// Two 12 × 16 text cells in each direction, anchored at LCD pixel (0, 0).
const W: usize = 24;
const H: usize = 32;
type Pixel = [u8; 2];

pub(super) struct ChannelBadge {
    channel: Option<Channel>,
    underneath: [[Pixel; W]; H],
}

impl ChannelBadge {
    pub(super) const fn new() -> Self {
        Self {
            channel: None,
            underneath: [[[0; 2]; W]; H],
        }
    }

    pub(super) async fn select(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        strip: &mut [Pixel],
        grid: &mut TextGrid,
        channel: Option<Channel>,
    ) -> Result<(), Error> {
        if self.channel == channel {
            return Ok(());
        }
        let mut lcd = lcd.as_async(clock)?;
        self.channel = channel;
        if channel.is_some() {
            let pixels = &mut strip[..W * H];
            for (y, row) in pixels.chunks_exact_mut(W).enumerate() {
                self.draw_row(0, y, row);
            }
            lcd.write_rect_buffered(0, 0, W as u16, H as u16, pixels.as_flattened())
                .await?;
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
            )
            .await?;
        }
        lcd.flush().await
    }

    pub(super) fn change_view(&mut self, background: Rgb565) {
        self.underneath
            .as_flattened_mut()
            .fill(background.to_be_bytes());
    }

    pub(super) async fn fill_rows(
        &mut self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        mut rows: Range<u16>,
        color: Rgb565,
    ) -> Result<(), Error> {
        let start = usize::from(rows.start).min(H);
        let end = usize::from(rows.end).min(H);
        self.underneath[start..end]
            .as_flattened_mut()
            .fill(color.to_be_bytes());
        if self.channel.is_some() && start < end {
            let below = end as u16;
            lcd.fill_rect(
                W as u16,
                rows.start,
                SIZE - W as u16,
                below - rows.start,
                color.into_storage(),
            )
            .await?;
            rows.start = below;
        }
        if !rows.is_empty() {
            lcd.fill_rect(
                0,
                rows.start,
                SIZE,
                rows.end - rows.start,
                color.into_storage(),
            )
            .await?;
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
        let mask = super::lettering::BADGE.frame(self.channel.expect("visible badge").index());
        for (x, pixel) in (x..).zip(pixels) {
            *pixel = [0; 2];
            super::lettering::blend(pixel, Rgb565::WHITE, mask.coverage(x, y));
        }
    }
}
