//! JP-8080 rack panel above and below the channel's full-width oscilloscope.

use super::{BAND, GRID, mixer::Band, selector::Selector};
use crate::{firmware::Labels, instrument::SawControls, panel::Channel};
use embedded_graphics::{
    Drawable,
    framebuffer::Framebuffer,
    mono_font::{MonoTextStyle, ascii::FONT_9X15_BOLD},
    pixelcolor::{
        BinaryColor,
        raw::{BigEndian, RawU1, ToBytes},
    },
    prelude::{Point, Size},
    text::{Alignment, Baseline, Text},
};
use fm1_graphics::{Rgb565, WaveformStyle};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};

pub(super) const FACE: Rgb565 = Rgb565::new(3, 8, 13);
pub(super) const EDGE: Rgb565 = Rgb565::new(8, 37, 25);
pub(super) const AMBER: Rgb565 = Rgb565::new(31, 41, 6);
const PRINT: Rgb565 = Rgb565::new(22, 50, 28);
const BLACK: Rgb565 = Rgb565::new(1, 3, 4);
const RIM: Rgb565 = Rgb565::new(10, 23, 19);
const LCD: Rgb565 = Rgb565::new(12, 43, 24);
pub(super) const STYLE: WaveformStyle = WaveformStyle {
    background: Rgb565::new(1, 6, 7),
    grid: Rgb565::new(2, 14, 12),
    center: Rgb565::new(4, 23, 18),
    trace: Rgb565::new(14, 57, 25),
    grid_x: GRID,
    grid_y: GRID,
};

#[derive(Default)]
pub(super) struct Panel {
    status: Option<(Channel, bool, [u8; 4])>,
    controls: Option<SawControls>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]],
        selector: &Selector,
        labels: Labels,
        entering: bool,
    ) -> Result<(), Error> {
        let ink = |color| selector.shade(color);
        let mut lcd = lcd.as_async(clock)?;
        if entering {
            for y in (0..32).step_by(BAND) {
                let mut band = Band::new(pixels, 0, y, SIZE, 8, ink(FACE));
                band.label("SUPER SAW", Point::new(9, 18), Alignment::Left, ink(PRINT));
                band.rect(Point::new(66, 23), Size::new(49, 1), ink(EDGE));
                band.rect(Point::new(0, 30), Size::new(240, 1), ink(EDGE));
                band.rect(Point::new(0, 31), Size::new(240, 1), ink(BLACK));
                lcd.write_rect_buffered(0, y, SIZE, 8, band.pixels.as_flattened())
                    .await?;
            }
            wordmark(pixels, ink(FACE), ink(AMBER));
            lcd.write_rect_buffered(7, 2, 100, 16, pixels[..1600].as_flattened())
                .await?;
        }

        let mut key = labels.voice.note_name();
        if labels.voice.gate.note.is_none() && labels.seq.transport.playing() {
            key = if labels.seq.enabled[labels.channel.index()] {
                *b"SEQ "
            } else {
                *b"MUTE"
            };
        }
        let status = (labels.channel, labels.voice_tap, key);
        if entering || self.status != Some(status) {
            let mut display = Band::new(pixels, 0, 0, 102, 18, ink(BLACK));
            display.rect(Point::new(1, 1), Size::new(100, 16), ink(RIM));
            display.rect(Point::new(3, 3), Size::new(96, 12), ink(LCD));
            let left = Alignment::Left;
            display.label(
                ["CH1", "CH2", "CH3", "CH4"][labels.channel.index()],
                Point::new(7, 4),
                left,
                ink(BLACK),
            );
            display.label(
                if labels.voice_tap { "OSC" } else { "DAC" },
                Point::new(40, 4),
                left,
                ink(BLACK),
            );
            display.label(
                core::str::from_utf8(&key).expect("ASCII note name"),
                Point::new(70, 4),
                left,
                ink(BLACK),
            );
            lcd.write_rect_buffered(122, 7, 102, 18, display.pixels.as_flattened())
                .await?;
            self.status = Some(status);
        }

        for (x, name, value, old) in [
            (
                0,
                "1 DETUNE",
                labels.saw.detune,
                self.controls.map(|c| c.detune),
            ),
            (120, "2 MIX", labels.saw.mix, self.controls.map(|c| c.mix)),
        ] {
            if !entering && old == Some(value) {
                continue;
            }
            let left = i32::from(x);
            let number = [
                b'0' + value / 100,
                b'0' + value / 10 % 10,
                b'0' + value % 10,
            ];
            for y in (200..240).step_by(BAND) {
                let mut band = Band::new(pixels, x, y, 120, 8, ink(FACE));
                band.rect(Point::new(left, 200), Size::new(120, 1), ink(BLACK));
                band.rect(Point::new(left, 201), Size::new(120, 1), ink(EDGE));
                band.rect(Point::new(119, 207), Size::new(1, 26), ink(RIM));
                for (dx, dy) in [
                    (-11, 8),
                    (-13, 0),
                    (-9, -9),
                    (0, -13),
                    (9, -9),
                    (13, 0),
                    (11, 8),
                ] {
                    band.rect(
                        Point::new(left + 32 + dx, 221 + dy),
                        Size::new(2, 2),
                        ink(EDGE),
                    );
                }
                band.knob(
                    Point::new(left + 32, 221),
                    value,
                    127,
                    [ink(BLACK), ink(RIM), ink(AMBER)],
                );
                band.label(
                    name,
                    Point::new(left + 54, 207),
                    Alignment::Left,
                    ink(PRINT),
                );
                Text::with_baseline(
                    core::str::from_utf8(&number).expect("ASCII digits"),
                    Point::new(left + 54, 219),
                    MonoTextStyle::new(&FONT_9X15_BOLD, ink(AMBER)),
                    Baseline::Top,
                )
                .draw(&mut band)
                .expect("infallible strip");
                lcd.write_rect_buffered(x, y, 120, 8, band.pixels.as_flattened())
                    .await?;
            }
        }
        self.controls = Some(labels.saw);
        lcd.flush().await
    }
}

// Stretch and shear the glyphs on the rack's printed model badge.
fn wordmark(pixels: &mut [[u8; 2]], face: Rgb565, amber: Rgb565) {
    let mut mask = Framebuffer::<BinaryColor, RawU1, BigEndian, 64, 16, 128>::new();
    Text::with_baseline(
        "JP-8080",
        Point::new(0, 0),
        MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On),
        Baseline::Top,
    )
    .draw(&mut mask)
    .expect("infallible wordmark");
    let paper = face.to_be_bytes();
    let print = amber.to_be_bytes();
    for (y, row) in pixels[..1600].chunks_exact_mut(100).enumerate() {
        let shear = (15 - y) / 3;
        row.fill(paper);
        for (x, pixel) in row.iter_mut().skip(shear).take(96).enumerate() {
            let bit = x * 2 / 3;
            if mask.data()[y * 8 + bit / 8] & (0x80 >> (bit % 8)) != 0 {
                *pixel = print;
            }
        }
    }
}
