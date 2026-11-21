//! The boot illustration and live scope share one dirty-tile waveform renderer.

use crate::{
    capture::{FLAT, HEIGHT, Trace, WIDTH},
    firmware::Frame,
};
use embedded_graphics_core::pixelcolor::{IntoStorage, raw::ToBytes};
use fm1_graphics::{Rgb565, TextGrid, Waveform, WaveformStyle, palette};
use fm1_rticx::{
    bsp::{
        display::{Display, Error, SIZE},
        time::ticks,
    },
    time::{Clock, ExtU32, Timebase},
};
use micromath::F32Ext;
use static_cell::{ConstStaticCell, StaticCell};

const TOP: u16 = (SIZE - HEIGHT as u16) / 2;
const GRID: u8 = 24;
const STYLE: WaveformStyle = WaveformStyle {
    background: Rgb565::new(0, 2, 1),
    grid: Rgb565::new(2, 12, 4),
    center: Rgb565::new(4, 21, 7),
    trace: Rgb565::new(9, 51, 11),
    grid_x: GRID,
    grid_y: GRID,
};
const BOOT_STYLE: WaveformStyle = WaveformStyle {
    trace: Rgb565::new(3, 34, 8),
    ..STYLE
};
const BAND: usize = 8;
const TRAIL: usize = 32;
const LOGO_W: usize = 208;
const LOGO_H: usize = 36;
const LOGO_X: usize = (WIDTH - LOGO_W) / 2;
const WORDMARK: &[u8; LOGO_W * LOGO_H / 4] = include_bytes!("../assets/wordmark.2bpp");
const INKS: [Rgb565; 3] = [
    Rgb565::new(0, 17, 4),
    Rgb565::new(0, 35, 7),
    Rgb565::new(6, 63, 15),
];

const CENTER_Y: i16 = (SIZE as i16 - LOGO_H as i16) / 2;
const SETTLE_TICKS: u32 = 52;

// Smoothly turn at each extremum. The clock runs at 100 Hz.
fn logo_y(tick: u32) -> i16 {
    let (from, to, elapsed, duration) = match tick {
        0..26 => (-(LOGO_H as i16), CENTER_Y + 24, tick, 26),
        26..39 => (CENTER_Y + 24, CENTER_Y - 10, tick - 26, 13),
        39..47 => (CENTER_Y - 10, CENTER_Y + 3, tick - 39, 8),
        47..SETTLE_TICKS => (CENTER_Y + 3, CENTER_Y, tick - 47, SETTLE_TICKS - 47),
        _ => return CENTER_Y,
    };
    // Cubic smoothstep t²(3 − 2t) has zero velocity at both ends of each leg.
    let weight = elapsed * elapsed * (3 * duration - 2 * elapsed);
    from + (i32::from(to - from) * weight as i32 / duration.pow(3) as i32) as i16
}

fn paper(x: usize, y: usize) -> Rgb565 {
    if y == WIDTH / 2 {
        STYLE.center
    } else if x.is_multiple_of(usize::from(GRID)) || y.is_multiple_of(usize::from(GRID)) {
        STYLE.grid
    } else {
        STYLE.background
    }
}

pub(super) struct Screen {
    plot: &'static mut Waveform<WIDTH, WIDTH>,
    text: &'static mut TextGrid,
    strip: &'static mut [[u8; 2]; BAND * HEIGHT],
    beam: Beam,
}

impl Screen {
    pub(super) fn new() -> Self {
        static PLOT: StaticCell<Waveform<WIDTH, WIDTH>> = StaticCell::new();
        static TEXT: StaticCell<TextGrid> = StaticCell::new();
        static STRIP: ConstStaticCell<[[u8; 2]; BAND * HEIGHT]> =
            ConstStaticCell::new([[0; 2]; BAND * HEIGHT]);
        Self {
            plot: PLOT.init_with(|| Waveform::new(0, 0).expect("panel-sized scope")),
            text: TEXT.init_with(|| TextGrid::with_background(STYLE.background)),
            strip: STRIP.take(),
            beam: Beam {
                old_head: None,
                old_trace: FLAT,
            },
        }
    }

