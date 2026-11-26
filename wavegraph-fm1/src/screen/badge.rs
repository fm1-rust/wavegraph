//! PRESETS feedback drawn over the finished channel page.
use super::{
    BAND, InstrumentStyle, Selector,
    lettering::{BADGE, GRID},
    mixer::Band,
    viewport::EDGE,
};
use crate::{instrument::Part, panel::Channel, scope::Window};
use fm1_graphics::Rgb565;
use fm1_graphics::embedded_graphics::{
    pixelcolor::{IntoStorage, RgbColor},
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Mode {
    Channel,
    InstrumentSwap(Swap),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Swap {
    pub(super) channel: Channel,
    pub(super) live: Part,
    pub(super) selected: Part,
}

pub(super) fn draw(band: &mut Band<'_>, channel: Channel, at: Point) {
    let mask = BADGE.frame(channel.index());
    band.rect(
        at,
        Size::new(mask.width as u32, mask.height as u32),
        Rgb565::BLACK,
    );
    band.mask(at, mask, 0, [Rgb565::WHITE; 2]);
}

pub(super) async fn paint(
    lcd: &mut Display,
    clock: &mut Clock,
    strip: &mut [[u8; 2]],
    channel: Channel,
) -> Result<(), Error> {
    let (w, h) = (BADGE.width as u16, BADGE.height as u16);
    let mut tile = Band::new(strip, 0, 0, w, h, Rgb565::BLACK);
    draw(&mut tile, channel, Point::zero());
    let mut lcd = lcd.as_async(clock)?;
    lcd.write_rect_buffered(0, 0, w, h, tile.pixels.as_flattened())
        .await?;
    lcd.flush().await
}

/// Keep the instrument name clear of the channel number even on narrow faceplates.
pub(super) async fn swap(
    lcd: &mut Display,
    clock: &mut Clock,
    strip: &mut [[u8; 2]],
    state: Swap,
    selector: &Selector,
    entering: bool,
) -> Result<(), Error> {
    let theme = InstrumentStyle::for_part(state.live);
    let mut lcd = lcd.as_async(clock)?;
    let edge = EDGE as u16;
    let header = BADGE.height as u16;
    if entering {
        for (x, y, w, h) in [
            (0, header, edge, SIZE - header),
            (SIZE - edge, header, edge, SIZE - header),
            (edge, SIZE - edge, SIZE - 2 * edge, edge),
        ] {
            lcd.fill_rect(x, y, w, h, Rgb565::GREEN.into_storage())
                .await?;
        }
    }
    for y in (0..BADGE.height as u16).step_by(BAND) {
        let mut band = Band::new(strip, 0, y, SIZE, BAND as u16, theme.face);
        band.label(
            "PRESETS / INSTRUMENT",
            Point::new(34, 3),
            Alignment::Left,
            theme.ink,
        );
        for (i, c) in state.selected.name().bytes().enumerate() {
            band.mask(
                Point::new(34 + (i * GRID.width) as i32, 14),
                GRID.glyph(c),
                0,
                [theme.ink; 2],
            );
        }
        band.label(
            "EDIT EXIT",
            Point::new(227, 17),
            Alignment::Right,
            theme.ink,
        );
        // Send finished pixels, including the two picker scanlines overlapping the header.
        selector.overlay(
            Window::new(0, usize::from(y), usize::from(SIZE), BAND),
            band.pixels,
        );
        band.rect(
            Point::zero(),
            Size::new(u32::from(SIZE), EDGE as u32),
            Rgb565::GREEN,
        );
        band.rect(
            Point::new(i32::from(SIZE - edge), 0),
            Size::new(EDGE as u32, BADGE.height as u32),
            Rgb565::GREEN,
        );
        draw(&mut band, state.channel, Point::zero());
        lcd.write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
            .await?;
    }
    lcd.flush().await
}
