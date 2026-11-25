//! Two-operator routing, a green LCD and four flat membrane-style controls.

use super::{BAND, GRID, mixer::Band, selector::Selector};
use crate::{
    firmware::Labels,
    panel::{Channel, Polyphony},
};
use core::fmt::Write;
use fm1_graphics::{
    Rgb565, WaveformStyle,
    embedded_graphics::{
        prelude::{Point, Size},
        text::Alignment,
    },
};
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Error},
    time::Clock,
};
use heapless::String;
use wavegraph_fm::{Patch, VoiceMode};

pub(super) const FACE: Rgb565 = Rgb565::new(6, 10, 5);
pub(super) const GREEN: Rgb565 = Rgb565::new(13, 53, 20);
pub(super) const PRINT: Rgb565 = Rgb565::new(29, 58, 26);
const LCD: Rgb565 = Rgb565::new(20, 45, 13);
const INK: Rgb565 = Rgb565::new(3, 10, 4);
// Leave the overload indicator at the right edge of the header visible.
const HEADER_WIDTH: u16 = 228;
const KEY: Rgb565 = Rgb565::new(4, 22, 12);

pub(super) const STYLE: WaveformStyle = WaveformStyle {
    background: LCD,
    grid: Rgb565::new(18, 41, 12),
    center: Rgb565::new(15, 34, 10),
    trace: INK,
    grid_x: GRID,
    grid_y: GRID,
};
pub(super) const PANEL_STYLE: WaveformStyle = WaveformStyle {
    background: FACE,
    grid: KEY,
    center: GREEN,
    trace: GREEN,
    ..STYLE
};

#[derive(Default)]
pub(super) struct Panel {
    header: Option<(u8, Channel, VoiceMode, bool)>,
    patch: Option<Patch>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        pixels: &mut [[u8; 2]],
        selector: &mut Selector,
        labels: &Labels,
        entering: bool,
    ) -> Result<(), Error> {
        use Alignment::{Center, Left};
        let shade = selector.shader();
        let fm = labels.fm;
        let header = (fm.program, labels.channel, labels.mode, labels.voice_tap);
        if entering || self.header != Some(header) {
            for y in (0..40).step_by(BAND) {
                let mut band = Band::new(pixels, 0, y, HEADER_WIDTH, BAND as u16, shade(FACE));
                band.title("WAVEGRAPH FM", Point::new(6, 2), shade(PRINT));
                // Operator 2 feeds operator 1. The return above 2 is its one-sample feedback.
                band.rect(Point::new(8, 19), Size::new(16, 1), shade(GREEN));
                band.rect(Point::new(8, 19), Size::new(1, 8), shade(GREEN));
                band.rect(Point::new(23, 19), Size::new(1, 8), shade(GREEN));
                band.rect(Point::new(8, 27), Size::new(68, 1), shade(GREEN));
                for (x, number) in [(13, "2"), (46, "1")] {
                    band.rect(Point::new(x, 22), Size::new(14, 12), shade(GREEN));
                    band.label(number, Point::new(x + 7, 23), Center, shade(INK));
                }
                band.rect(Point::new(93, 4), Size::new(87, 31), shade(GREEN));
                band.rect(Point::new(94, 5), Size::new(85, 29), shade(LCD));
                band.label(
                    ["PROGRAM 01", "PROGRAM 02", "PROGRAM 03"][usize::from(fm.program)],
                    Point::new(99, 7),
                    Left,
                    shade(INK),
                );
                band.title(
                    Patch::PRESETS[usize::from(fm.program)].0,
                    Point::new(99, 18),
                    shade(INK),
                );
                band.label(
                    ["CH 1", "CH 2", "CH 3", "CH 4"][labels.channel.index()],
                    Point::new(187, 3),
                    Left,
                    shade(PRINT),
                );
                band.label(
                    Polyphony::label(labels.mode),
                    Point::new(187, 15),
                    Left,
                    shade(GREEN),
                );
                band.label(
                    if labels.voice_tap { "SRC" } else { "DAC" },
                    Point::new(187, 26),
                    Left,
                    shade(PRINT),
                );
                band.compose(selector);
                band.compose_badge(&mut selector.badge);
                lcd.write_rect_buffered(
                    0,
                    y,
                    HEADER_WIDTH,
                    BAND as u16,
                    band.pixels.as_flattened(),
                )
                .await?;
            }
            self.header = Some(header);
        }

        if entering || self.patch != Some(fm.patch) {
            let patch = fm.patch;
            for (i, name) in ["1 RATIO", "2 INDEX", "3 DECAY", "4 FDBK"]
                .into_iter()
                .enumerate()
            {
                let mut value = String::<8>::new();
                match i {
                    0 => value.push_str(patch.ratio.label()).expect("ratio label"),
                    1 => write!(value, "{}", patch.index).expect("index"),
                    2 => {
                        let us = patch.modulator.decay.micros();
                        if us.is_multiple_of(1000) {
                            write!(value, "{}ms", us / 1000)
                        } else {
                            write!(value, "{}us", us)
                        }
                        .expect("segment time")
                    }
                    _ => write!(value, "{}", patch.feedback).expect("feedback"),
                }
                let x = u16::try_from(i * 60).expect("four keys");
                let left = i32::from(x);
                let mut band = Band::new(pixels, x, 202, 60, 32, shade(FACE));
                band.rect(Point::new(left + 2, 204), Size::new(56, 29), shade(GREEN));
                band.rect(Point::new(left + 3, 205), Size::new(54, 27), shade(KEY));
                band.label(name, Point::new(left + 30, 207), Center, shade(GREEN));
                band.label(&value, Point::new(left + 30, 220), Center, shade(PRINT));
                lcd.write_rect_buffered(x, 202, 60, 32, band.pixels.as_flattened())
                    .await?;
            }
            self.patch = Some(patch);
        }
        lcd.flush().await
    }
}
