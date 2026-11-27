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
    ChannelHud,
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
        const HUD: Window = selector::ChannelHud::AREA;
        const HUD_RIGHT: usize = HUD.x + HUD.width;
        const HUD_BOTTOM: usize = HUD.y + HUD.height;
        const AROUND_HUD: [Window; 5] = [
            Window::new(BADGE.width, 0, WIDTH - BADGE.width, BADGE.height),
            Window::new(0, BADGE.height, WIDTH, HUD.y - BADGE.height),
            Window::new(0, HUD.y, HUD.x, HUD.height),
            Window::new(HUD_RIGHT, HUD.y, WIDTH - HUD_RIGHT, HUD.height),
            Window::new(0, HUD_BOTTOM, WIDTH, WIDTH - HUD_BOTTOM),
        ];
        let windows: &[_] = match self {
            Self::Full => &[FULL],
            Self::ChannelHud => &AROUND_HUD,
            Self::Swap => &[BODY],
            Self::SwapWithPicker => &EXPOSED,
        };
        windows
            .iter()
            .filter_map(move |area| source.intersection(*area))
    }
}

/// Uncovered pieces of a new instrument faceplate. LCD coordinates fit in one byte.
#[derive(Clone, Copy)]
struct Area {
    left: u8,
    top: u8,
    right: u8,
    bottom: u8,
}

impl Area {
    fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            left: x as u8,
            top: y as u8,
            right: (x + width) as u8,
            bottom: (y + height) as u8,
        }
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let cut = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (cut.left < cut.right && cut.top < cut.bottom).then_some(cut)
    }

    fn around(self, cut: Self) -> [Self; 4] {
        [
            Self {
                bottom: cut.top,
                ..self
            },
            Self {
                top: cut.bottom,
                ..self
            },
            Self {
                right: cut.left,
                top: cut.top,
                bottom: cut.bottom,
                ..self
            },
            Self {
                left: cut.right,
                top: cut.top,
                bottom: cut.bottom,
                ..self
            },
        ]
    }
}

pub(super) struct FaceBackground {
    color: u16,
    gaps: heapless::Vec<Area, 40>,
}

impl FaceBackground {
    pub(super) fn new(header: u16, footer: u16, color: u16, viewport: Viewport) -> Self {
        let mut gaps = heapless::Vec::new();
        for face in [
            Window::new(0, 0, WIDTH, usize::from(header)),
            Window::new(0, WIDTH - usize::from(footer), WIDTH, usize::from(footer)),
        ] {
            for w in viewport.windows(face) {
                let _ = gaps.push(Area::new(
                    w.x as u16,
                    w.y as u16,
                    w.width as u16,
                    w.height as u16,
                ));
            }
        }
        Self { color, gaps }
    }

    // The widgets are opaque. Keep only the parts of the faceplate they leave bare.
    // If a future layout fragments the list further, clear the remaining background
    // before its next widget. All previously written artwork is outside these gaps.
    fn cover(&mut self, tile: Area) -> bool {
        for i in (0..self.gaps.len()).rev() {
            let area = self.gaps[i];
            if let Some(cut) = area.intersection(tile) {
                let pieces = area.around(cut);
                let pieces = pieces
                    .into_iter()
                    .filter(|a| a.left < a.right && a.top < a.bottom);
                if self.gaps.len() - 1 + pieces.clone().count() > self.gaps.capacity() {
                    return false;
                }
                self.gaps.swap_remove(i);
                self.gaps.extend(pieces);
            }
        }
        true
    }
}

pub(super) struct PanelDisplay<'a> {
    lcd: AsyncDisplay<'a, Clock>,
    viewport: Viewport,
    background: Option<&'a mut FaceBackground>,
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
            background: None,
        })
    }

    pub(super) fn complete_face(&mut self, background: &'a mut FaceBackground) {
        self.background = Some(background);
    }

    pub(super) async fn paint_background(&mut self) -> Result<(), Error> {
        if let Some(background) = self.background.take() {
            for a in &background.gaps {
                self.lcd
                    .fill_rect(
                        u16::from(a.left),
                        u16::from(a.top),
                        u16::from(a.right - a.left),
                        u16::from(a.bottom - a.top),
                        background.color,
                    )
                    .await?;
            }
            self.lcd.flush().await?;
        }
        Ok(())
    }

    async fn cover(&mut self, x: u16, y: u16, w: u16, h: u16) -> Result<(), Error> {
        if self
            .background
            .as_mut()
            .is_some_and(|b| !b.cover(Area::new(x, y, w, h)))
        {
            self.paint_background().await?;
        }
        Ok(())
    }

    pub(super) async fn fill_rect(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        color: u16,
    ) -> Result<(), Error> {
        self.cover(x, y, w, h).await?;
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
        self.cover(x, y, w, h).await?;
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
