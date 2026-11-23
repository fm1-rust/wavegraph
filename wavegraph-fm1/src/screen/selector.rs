//! Two-bit foreground mask over the live scope. Zero leaves the instrument visible.

use crate::{
    instrument::Part,
    panel::{CHANNELS, Channel},
    scope::Window,
};
use embedded_graphics::{
    Drawable,
    framebuffer::{Framebuffer, buffer_size},
    mono_font::{MonoTextStyle, ascii::FONT_9X15_BOLD},
    pixelcolor::{
        Gray2, RgbColor,
        raw::{BigEndian, RawU2, ToBytes},
    },
    prelude::{DrawTarget, Point, Primitive, Size},
    primitives::{PrimitiveStyleBuilder, Rectangle, RoundedRectangle},
    text::{Baseline, Text},
};
use fm1_graphics::{Rgb565, WaveformStyle, palette};

const WIDTH: usize = 144;
const ROW_HEIGHT: usize = 21;
const HEIGHT: usize = Part::ALL.len() * ROW_HEIGHT;
pub(super) const LEFT: usize = (crate::capture::WIDTH - WIDTH) / 2;
pub(super) const RIGHT: usize = LEFT + WIDTH;
pub(super) const TOP: usize = (crate::capture::WIDTH - HEIGHT) / 2;
pub(super) const BOTTOM: usize = TOP + HEIGHT;

type Mask =
    Framebuffer<Gray2, RawU2, BigEndian, WIDTH, HEIGHT, { buffer_size::<Gray2>(WIDTH, HEIGHT) }>;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Menu {
    Channels {
        parts: [Part; CHANNELS],
        focus: Channel,
    },
    Instruments(Part),
}

pub(super) struct Selector {
    menu: Option<Menu>,
    mask: Mask,
    pub(super) badge: super::badge::ChannelBadge,
}

impl Selector {
    pub(super) const fn new() -> Self {
        Self {
            menu: None,
            mask: Mask::new(),
            badge: super::badge::ChannelBadge::new(),
        }
    }

    pub(super) fn select(&mut self, menu: Option<Menu>) -> bool {
        if self.menu == menu {
            return false;
        }
        self.menu = menu;
        if let Some(menu) = menu {
            self.mask
                .clear(Gray2::new(u8::from(matches!(menu, Menu::Instruments(_)))))
                .expect("infallible framebuffer");
            let text = MonoTextStyle::new(&FONT_9X15_BOLD, Gray2::new(3));
            let highlight = PrimitiveStyleBuilder::new()
                .fill_color(Gray2::new(1))
                .stroke_color(Gray2::new(3))
                .stroke_width(2)
                .build();
            let (parts, active, numbered): (&[Part], _, _) = match &menu {
                Menu::Channels { parts, focus } => (parts, focus.index(), true),
                Menu::Instruments(part) => (&Part::ALL, *part as usize, false),
            };
            let offset = (HEIGHT - parts.len() * ROW_HEIGHT) / 2;
            for (row, part) in parts.iter().enumerate() {
                let y = (offset + row * ROW_HEIGHT) as i32;
                if row == active {
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
                if numbered {
                    Text::with_baseline(
                        ["1", "2", "3", "4"][row],
                        Point::new(8, y + 3),
                        text,
                        Baseline::Top,
                    )
                    .draw(&mut self.mask)
                    .expect("infallible framebuffer");
                }
                Text::with_baseline(
                    part.name(),
                    Point::new(if numbered { 27 } else { 8 }, y + 3),
                    text,
                    Baseline::Top,
                )
                .draw(&mut self.mask)
                .expect("infallible framebuffer");
            }
        }
        true
    }

    pub(super) fn visible(&self) -> bool {
        self.menu.is_some()
    }

    pub(super) fn shade(&self, color: Rgb565) -> Rgb565 {
        self.shader()(color)
    }

    pub(super) fn shader(&self) -> impl Fn(Rgb565) -> Rgb565 + use<> {
        let dimmed = self.visible();
        move |color| {
            if dimmed {
                Rgb565::new(color.r() / 2, color.g() / 2, color.b() / 2)
            } else {
                color
            }
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
        self.ink(x, y).unwrap_or(background)
    }

    pub(super) fn bounds(&self) -> Option<Window> {
        self.visible().then_some(Window {
            x: LEFT,
            y: TOP,
            width: WIDTH,
            height: HEIGHT,
        })
    }

    pub(super) fn overlay(&self, w: Window, pixels: &mut [[u8; 2]]) {
        if !self.visible() {
            return;
        }
        for y in TOP.max(w.y)..BOTTOM.min(w.y + w.height) {
            for x in LEFT.max(w.x)..RIGHT.min(w.x + w.width) {
                if let Some(ink) = self.ink(x, y) {
                    pixels[(y - w.y) * w.width + x - w.x] = ink.to_be_bytes();
                }
            }
        }
    }

    fn ink(&self, x: usize, y: usize) -> Option<Rgb565> {
        let i = (y - TOP) * WIDTH + x - LEFT;
        match (self.mask.data()[i / 4] >> (6 - 2 * (i % 4))) & 3 {
            0 => None,
            1 => Some(Rgb565::BLACK),
            _ => Some(palette::WHITE),
        }
    }
}
