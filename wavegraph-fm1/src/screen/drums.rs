//! Drum faceplates and per-voice activity beneath the live kit scope.

use super::{mixer::Band, selector::Selector, tr808};
use crate::assets::fonts::{FONT_5X8, FONT_6X10, FONT_9X15_BOLD};
use crate::{instrument::Part, panel::Channel, sequencer::DrumLamp};
use embedded_graphics::{
    Drawable,
    framebuffer::Framebuffer,
    mono_font::MonoTextStyle,
    pixelcolor::{
        BinaryColor,
        raw::{BigEndian, RawU1, ToBytes},
    },
    prelude::{Point, Primitive, Size},
    primitives::{PrimitiveStyle, Rectangle, RoundedRectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use fm1_graphics::{Rgb565, WaveformStyle};
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Error},
    time::Clock,
};
use wavegraph_909::Voice;

pub(super) const GRAPHITE: Rgb565 = Rgb565::new(7, 15, 7);
pub(super) const ORANGE: Rgb565 = Rgb565::new(22, 17, 2);
pub(super) const HOT: Rgb565 = Rgb565::new(31, 26, 1);
pub(super) const GLOW: Rgb565 = Rgb565::new(31, 48, 17);
pub(super) const EMBER: Rgb565 = Rgb565::new(17, 13, 4);
pub(super) const STYLE: WaveformStyle = WaveformStyle {
    background: Rgb565::new(29, 57, 26),
    grid: Rgb565::new(25, 50, 23),
    center: Rgb565::new(20, 40, 18),
    trace: GRAPHITE,
    grid_x: super::GRID,
    grid_y: super::GRID,
};

pub(super) const CREAM: Rgb565 = Rgb565::new(30, 53, 20);
pub(super) const STYLE_808: WaveformStyle = WaveformStyle {
    background: Rgb565::new(3, 6, 3),
    grid: Rgb565::new(7, 12, 4),
    center: Rgb565::new(12, 20, 6),
    trace: Rgb565::new(29, 32, 5),
    ..STYLE
};

pub(super) fn style(part: Part) -> WaveformStyle {
    if part == Part::Tr808 {
        STYLE_808
    } else {
        STYLE
    }
}

const KEY_W: usize = 39;
const KEY_H: usize = 47;
const KEY_Y: u16 = 189;
const LENS_W: usize = 20;
const LENS_H: usize = 7;
const LENS_X: u16 = 10;
const LENS_Y: u16 = 6;
const _: () = assert!(KEY_W * KEY_H <= super::BAND * crate::capture::WIDTH);

#[derive(Default)]
pub(super) struct Panel {
    previous: Option<[DrumLamp; 6]>,
    controls: Option<wavegraph_808::Controls>,
    status808: Option<(Channel, bool, &'static str)>,
}

impl Panel {
    async fn title(
        lcd: &mut AsyncDisplay<'_, Clock>,
        strip: &mut [[u8; 2]],
        selector: &mut Selector,
        part: Part,
    ) -> Result<(), Error> {
        // Expand the one-bit wordmark in eight-row strips.
        type Title = Framebuffer<BinaryColor, RawU1, BigEndian, 56, 16, 112>;
        let mut mask = Title::new();
        Text::with_baseline(
            part.name(),
            Point::zero(),
            MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On),
            Baseline::Top,
        )
        .draw(&mut mask)
        .expect("infallible framebuffer");
        let [paper, gray, orange] = [
            style(part).background,
            if part == Part::Tr808 { CREAM } else { GRAPHITE },
            ORANGE,
        ]
        .map(|ink| selector.shade(ink).to_be_bytes());
        let rows = mask.data().as_chunks::<7>().0;
        for (band, source) in rows.chunks_exact(4).enumerate() {
            let pixels = &mut strip[..112 * 8];
            for (y, row) in pixels.chunks_exact_mut(112).enumerate() {
                for (x, pixel) in row.iter_mut().enumerate() {
                    let bit = x / 2;
                    *pixel = if source[y / 2][bit / 8] & (0x80 >> (bit % 8)) == 0 {
                        paper
                    } else if bit < 3 * 9 {
                        gray
                    } else {
                        orange
                    };
                }
            }
            let y = 1 + 8 * band;
            selector.badge.compose(10, y, 112, pixels);
            lcd.write_rect_buffered(10, y as u16, 112, 8, pixels.as_flattened())
                .await?;
        }
        Ok(())
    }