    pub(super) async fn splash(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
    ) -> Result<(), Error> {
        // A phase-modulated sine with a second harmonic. One sample per LCD column.
        let trace: Trace = core::array::from_fn(|x| {
            let t = x as f32 / WIDTH as f32;
            let phase = 1.8 * t + 0.14 * (3.0 * t).sin_turns();
            (120.0 - 61.0 * phase.sin_turns() - 15.0 * (2.0 * phase).sin_turns()) as u8
        });
        self.plot.set_style(BOOT_STYLE);
        self.plot.paint_async(lcd, &trace, clock).await?;

        let mut previous = logo_y(0);
        let start = ticks();
        loop {
            let elapsed = ticks().wrapping_sub(start);
            let y = logo_y(elapsed);
            if y != previous {
                move_logo(lcd, clock, self.strip, &trace, previous, y).await?;
                previous = y;
            }
            if elapsed >= SETTLE_TICKS {
                break;
            }
            Clock::delay(10.millis()).await;
        }
        Clock::delay(500.millis()).await;
        // Changing the style invalidates the entire plot, including the logo.
        self.plot.set_style(STYLE);
        Ok(())
    }

    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        frame: &Frame,
        cutoff_hz: u16,
        late: bool,
    ) -> Result<(), Error> {
        let trace = frame.trace.map(|y| y + TOP as u8);
        self.plot.paint_async(lcd, &trace, clock).await?;
        if self.beam.old_head.is_none() {
            // The live trace is confined to rows 24..216, clear the two label rows once.
            for y in [0, SIZE - 16] {
                lcd.fill_rect(0, y, SIZE, 16, STYLE.background.into_storage())?;
            }
        }
        self.beam
            .paint(lcd, clock, &frame.trace, self.strip)
            .await?;
        let labels = frame.labels;
        self.text.text(0, 0, b"CUT", palette::MUTED);
        self.text
            .number(4, 0, 5, u32::from(cutoff_hz), palette::GREEN);
        self.text.text(9, 0, b"HZ", palette::MUTED);
        self.text.text(
            12,
            0,
            if labels.half_gain { b"HALF" } else { b"DRY " },
            palette::MUTED,
        );
        self.text.text(
            17,
            0,
            if labels.voice_tap { b"VCF" } else { b"DAC" },
            palette::MUTED,
        );
        self.text.text(0, 14, labels.voice.label(), palette::GREEN);
        self.text
            .text(8, 14, labels.voice.note_name(), palette::WHITE);
        if late {
            self.text.text(11, 14, b"LATE", palette::RED);
        }
        self.text.text(16, 14, b"V", palette::MUTED);
        let velocity = labels.voice.note.map_or(0, |note| u32::from(note.velocity));
        self.text.number(17, 14, 3, velocity, palette::GREEN);
        self.text.paint_async(lcd, clock).await
    }
}

