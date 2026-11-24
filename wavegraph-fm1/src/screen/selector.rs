//! A one-bit ink mask over the live scope, with a shared mask for the selected row.

use crate::assets::fonts::FONT_9X15_BOLD;
use crate::{
    fx::InsertKind,
    instrument::Part,
    panel::{CHANNELS, Channel},
    scope::Window,
};
use embedded_graphics::{
    Drawable,
    framebuffer::{Framebuffer, buffer_size},
    mono_font::MonoTextStyle,
    pixelcolor::{
        BinaryColor, RgbColor,
        raw::{BigEndian, RawU1, ToBytes},
    },
    prelude::{DrawTarget, Point, Primitive, Size},
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, RoundedRectangle},
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

type Mask<const H: usize, const BYTES: usize> =
    Framebuffer<BinaryColor, RawU1, BigEndian, WIDTH, H, BYTES>;

fn highlight(y: i32) -> RoundedRectangle {
    RoundedRectangle::with_equal_corners(
        Rectangle::new(Point::new(0, y), Size::new(WIDTH as u32, ROW_HEIGHT as u32)),
        Size::new(4, 4),
    )
}

fn marked(mask: &[u8], x: usize, y: usize) -> bool {
    let i = y * WIDTH + x;
    mask[i / 8] & (0x80 >> (i % 8)) != 0
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Menu {
    Channels {
        parts: [Part; CHANNELS],
        focus: Channel,
    },
    Instruments(Part),
    Effects(InsertKind),
}

pub(super) struct Selector {
    menu: Option<Menu>,
    mask: Mask<HEIGHT, { buffer_size::<BinaryColor>(WIDTH, HEIGHT) }>,
    row: Mask<ROW_HEIGHT, { buffer_size::<BinaryColor>(WIDTH, ROW_HEIGHT) }>,
    pub(super) badge: super::badge::ChannelBadge,
}

impl Selector {
    pub(super) fn new() -> Self {
        let mut row = Mask::new();
        highlight(0)
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut row)
            .expect("infallible framebuffer");
        Self {
            menu: None,
            mask: Mask::new(),
            row,
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
                .clear(BinaryColor::Off)
                .expect("infallible framebuffer");
            let text = MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On);
            let selected = PrimitiveStyleBuilder::new()
                .fill_color(BinaryColor::Off)
                .stroke_color(BinaryColor::On)
                .stroke_width(2)
                .build();
            let (parts, active, numbered): (&[Part], _, _) = match &menu {
                Menu::Channels { parts, focus } => (parts, focus.index(), true),
                Menu::Instruments(part) => (&Part::ALL, *part as usize, false),
                Menu::Effects(active) => {
                    let offset = (HEIGHT - InsertKind::ALL.len() * ROW_HEIGHT) / 2;
                    for (row, kind) in InsertKind::ALL.into_iter().enumerate() {
                        let y = (offset + row * ROW_HEIGHT) as i32;
                        if kind == *active {
                            highlight(y)
                                .into_styled(selected)
                                .draw(&mut self.mask)
                                .expect("infallible framebuffer");
                        }
                        Text::with_baseline(kind.name(), Point::new(8, y + 3), text, Baseline::Top)
                            .draw(&mut self.mask)
                            .expect("infallible framebuffer");
                    }
                    return true;
                }
            };
            let offset = (HEIGHT - parts.len() * ROW_HEIGHT) / 2;
            for (row, part) in parts.iter().enumerate() {
                let y = (offset + row * ROW_HEIGHT) as i32;
                if row == active {
                    highlight(y)
                        .into_styled(selected)
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
                if super::lettering::panel_type(*part) {
                    let left = if numbered { 27 } else { 8 };
                    for (i, ch) in part.name().bytes().enumerate() {
                        let mask = super::lettering::GRID.glyph(ch);
                        for dy in 0..mask.height {
                            for dx in 0..mask.width {
                                if mask.coverage(dx, dy) >= 2 {
                                    let p = Point::new(
                                        (left + i * mask.width + dx) as i32,
                                        y + 2 + dy as i32,
                                    );
                                    embedded_graphics::Pixel(p, BinaryColor::On)
                                        .draw(&mut self.mask)
                                        .expect("infallible mask");
                                }
                            }
                        }
                    }
                    continue;
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
        let (x, y) = (x - LEFT, y - TOP);
        if marked(self.mask.data(), x, y) {
            return Some(palette::WHITE);
        }
        if let Menu::Effects(_) = self.menu? {
            let height = InsertKind::ALL.len() * ROW_HEIGHT;
            let top = (HEIGHT - height) / 2;
            if !(top..top + height).contains(&y) {
                return None;
            }
        }
        if let Menu::Channels { focus, .. } = self.menu? {
            let top = (HEIGHT - CHANNELS * ROW_HEIGHT) / 2 + focus.index() * ROW_HEIGHT;
            let row = y.checked_sub(top)?;
            if row >= ROW_HEIGHT || !marked(self.row.data(), x, row) {
                return None;
            }
        }
        Some(Rgb565::BLACK)
    }
}
