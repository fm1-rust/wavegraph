//! PRESETS feedback drawn over the finished channel page.
use super::lettering::BADGE;
use crate::panel::Channel;
use fm1_graphics::Rgb565;
use fm1_graphics::embedded_graphics::pixelcolor::raw::ToBytes;
use fm1_rticx::{
    bsp::display::{Display, Error},
    time::Clock,
};

pub(super) async fn paint(
    lcd: &mut Display,
    clock: &mut Clock,
    strip: &mut [[u8; 2]],
    channel: Channel,
    [ink, background]: [Rgb565; 2],
) -> Result<(), Error> {
    let mask = BADGE.frame(channel.index());
    let pixels = &mut strip[..mask.width * mask.height];
    for (y, row) in pixels.chunks_exact_mut(mask.width).enumerate() {
        for (x, pixel) in row.iter_mut().enumerate() {
            *pixel = background.to_be_bytes();
            super::lettering::blend(pixel, ink, mask.coverage(x, y));
        }
    }
    let mut lcd = lcd.as_async(clock)?;
    lcd.write_rect_buffered(
        0,
        0,
        mask.width as u16,
        mask.height as u16,
        pixels.as_flattened(),
    )
    .await?;
    lcd.flush().await
}