// Reconstruct the graph paper and the connected trace under both sprite positions.
// The strip is shared with the live phosphor sweep. No full-screen framebuffer.
async fn move_logo(
    lcd: &mut Display,
    clock: &mut Clock,
    strip: &mut [[u8; 2]],
    trace: &Trace,
    previous: i16,
    top: i16,
) -> Result<(), Error> {
    let first = previous.min(top).max(0) as usize;
    let end = (previous.max(top) + LOGO_H as i16).clamp(0, SIZE as i16) as usize;
    let rows = strip.len() / LOGO_W;
    let mut lcd = lcd.as_async(clock)?;
    for start in (first..end).step_by(rows) {
        let height = rows.min(end - start);
        let pixels = &mut strip[..height * LOGO_W];
        for dx in 0..LOGO_W {
            let x = LOGO_X + dx;
            let (a, b) = segment(trace, x);
            for dy in 0..height {
                let y = start + dy;
                let sy = y as i16 - top;
                let ink = if (0..LOGO_H as i16).contains(&sy) {
                    let packed = WORDMARK[sy as usize * (LOGO_W / 4) + dx / 4];
                    (packed >> (6 - 2 * (dx % 4))) & 3
                } else {
                    0
                };
                let color = if ink != 0 {
                    INKS[usize::from(ink - 1)]
                } else if (a..=b).contains(&y) {
                    BOOT_STYLE.trace
                } else {
                    paper(x, y)
                };
                pixels[dy * LOGO_W + dx] = color.to_be_bytes();
            }
        }
        lcd.write_rect_buffered(
            LOGO_X as u16,
            start as u16,
            LOGO_W as u16,
            height as u16,
            pixels.as_flattened(),
        )
        .await?;
    }
    lcd.flush().await
}

struct Beam {
    old_head: Option<usize>,
    old_trace: Trace,
}

fn segment(trace: &Trace, x: usize) -> (usize, usize) {
    let a = usize::from(trace[x.saturating_sub(1)]);
    let b = usize::from(trace[x]);
    (a.min(b), a.max(b))
}

fn age(head: usize, x: usize) -> usize {
    (head + WIDTH - x) % WIDTH
}

// The brightest point leads. Its one-pixel halo disappears before the tail.
fn phosphor(age: usize) -> (Rgb565, Option<Rgb565>) {
    match age {
        0..=3 => (Rgb565::new(26, 63, 23), Some(Rgb565::new(9, 40, 10))),
        4..=9 => (Rgb565::new(18, 61, 17), Some(Rgb565::new(5, 28, 7))),
        10..=19 => (Rgb565::new(14, 58, 14), Some(Rgb565::new(3, 20, 5))),
        20..TRAIL => (Rgb565::new(11, 54, 12), None),
        _ => (STYLE.trace, None),
    }
}

impl Beam {
    async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        trace: &Trace,
        scratch: &mut [[u8; 2]],
    ) -> Result<(), Error> {
        let head = self.old_head.map_or(0, |x| (x + BAND) % WIDTH);
        let mut lcd = lcd.as_async(clock)?;
        for x0 in (0..WIDTH).step_by(BAND) {
            // Include the old halo when a note or its gain changes under the beam.
            let mut top = HEIGHT;
            let mut bottom = 0;
            for x in x0..x0 + BAND {
                for (which_head, which_trace) in
                    [(Some(head), trace), (self.old_head, &self.old_trace)]
                {
                    if which_head.is_some_and(|h| age(h, x) < TRAIL) {
                        let (a, b) = segment(which_trace, x);
                        top = top.min(a.saturating_sub(1));
                        bottom = bottom.max((b + 1).min(HEIGHT - 1));
                    }
                }
            }
            if top == HEIGHT {
                continue;
            }
            let height = bottom - top + 1;
            let pixels = &mut scratch[..BAND * height];
            for dx in 0..BAND {
                let x = x0 + dx;
                let (a, b) = segment(trace, x);
                let (ink, halo) = phosphor(age(head, x));
                for y in top..=bottom {
                    let paper = paper(x, usize::from(TOP) + y);
                    let color = if (a..=b).contains(&y) {
                        ink
                    } else if y + 1 == a || y == b + 1 {
                        halo.unwrap_or(paper)
                    } else {
                        paper
                    };
                    pixels[(y - top) * BAND + dx] = color.to_be_bytes();
                }
            }
            lcd.write_rect_buffered(
                x0 as u16,
                TOP + top as u16,
                BAND as u16,
                height as u16,
                pixels.as_flattened(),
            )
            .await?;
        }
        lcd.flush().await?;
        self.old_head = Some(head);
        self.old_trace = *trace;
        Ok(())
    }
}
