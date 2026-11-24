//! Envelope plots share the screen's pixel strip. Each keeps only its last trace.
use super::{BAND, Clock, Display, Error, WaveformStyle};
use fm1_graphics::embedded_graphics::pixelcolor::raw::ToBytes;

pub(super) struct Plot<const W: usize, const H: usize> {
    last: [u8; W],
    valid: bool,
}

#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Both plot dimensions and the origin fit the 240-pixel LCD."
)]
impl<const W: usize, const H: usize> Plot<W, H> {
    const TILE: usize = 16;

    pub(super) const fn new() -> Self {
        Self {
            last: [0; W],
            valid: false,
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.valid = false;
    }

    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]],
        origin: (u16, u16),
        style: WaveformStyle,
        trace: &[u8; W],
    ) -> Result<(), Error> {
        if self.valid && self.last == *trace {
            return Ok(());
        }
        let mut dirty = [0_u16; super::WIDTH.div_ceil(BAND)];
        let bands = &mut dirty[..H.div_ceil(BAND)];
        let segment = |line: &[u8; W], x: usize| {
            let a = usize::from(line[x.saturating_sub(1)]).min(H - 1);
            let b = usize::from(line[x]).min(H - 1);
            a.min(b)..=a.max(b)
        };
        if self.valid {
            for x in 0..W {
                let old = segment(&self.last, x);
                let new = segment(trace, x);
                if old != new {
                    for span in [old, new] {
                        for mask in &mut bands[span.start() / BAND..=span.end() / BAND] {
                            *mask |= 1 << (x / Self::TILE);
                        }
                    }
                }
            }
        } else {
            bands.fill((1 << W.div_ceil(Self::TILE)) - 1);
        }
        self.invalidate();
        let mut lcd = lcd.as_async(clock)?;
        for (band, &mask) in bands.iter().enumerate() {
            let y = band * BAND;
            let h = BAND.min(H - y);
            let mut mask = mask;
            while mask != 0 {
                let tile = mask.trailing_zeros() as usize;
                let count = (mask >> tile).trailing_ones() as usize;
                mask &= !(((1 << count) - 1) << tile);
                let x = tile * Self::TILE;
                let w = (count * Self::TILE).min(W - x);
                let strip = &mut pixels[..w * h];
                for dx in 0..w {
                    let span = segment(trace, x + dx);
                    let grid =
                        style.grid_x != 0 && (x + dx).is_multiple_of(usize::from(style.grid_x));
                    for dy in 0..h {
                        let row = y + dy;
                        let color = if span.contains(&row) {
                            style.trace
                        } else if row == H / 2 {
                            style.center
                        } else if grid
                            || (style.grid_y != 0 && row.is_multiple_of(usize::from(style.grid_y)))
                        {
                            style.grid
                        } else {
                            style.background
                        };
                        strip[dy * w + dx] = color.to_be_bytes();
                    }
                }
                lcd.write_rect_buffered(
                    origin.0 + x as u16,
                    origin.1 + y as u16,
                    w as u16,
                    h as u16,
                    strip.as_flattened(),
                )
                .await?;
            }
        }
        lcd.flush().await?;
        self.last = *trace;
        self.valid = true;
        Ok(())
    }
}
