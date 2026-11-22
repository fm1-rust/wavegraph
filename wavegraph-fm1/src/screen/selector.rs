//! Two-bit foreground mask over the live scope. Zero leaves the instrument visible.

use crate::instrument::Part;
use embedded_graphics::{
    Drawable,
    framebuffer::{Framebuffer, buffer_size},
    mono_font::{MonoTextStyle, ascii::FONT_9X15_BOLD},
    pixelcolor::{
        Gray2, RgbColor,
        raw::{BigEndian, RawU2},
    },
    prelude::{DrawTarget, Point, Primitive, Size},
    primitives::{PrimitiveStyleBuilder, Rectangle, RoundedRectangle},
    text::{Baseline, Text},
};
use fm1_graphics::{Rgb565, WaveformStyle, palette};

const WIDTH: usize = 144;
const ROW_HEIGHT: usize = 21;
const HEIGHT: usize = 3 * ROW_HEIGHT;
pub(super) const LEFT: usize = (crate::capture::WIDTH - WIDTH) / 2;
pub(super) const RIGHT: usize = LEFT + WIDTH;
pub(super) const TOP: usize = (crate::capture::WIDTH - HEIGHT) / 2;
pub(super) const BOTTOM: usize = TOP + HEIGHT;

type Mask =
    Framebuffer<Gray2, RawU2, BigEndian, WIDTH, HEIGHT, { buffer_size::<Gray2>(WIDTH, HEIGHT) }>;

pub(super) struct Selector {
    focus: Option<Part>,
    mask: Mask,
}

impl Selector {
    pub(super) const fn new() -> Self {
        Self {
            focus: None,
            mask: Mask::new(),
        }
    }

    pub(super) fn select(&mut self, focus: Option<Part>) -> bool {
        if self.focus == focus {
            return false;
        }
        self.focus = focus;
        if let Some(focus) = focus {
            self.mask
                .clear(Gray2::new(0))
                .expect("infallible framebuffer");
            let text = MonoTextStyle::new(&FONT_9X15_BOLD, Gray2::new(3));
            let highlight = PrimitiveStyleBuilder::new()
                .fill_color(Gray2::new(1))
                .stroke_color(Gray2::new(3))
                .stroke_width(2)
                .build();
            for (row, (part, name)) in [
                (Part::Mono, "001 MONOSYNTH"),
                (Part::Organ, "002 M1 ORGAN"),
                (Part::Bass, "003 TB-303"),
            ]
            .into_iter()
            .enumerate()
            {
                let y = (row * ROW_HEIGHT) as i32;
                if part == focus {
                    RoundedRectangle::with_equal_corners(
                        Rectangle::new(
                            Point::new(0, y),
                            Size::new(WIDTH as u32, ROW_HEIGHT as u32),
                        ),
                        Size::new(4, 4),
                    )
                    .into_styled(highlight)
                    .draw(&mut self.mask)
                    .expect("infallible framebuffer");
                }
                Text::with_baseline(name, Point::new(8, y + 3), text, Baseline::Top)
                    .draw(&mut self.mask)
                    .expect("infallible framebuffer");
            }
        }
        true
    }

    pub(super) fn visible(&self) -> bool {
        self.focus.is_some()
    }

    pub(super) fn shade(&self, color: Rgb565) -> Rgb565 {
        if self.visible() {
            Rgb565::new(color.r() / 2, color.g() / 2, color.b() / 2)
        } else {
            color
        }
    }

    pub(super) fn style(&self, style: WaveformStyle) -> WaveformStyle {
        WaveformStyle {
            background: self.shade(style.background),
            grid: self.shade(style.grid),
            center: self.shade(style.center),
            trace: self.shade(style.trace),
            ..style
        }
    }

    pub(super) fn pixel(&self, x: usize, y: usize, background: Rgb565) -> Rgb565 {
        if !self.visible() || !(LEFT..RIGHT).contains(&x) || !(TOP..BOTTOM).contains(&y) {
            return background;
        }
        let i = (y - TOP) * WIDTH + x - LEFT;
        match (self.mask.data()[i / 4] >> (6 - 2 * (i % 4))) & 3 {
            0 => background,
            1 => Rgb565::BLACK,
            _ => palette::WHITE,
        }
    }
}
