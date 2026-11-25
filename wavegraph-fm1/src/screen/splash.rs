//! Boot artwork composited over the scope in buffered LCD strips.

#[path = "../../assets/boot_timing.rs"]
mod boot_timing;

use super::{STYLE, Screen, paper};
use crate::assets::fonts::FONT_9X15_BOLD;
use crate::{
    capture::{Trace, WIDTH},
    scope::segment,
};
use fm1_graphics::embedded_graphics::{
    Drawable,
    draw_target::DrawTarget,
    framebuffer::{Framebuffer, buffer_size},
    geometry::Point,
    mono_font::MonoTextStyle,
    pixelcolor::{
        Gray2, IntoStorage,
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
// Round score times to the nearest 100 Hz display tick.
const fn cue_tick(ms: u16) -> u32 {
    (ms as u32 + 5) / 10
}

const IMPACT: u32 = cue_tick(boot_timing::ACCENT_A);
const REBOUND: u32 = cue_tick(boot_timing::SLIDE_C);
const TITLE_SETTLED: u32 = cue_tick(boot_timing::FIRST_KNOCK);
const MODEL_LANDED: u32 = cue_tick(boot_timing::HIGH_D);

struct Sprite<'a> {
    width: usize,
    data: &'a [u8], // Four 2-bit pixels per byte, high bits first. Index zero is transparent.
    inks: [Rgb565; 3],
}

const TITLE: Sprite<'_> = Sprite {
    width: 208,
    data: crate::assets::flash_asset!(u8, include_bytes!("../../assets/wordmark.2bpp")),
    inks: [
        Rgb565::new(0, 17, 4),
        Rgb565::new(0, 35, 7),
        Rgb565::new(6, 63, 15),
    ],
};
const MODEL: Sprite<'_> = Sprite {
    width: TITLE.width / 2,
    data: crate::assets::flash_asset!(u8, include_bytes!("../../assets/fm1.2bpp")),
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
        0..IMPACT => ease(-TITLE.height(), TITLE_Y + 18, tick, IMPACT),
        IMPACT..REBOUND => ease(TITLE_Y + 18, TITLE_Y - 9, tick - IMPACT, REBOUND - IMPACT),
        REBOUND..TITLE_SETTLED => ease(
            TITLE_Y - 9,
            TITLE_Y,
            tick - REBOUND,
            TITLE_SETTLED - REBOUND,
        ),
        _ => TITLE_Y,
    }
}

impl Screen {
    pub(crate) async fn splash(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        start_cue: impl FnOnce(),
    ) -> Result<(), Error> {
        // The first completed LCD window turns the panel on. Initialize every pixel
        // before painting the boot waveform in eight-row strips.
        lcd.as_async(clock)?
            .fill_rect(0, 0, SIZE, SIZE, BOOT_STYLE.background.into_storage())
            .await?;
        // A phase-modulated sine with a second harmonic. One sample per LCD column.
        let trace: Trace = core::array::from_fn(|x| {
            let t = x as f32 / WIDTH as f32;
            let phase = 1.8 * t + 0.14 * (3.0 * t).sin_turns();
            (120.0 - 61.0 * phase.sin_turns() - 15.0 * (2.0 * phase).sin_turns()) as u8
        });
        self.plot
            .paint(lcd, clock, self.strip, (0, 0), BOOT_STYLE, &trace)
            .await?;
        let start = ticks();
        start_cue();
        let mut title = -TITLE.height();
        let mut model = SIZE as i16;
        loop {
            let tick = ticks().wrapping_sub(start).min(MODEL_LANDED);
            let y = title_y(tick);
            if title != y {
                TITLE.draw(lcd, clock, self.strip, &trace, title, y).await?;
                title = y;
            }
            if tick >= TITLE_SETTLED {
                let y = ease(
                    SIZE as i16,
                    MODEL_Y,
                    tick - TITLE_SETTLED,
                    MODEL_LANDED - TITLE_SETTLED,
                );
                if model != y {
                    MODEL.draw(lcd, clock, self.strip, &trace, model, y).await?;
                    model = y;
                }
            }
            if tick == MODEL_LANDED {
                break;
            }
            Clock::delay(10.millis()).await;
        }
        draw_version(lcd, clock, self.strip, &trace).await?;
        Clock::delay(500.millis()).await;
        Ok(())
    }
}

impl Sprite<'_> {
    const fn height(&self) -> i16 {
        (self.data.len() / (self.width / 4)) as i16
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
    const GLYPH_W: usize = FONT_9X15_BOLD.character_size.width as usize;
    const GLYPH_H: usize = FONT_9X15_BOLD.character_size.height as usize;
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
    let style = MonoTextStyle::new(&FONT_9X15_BOLD, Gray2::new(3));
    for (i, chunk) in VERSION.as_bytes().chunks(LINE_W / GLYPH_W).enumerate() {
        let y = VERSION_Y as usize + i * (GLYPH_H + 1);
        if y + GLYPH_H > SIZE as usize {
            break;
        }
        let mut line = Line::new();
        line.clear(Gray2::new(1)).expect("infallible framebuffer");
        let x = (LINE_W - chunk.len() * GLYPH_W) / 2;
        let text = core::str::from_utf8(chunk).expect("Cargo versions are ASCII");
        Text::with_baseline(text, Point::new(x as i32, 0), style, Baseline::Top)
            .draw(&mut line)
            .expect("infallible framebuffer");
        let label = Sprite {
            width: LINE_W,
            data: line.data(),
            inks: [BOOT_STYLE.background, BOOT_STYLE.background, palette::WHITE],
        };
        label
            .draw(lcd, clock, strip, trace, y as i16, y as i16)
            .await?;
    }
    Ok(())
}
