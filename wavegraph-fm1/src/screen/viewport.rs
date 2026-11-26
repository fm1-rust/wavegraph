//! Keep instrument DMA writes inside the visible parts of the faceplate.
use super::{WIDTH, lettering::BADGE, selector};
use crate::scope::Window;
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Display, Error},
    time::Clock,
};

pub(super) const EDGE: usize = 2;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Viewport {
    Full,
    Swap,
    SwapWithPicker,
}

impl Viewport {
    pub(super) fn windows(self, source: Window) -> impl Iterator<Item = Window> {
        // The badge and swap header occupy the first 32 rows. The picker begins
        // two rows above the header's bottom and leaves three exposed rectangles.
        const FULL: Window = Window::new(0, 0, WIDTH, WIDTH);
        const BODY: Window = Window::new(
            EDGE,
            BADGE.height,
            WIDTH - 2 * EDGE,
            WIDTH - EDGE - BADGE.height,
        );
        const SIDES_HEIGHT: usize = selector::BOTTOM - BADGE.height;
        const EXPOSED: [Window; 3] = [
            Window::new(EDGE, BADGE.height, selector::LEFT - EDGE, SIDES_HEIGHT),
            Window::new(
                selector::RIGHT,
                BADGE.height,
                WIDTH - EDGE - selector::RIGHT,
                SIDES_HEIGHT,
            ),
            Window::new(
                EDGE,
                selector::BOTTOM,
                BODY.width,
                WIDTH - EDGE - selector::BOTTOM,
            ),
        ];
        let windows: &[_] = match self {
            Self::Full => &[FULL],
            Self::Swap => &[BODY],
            Self::SwapWithPicker => &EXPOSED,
        };
        windows
            .iter()
            .filter_map(move |area| source.intersection(*area))
    }
}

pub(super) struct PanelDisplay<'a> {
    lcd: AsyncDisplay<'a, Clock>,
    viewport: Viewport,
}

impl<'a> PanelDisplay<'a> {
    pub(super) fn new(
        lcd: &'a mut Display,
        clock: &'a mut Clock,
        viewport: Viewport,
    ) -> Result<Self, Error> {
        Ok(Self {
            lcd: lcd.as_async(clock)?,
            viewport,
        })
    }

    pub(super) async fn fill_rect(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        color: u16,
    ) -> Result<(), Error> {
        for r in self
            .viewport
            .windows(Window::new(x.into(), y.into(), w.into(), h.into()))
        {
            self.lcd
                .fill_rect(
                    r.x as u16,
                    r.y as u16,
                    r.width as u16,
                    r.height as u16,
                    color,
                )
                .await?;
        }
        Ok(())
    }

    pub(super) async fn write_rect(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        pixels: &mut [[u8; 2]],
    ) -> Result<(), Error> {
        if self.viewport == Viewport::Full {
            return self
                .lcd
                .write_rect_buffered(x, y, w, h, pixels.as_flattened())
                .await;
        }
        let source = Window::new(x.into(), y.into(), w.into(), h.into());
        let mut pieces = self.viewport.windows(source).peekable();
        while let Some(r) = pieces.next() {
            let stride = source.width;
            let offset = (r.y - source.y) * stride + (r.x - source.x);
            let length = r.width;
            let (x, y, w, h) = (r.x as u16, r.y as u16, r.width as u16, r.height as u16);
            if r.width == source.width {
                self.lcd
                    .write_rect_buffered(
                        x,
                        y,
                        w,
                        h,
                        pixels[offset..offset + length * r.height].as_flattened(),
                    )
                    .await?;
            } else if pieces.peek().is_none() {
                // Pack the final cropped rectangle in the existing strip. Only
                // this last piece can reuse storage belonging to another one.
                for row in 0..r.height {
                    let start = offset + row * stride;
                    pixels.copy_within(start..start + length, row * length);
                }
                self.lcd
                    .write_rect_buffered(x, y, w, h, pixels[..length * r.height].as_flattened())
                    .await?;
            } else {
                for row in 0..r.height {
                    let start = offset + row * stride;
                    self.lcd
                        .write_rect_buffered(
                            x,
                            y + row as u16,
                            w,
                            1,
                            pixels[start..start + length].as_flattened(),
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }

    pub(super) async fn flush(&mut self) -> Result<(), Error> {
        self.lcd.flush().await
    }
}