    fn key_face(key: &mut Band<'_>, voice: Voice, slot: usize, selector: &Selector) {
        // Recess, lower bevel and face of the keycap.
        for (x, y, w, h, ink) in [
            (0, 0, 39, 47, GRAPHITE),
            (1, 1, 37, 44, Rgb565::new(23, 46, 21)),
            (2, 1, 35, 35, Rgb565::new(30, 60, 28)),
        ] {
            RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(x, y), Size::new(w, h)),
                Size::new(3, 3),
            )
            .into_styled(PrimitiveStyle::with_fill(selector.shade(ink)))
            .draw(key)
            .expect("infallible framebuffer");
        }
        let (first, second) = match voice {
            Voice::Kick => ("KICK", ""),
            Voice::Snare => ("SNARE", ""),
            Voice::Clap => ("CLAP", ""),
            Voice::ClosedHat => ("CLOSED", "HAT"),
            Voice::OpenHat => ("OPEN", "HAT"),
            Voice::Ride => ("RIDE", ""),
        };
        let text = MonoTextStyle::new(&FONT_6X10, selector.shade(GRAPHITE));
        let align = TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Top)
            .build();
        for (label, y) in [
            (first, if second.is_empty() { 20 } else { 16 }),
            (second, 26),
        ] {
            Text::with_text_style(label, Point::new(19, y), text, align)
                .draw(key)
                .expect("infallible framebuffer");
        }
        Text::with_text_style(
            ["F3", "G3", "A3", "B3", "C4", "D4"][slot],
            Point::new(19, 36),
            MonoTextStyle::new(&FONT_5X8, selector.shade(GRAPHITE)),
            align,
        )
        .draw(key)
        .expect("infallible framebuffer");
    }

    pub(super) async fn status808(
        &mut self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        strip: &mut [[u8; 2]],
        selector: &Selector,
        status: (Channel, bool, &'static str),
        entering: bool,
    ) -> Result<(), Error> {
        if entering || self.status808 != Some(status) {
            tr808::status(lcd, strip, selector, status).await?;
            self.status808 = Some(status);
        }
        Ok(())
    }

    pub(super) async fn controls(
        &mut self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        strip: &mut [[u8; 2]],
        selector: &Selector,
        c: wavegraph_808::Controls,
        entering: bool,
    ) -> Result<(), Error> {
        let previous = if entering { None } else { self.controls };
        if previous == Some(c) {
            return Ok(());
        }
        tr808::controls(lcd, strip, selector, c, previous).await?;
        self.controls = Some(c);
        Ok(())
    }

    pub(super) async fn paint(
        &mut self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        strip: &mut [[u8; 2]],
        selector: &mut Selector,
        entering: bool,
        part: Part,
        lamps: [DrumLamp; 6],
    ) -> Result<(), Error> {
        if !entering && self.previous == Some(lamps) {
            return Ok(());
        }
        if entering {
            Self::title(lcd, strip, selector, part).await?;
            let rule = selector.shade(ORANGE);
            for y in [52, 185] {
                super::fill_rows(lcd, strip, 0, y..y + 2, rule).await?;
            }
        }
        if part == Part::Tr808 {
            tr808::switches(
                lcd,
                strip,
                selector,
                lamps,
                if entering { None } else { self.previous },
            )
            .await?;
            self.previous = Some(lamps);
            return Ok(());
        }
        for (i, voice) in Voice::ALL.into_iter().enumerate() {
            let lamp = lamps[voice as usize];
            let x = 1 + 40 * u16::try_from(i).expect("six keys");
            if entering {
                let mut key = Band::new(
                    strip,
                    0,
                    0,
                    KEY_W as u16,
                    KEY_H as u16,
                    selector.shade(STYLE.background),
                );
                Self::key_face(&mut key, voice, i, selector);
                lcd.write_rect_buffered(
                    x,
                    KEY_Y,
                    KEY_W as u16,
                    KEY_H as u16,
                    key.pixels.as_flattened(),
                )
                .await?;
            } else if self.previous.is_some_and(|old| old[voice as usize] == lamp) {
                continue;
            }
            let (lens, core) = match lamp {
                DrumLamp::Off => (Rgb565::new(7, 14, 6), Rgb565::new(10, 19, 8)),
                DrumLamp::Tail => (EMBER, Rgb565::new(25, 21, 4)),
                DrumLamp::Strike => (HOT, Rgb565::new(31, 53, 13)),
            };
            let [rim, lens, core] =
                [GRAPHITE, lens, core].map(|color| selector.shade(color).to_be_bytes());
            let pixels: [[_; LENS_W]; LENS_H] = core::array::from_fn(|y| {
                core::array::from_fn(|x| {
                    if y == 0 || y + 1 == LENS_H || x == 0 || x + 1 == LENS_W {
                        rim
                    } else if (2..=3).contains(&y) && (4..LENS_W - 4).contains(&x) {
                        core
                    } else {
                        lens
                    }
                })
            });
            lcd.write_rect_buffered(
                x + LENS_X,
                KEY_Y + LENS_Y,
                LENS_W as u16,
                LENS_H as u16,
                pixels.as_flattened().as_flattened(),
            )
            .await?;
        }
        self.previous = Some(lamps);
        Ok(())
    }
}
