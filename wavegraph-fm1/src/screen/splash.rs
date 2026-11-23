//! Boot artwork composited over the scope in buffered LCD strips.

use super::{STYLE, Screen, paper, segment};
use crate::capture::{Trace, WIDTH};
use embedded_graphics::{
    Drawable,
    framebuffer::{Framebuffer, buffer_size},
    geometry::Point,
    mono_font::{MonoTextStyle, ascii::FONT_6X9},
    pixelcolor::{
        Gray2,
        raw::{BigEndian, RawU2, ToBytes},
    },
    text::{Baseline, Text},
};
use fm1_graphics::{Rgb565, WaveformStyle, palette};
use fm1_rticx::{
    bsp::{
        display::{Display, Error, SIZE},
        time::ticks,
    },
    time::{Clock, ExtU32, Timebase},
};
use micromath::F32Ext;

const BOOT_STYLE: WaveformStyle = WaveformStyle {
    trace: Rgb565::new(3, 34, 8),
    ..STYLE
};
const TITLE_Y: i16 = (SIZE as i16 - TITLE.height()) / 2;
const MODEL_Y: i16 = TITLE_Y + TITLE.height() + 4;
const VERSION_Y: i16 = MODEL_Y + MODEL.height() + 5;
const DROP_TICKS: u32 = 52;
const RISE_TICKS: u32 = 30;

struct Sprite<'a> {
    width: usize,
    data: &'a [u8], // Four 2-bit pixels per byte, high bits first. Index zero is transparent.
    inks: [Rgb565; 3],
}

const TITLE: Sprite<'_> = Sprite {
    width: 208,
    data: include_bytes!("../../assets/wordmark.2bpp"),
    inks: [
        Rgb565::new(0, 17, 4),
        Rgb565::new(0, 35, 7),
        Rgb565::new(6, 63, 15),
    ],
};
const MODEL: Sprite<'_> = Sprite {
    width: TITLE.width / 2,
    data: include_bytes!("../../assets/fm1.2bpp"),
    inks: [
        Rgb565::new(16, 1, 10),
        Rgb565::new(31, 2, 18),
        Rgb565::new(31, 29, 26),
    ],
};

// Cubic smoothstep has zero velocity at both ends of a leg. The clock runs at 100 Hz.
fn ease(from: i16, to: i16, elapsed: u32, duration: u32) -> i16 {
    let weight = elapsed * elapsed * (3 * duration - 2 * elapsed);
    from + (i32::from(to - from) * weight as i32 / duration.pow(3) as i32) as i16
}

fn title_y(tick: u32) -> i16 {
    match tick {
        0..26 => ease(-TITLE.height(), TITLE_Y + 24, tick, 26),
        26..39 => ease(TITLE_Y + 24, TITLE_Y - 10, tick - 26, 13),
        39..47 => ease(TITLE_Y - 10, TITLE_Y + 3, tick - 39, 8),
        47..DROP_TICKS => ease(TITLE_Y + 3, TITLE_Y, tick - 47, DROP_TICKS - 47),
        _ => TITLE_Y,
    }
}

impl Screen {
    pub(crate) async fn splash(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        title_landed: impl FnOnce(),
    ) -> Result<(), Error> {
        // A phase-modulated sine with a second harmonic. One sample per LCD column.
        let trace: Trace = core::array::from_fn(|x| {
            let t = x as f32 / WIDTH as f32;
            let phase = 1.8 * t + 0.14 * (3.0 * t).sin_turns();
            (120.0 - 61.0 * phase.sin_turns() - 15.0 * (2.0 * phase).sin_turns()) as u8
        });
        self.plot.set_style(BOOT_STYLE);
        self.plot.paint_async(lcd, &trace, clock).await?;
        TITLE
            .animate(lcd, clock, self.strip, &trace, DROP_TICKS, title_y)
            .await?;
        title_landed();
        MODEL
            .animate(lcd, clock, self.strip, &trace, RISE_TICKS, |t| {
                ease(SIZE as i16, MODEL_Y, t, RISE_TICKS)
            })
            .await?;
        draw_version(lcd, clock, self.strip, &trace).await?;
        Clock::delay(500.millis()).await;
        Ok(())
    }
}

