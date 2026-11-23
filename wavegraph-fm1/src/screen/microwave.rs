//! Blue rack face, red position dial and a green readout around the live scope.

use super::{BAND, GRID, mixer::Band, selector::Selector};
use crate::{firmware::Labels, panel::Channel};
use embedded_graphics::{
    Drawable,
    mono_font::{MonoTextStyle, ascii::FONT_9X15_BOLD},
    prelude::{Point, Size},
    text::{Alignment, Baseline, Text},
};
use fm1_graphics::{Rgb565, WaveformStyle};
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Error},
    time::Clock,
};
use wavegraph_wavetable::{Mode, Scan};

pub(super) const FACE: Rgb565 = Rgb565::new(8, 17, 16);
pub(super) const RED: Rgb565 = Rgb565::new(29, 9, 6);
pub(super) const PRINT: Rgb565 = Rgb565::new(28, 57, 30);
const SHADOW: Rgb565 = Rgb565::new(3, 7, 7);
const LIME: Rgb565 = Rgb565::new(20, 48, 11);
const INK: Rgb565 = Rgb565::new(2, 8, 3);
pub(super) const STYLE: WaveformStyle = WaveformStyle {
    background: Rgb565::new(2, 8, 4),
    grid: Rgb565::new(4, 15, 6),
    center: Rgb565::new(6, 23, 8),
    trace: Rgb565::new(23, 58, 15),
    grid_x: GRID,
    grid_y: GRID,
};

#[derive(Default)]
pub(super) struct Panel {
    bank: Option<u8>,
    frame: Option<u8>,
    status: Option<(Channel, bool)>,
    controls: Option<Scan>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        pixels: &mut [[u8; 2]],
        selector: &Selector,
        labels: &Labels,
        frame: u8,
        entering: bool,
    ) -> Result<(), Error> {
        use Alignment::{Center, Left};
        let ink = |color| selector.shade(color);
        let bank = labels.voice.waveform;
        if entering || self.bank != Some(bank) {
            let mut top = Band::new(pixels, 4, 0, 112, 16, ink(FACE));
            Text::with_baseline(
                "waveTable",
                Point::new(8, 1),
                MonoTextStyle::new(&FONT_9X15_BOLD, ink(PRINT)),
                Baseline::Top,
            )
            .draw(&mut top)
            .expect("infallible strip");
            lcd.write_rect_buffered(4, 0, 112, 16, top.pixels.as_flattened())
                .await?;
            let mut sub = Band::new(pixels, 4, 16, 112, 16, ink(FACE));
            sub.label(
                ["RESONANT SAW", "VOCAL FORMANTS"][usize::from(bank)],
                Point::new(8, 18),
                Left,
                ink(PRINT),
            );
            lcd.write_rect_buffered(4, 16, 112, 16, sub.pixels.as_flattened())
                .await?;
            self.bank = Some(bank);
        }
        if entering || self.frame != Some(frame) {
            let mut readout = Band::new(pixels, 119, 2, 68, 27, ink(FACE));
            readout.knob(
                Point::new(132, 15),
                frame,
                63,
                [ink(RED), ink(SHADOW), ink(PRINT)],
            );
            readout.rect(Point::new(148, 3), Size::new(38, 24), ink(SHADOW));
            readout.rect(Point::new(149, 4), Size::new(36, 22), ink(LIME));
            readout.label("FRAME", Point::new(167, 4), Center, ink(INK));
            let digits = [b'0' + frame / 10, b'0' + frame % 10];
            readout.label(
                core::str::from_utf8(&digits).expect("digits"),
                Point::new(167, 15),
                Center,
                ink(INK),
            );
            lcd.write_rect_buffered(119, 2, 68, 27, readout.pixels.as_flattened())
                .await?;
            self.frame = Some(frame);
        }
        let status = (labels.channel, labels.voice_tap);
        if entering || self.status != Some(status) {
            let mut side = Band::new(pixels, 190, 2, 34, 27, ink(FACE));
            side.label(
                ["CH1", "CH2", "CH3", "CH4"][labels.channel.index()],
                Point::new(207, 4),
                Center,
                ink(PRINT),
            );
            side.label(
                if labels.voice_tap { "OSC" } else { "DAC" },
                Point::new(207, 16),
                Center,
                ink(LIME),
            );
            lcd.write_rect_buffered(190, 2, 34, 27, side.pixels.as_flattened())
                .await?;
            self.status = Some(status);
        }

        let scan = labels.wave_scan;
        if entering || self.controls != Some(scan) {
            let mut time = heapless::String::<8>::new();
            let mut start = heapless::String::<3>::new();
            let mut end = heapless::String::<3>::new();
            use core::fmt::Write;
            write!(start, "{}", scan.start).expect("three digits");
            write!(end, "{}", scan.end).expect("three digits");
            write!(time, "{}ms", scan.travel_ms).expect("five digits and units");
            let mode = match scan.mode {
                Mode::Stepped => "STEP",
                Mode::Smooth => "SMOOTH",
                Mode::OneShot => "ONCE",
                Mode::PingPong => "PINGPONG",
            };
            let moving = matches!(scan.mode, Mode::OneShot | Mode::PingPong);
            for (x, name, value, enabled) in [
                (0, "1 START", start.as_str(), true),
                (60, "2 END", end.as_str(), moving),
                (120, "3 TIME", time.as_str(), moving),
                (180, "4 MODE", mode, true),
            ] {
                let left = i32::from(x);
                for y in (200..240).step_by(BAND) {
                    let mut band = Band::new(pixels, x, y, 60, 8, ink(FACE));
                    band.rect(Point::new(left, 200), Size::new(60, 2), ink(SHADOW));
                    band.label(name, Point::new(left + 30, 204), Center, ink(PRINT));
                    band.rect(Point::new(left + 3, 217), Size::new(54, 17), ink(SHADOW));
                    band.rect(
                        Point::new(left + 4, 218),
                        Size::new(52, 15),
                        ink(if enabled { LIME } else { FACE }),
                    );
                    band.label(
                        value,
                        Point::new(left + 30, 220),
                        Center,
                        ink(if enabled { INK } else { PRINT }),
                    );
                    lcd.write_rect_buffered(x, y, 60, 8, band.pixels.as_flattened())
                        .await?;
                }
            }
            self.controls = Some(scan);
        }
        lcd.flush().await
    }
}
