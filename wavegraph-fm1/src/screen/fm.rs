//! Two-operator routing, a green LCD and four flat membrane-style controls.

use super::{BAND, mixer::Band, selector::Selector};
use crate::{
    firmware::Labels,
    panel::{Channel, Polyphony},
};
use core::fmt::Write;
use fm1_graphics::{
    Rgb565,
    embedded_graphics::{
        prelude::{Point, Size},
        text::Alignment,
    },
};
use fm1_rticx::bsp::display::Error;
use heapless::String;
use wavegraph_fm::{Patch, VoiceMode};

pub(super) const FACE: Rgb565 = super::style::FM.face;
pub(super) const GREEN: Rgb565 = super::style::FM.accent;
pub(super) const PRINT: Rgb565 = super::style::FM.ink;
const LCD: Rgb565 = super::style::FM.scope.background;
const INK: Rgb565 = super::style::FM.scope.trace;
// Leave the overload indicator at the right edge of the header visible.
const HEADER_WIDTH: u16 = 228;
const KEY: Rgb565 = super::style::FM.surface;

#[derive(Default)]
pub(super) struct Panel {
    header: Option<(u8, Channel, VoiceMode, bool)>,
    patch: Option<Patch>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut super::PanelDisplay<'_>,
        pixels: &mut [[u8; 2]],
        selector: &mut Selector,
        labels: &Labels,
        entering: bool,
    ) -> Result<(), Error> {
        use Alignment::{Center, Left};
        let fm = labels.fm;
        let header = (fm.program, labels.channel, labels.mode, labels.voice_tap);
        if entering || self.header != Some(header) {
            for y in (0..40).step_by(BAND) {
                let mut band = Band::new(pixels, 0, y, HEADER_WIDTH, BAND as u16, FACE);
                band.title("WAVEGRAPH FM", Point::new(6, 2), PRINT);
                // Operator 2 feeds operator 1. The return above 2 is its one-sample feedback.
                band.rect(Point::new(8, 19), Size::new(16, 1), GREEN);
                band.rect(Point::new(8, 19), Size::new(1, 8), GREEN);
                band.rect(Point::new(23, 19), Size::new(1, 8), GREEN);
                band.rect(Point::new(8, 27), Size::new(68, 1), GREEN);
                for (x, number) in [(13, "2"), (46, "1")] {
                    band.rect(Point::new(x, 22), Size::new(14, 12), GREEN);
                    band.label(number, Point::new(x + 7, 23), Center, INK);
                }
                band.rect(Point::new(93, 4), Size::new(87, 31), GREEN);
                band.rect(Point::new(94, 5), Size::new(85, 29), LCD);
                band.label(
                    ["PROGRAM 01", "PROGRAM 02", "PROGRAM 03"][usize::from(fm.program)],
                    Point::new(99, 7),
                    Left,
                    INK,
                );
                band.title(
                    Patch::PRESETS[usize::from(fm.program)].0,
                    Point::new(99, 18),
                    INK,
                );
                band.label(labels.channel.label(), Point::new(187, 3), Left, PRINT);
                band.label(
                    Polyphony::label(labels.mode),
                    Point::new(187, 15),
                    Left,
                    GREEN,
                );
                band.label(
                    if labels.voice_tap { "SRC" } else { "DAC" },
                    Point::new(187, 26),
                    Left,
                    PRINT,
                );
                band.compose(selector);
                lcd.write_rect(0, y, HEADER_WIDTH, BAND as u16, band.pixels)
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
                let mut band = Band::new(pixels, x, 202, 60, 32, FACE);
                band.rect(Point::new(left + 2, 204), Size::new(56, 29), GREEN);
                band.rect(Point::new(left + 3, 205), Size::new(54, 27), KEY);
                band.label(name, Point::new(left + 30, 207), Center, GREEN);
                band.label(&value, Point::new(left + 30, 220), Center, PRINT);
                band.compose(selector);
                lcd.write_rect(x, 202, 60, 32, band.pixels).await?;
            }
            self.patch = Some(patch);
        }
        lcd.flush().await
    }
}