impl Sprite<'_> {
    const fn height(&self) -> i16 {
        (self.data.len() / (self.width / 4)) as i16
    }

    async fn animate(
        &self,
        lcd: &mut Display,
        clock: &mut Clock,
        strip: &mut [[u8; 2]],
        trace: &Trace,
        duration: u32,
        position: fn(u32) -> i16,
    ) -> Result<(), Error> {
        let mut previous = position(0);
        let start = ticks();
        loop {
            let elapsed = ticks().wrapping_sub(start).min(duration);
            let top = position(elapsed);
            if top != previous {
                self.draw(lcd, clock, strip, trace, previous, top).await?;
                previous = top;
            }
            if elapsed == duration {
                return Ok(());
            }
            Clock::delay(10.millis()).await;
        }
    }

    // Rebuild the trace under the old and new positions in the live scope's scratch strip.
    async fn draw(
        &self,
        lcd: &mut Display,
        clock: &mut Clock,
        strip: &mut [[u8; 2]],
        trace: &Trace,
        previous: i16,
        top: i16,
    ) -> Result<(), Error> {
        let width = self.width;
        let height = self.height();
        let left = (WIDTH - width) / 2;
        let first = previous.min(top).max(0) as usize;
        let end = (previous.max(top) + height).clamp(0, SIZE as i16) as usize;
        let rows = strip.len() / width;
        let mut lcd = lcd.as_async(clock)?;
        for start in (first..end).step_by(rows) {
            let count = rows.min(end - start);
            let pixels = &mut strip[..count * width];
            for dx in 0..width {
                let x = left + dx;
                let (a, b) = segment(trace, x);
                for dy in 0..count {
                    let y = start + dy;
                    let sy = y as i16 - top;
                    let ink = if (0..height).contains(&sy) {
                        let packed = self.data[sy as usize * (width / 4) + dx / 4];
                        (packed >> (6 - 2 * (dx % 4))) & 3
                    } else {
                        0
                    };
                    let color = if ink != 0 {
                        self.inks[usize::from(ink - 1)]
                    } else if (a..=b).contains(&y) {
                        BOOT_STYLE.trace
                    } else {
                        paper(BOOT_STYLE, x, y)
                    };
                    pixels[dy * width + dx] = color.to_be_bytes();
                }
            }
            lcd.write_rect_buffered(
                left as u16,
                start as u16,
                width as u16,
                count as u16,
                pixels.as_flattened(),
            )
            .await?;
        }
        lcd.flush().await
    }
}

async fn draw_version(
    lcd: &mut Display,
    clock: &mut Clock,
    strip: &mut [[u8; 2]],
    trace: &Trace,
) -> Result<(), Error> {
    const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
    const GLYPH_W: usize = FONT_6X9.character_size.width as usize;
    const GLYPH_H: usize = FONT_6X9.character_size.height as usize;
    // The packed sprite needs complete bytes. Longer versions wrap at the LCD edge.
    const LINE_W: usize = {
        let width = (VERSION.len() * GLYPH_W).next_multiple_of(4);
        if width < WIDTH { width } else { WIDTH }
    };
    type Line = Framebuffer<
        Gray2,
        RawU2,
        BigEndian,
        LINE_W,
        GLYPH_H,
        { buffer_size::<Gray2>(LINE_W, GLYPH_H) },
    >;
    let style = MonoTextStyle::new(&FONT_6X9, Gray2::new(3));
    for (i, chunk) in VERSION.as_bytes().chunks(LINE_W / GLYPH_W).enumerate() {
        let y = VERSION_Y as usize + i * (GLYPH_H + 1);
        if y + GLYPH_H > SIZE as usize {
            break;
        }
        let mut line = Line::new();
        let x = (LINE_W - chunk.len() * GLYPH_W) / 2;
        let text = core::str::from_utf8(chunk).expect("Cargo versions are ASCII");
        Text::with_baseline(text, Point::new(x as i32, 0), style, Baseline::Top)
            .draw(&mut line)
            .expect("infallible framebuffer");
        let label = Sprite {
            width: LINE_W,
            data: line.data(),
            inks: [palette::MUTED; 3],
        };
        label
            .draw(lcd, clock, strip, trace, y as i16, y as i16)
            .await?;
    }
    Ok(())
}
