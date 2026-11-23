//! Four horizontal strips. Only changed strips and meter rectangles go to the LCD.

use super::{BAND, Selector, WIDTH, selector};
use crate::{
    firmware::Labels,
    mixer::{MAX_LEVEL, SEGMENTS, Strip, UNITY},
    panel::{CHANNELS, View},
};
use core::{convert::Infallible, str};
use embedded_graphics::{
    Drawable, Pixel,
    draw_target::DrawTarget,
    geometry::OriginDimensions,
    mono_font::{
        MonoTextStyle,
        ascii::{FONT_6X10, FONT_7X13_BOLD},
    },
    pixelcolor::raw::ToBytes,
    prelude::{Point, Primitive, Size},
    primitives::{Circle, Line, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use fm1_graphics::{Rgb565, palette};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};
use micromath::F32Ext;

pub(super) const BACKGROUND: Rgb565 = Rgb565::new(2, 4, 6);
const STRIP: Rgb565 = Rgb565::new(4, 9, 12);
const SELECTED: Rgb565 = Rgb565::new(5, 19, 22);
const KNOB: Rgb565 = Rgb565::new(7, 15, 19);
const OUTLINE: Rgb565 = Rgb565::new(13, 28, 22);
const POINTER: Rgb565 = Rgb565::new(22, 47, 28);
const GREEN: Rgb565 = Rgb565::new(5, 55, 17);
const AMBER: Rgb565 = Rgb565::new(31, 44, 1);
const RED: Rgb565 = Rgb565::new(31, 11, 8);
const MOTION_OFF: Rgb565 = Rgb565::new(2, 6, 8);
const LABEL: Rgb565 = Rgb565::new(20, 41, 25);

const TOP: u16 = 23;
const STRIP_HEIGHT: u16 = 48;
const METER_X: u16 = 10;
const METER_Y: u16 = 29;
const METER_WIDTH: u16 = 106;
const METER_HEIGHT: u16 = 8;
const CENTERS: [i32; 3] = [137, 176, 215];

#[derive(Clone, Copy, Eq, PartialEq)]
struct ChannelStrip {
    part: crate::instrument::Part,
    controls: Strip,
    active: bool,
    selected: bool,
}

#[derive(Default)]
pub(super) struct Surface {
    strips: Option<[ChannelStrip; CHANNELS]>,
    peaks: [u8; CHANNELS],
    editing: bool,
}

impl Surface {
    pub(super) fn reset(&mut self) {
        self.strips = None;
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Mixer bands, strips and the footer are bounded by the display dimensions"
    )]
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]; BAND * WIDTH],
        selector: &Selector,
        labels: Labels,
        peaks: [u8; CHANNELS],
    ) -> Result<(), Error> {
        let editing = labels.view == View::Channel;
        let mut lcd = lcd.as_async(clock)?;
        if self.strips.is_none() {
            for y in (0..TOP).step_by(BAND) {
                let mut band =
                    Band::new(pixels, 0, y, SIZE, (TOP - y).min(BAND as u16), BACKGROUND);
                band.title("GLO / MIXER", Point::new(8, 5), palette::WHITE);
                for (x, label) in CENTERS.into_iter().zip(["1 LVL", "2 REV", "3 DLY"]) {
                    band.label(label, Point::new(x, 9), Alignment::Center, LABEL);
                }
                band.compose(selector);
                lcd.write_rect_buffered(0, y, SIZE, band.height, band.pixels.as_flattened())
                    .await?;
            }
        }
        let strips = core::array::from_fn(|i| ChannelStrip {
            part: labels.assignments[i],
            controls: labels.mixer.0[i],
            active: labels.channel.index() == i,
            selected: editing && labels.channel.index() == i,
        });
        for (i, strip) in strips.into_iter().enumerate() {
            let y = TOP + STRIP_HEIGHT * i as u16;
            if self.strips.is_none_or(|previous| previous[i] != strip) {
                for row in (y..y + STRIP_HEIGHT).step_by(BAND) {
                    let mut band = Band::new(pixels, 0, row, SIZE, BAND as u16, BACKGROUND);
                    band.strip(i, y, strip, peaks[i]);
                    band.compose(selector);
                    lcd.write_rect_buffered(0, row, SIZE, BAND as u16, band.pixels.as_flattened())
                        .await?;
                }
            } else if peaks[i] != self.peaks[i] {
                let mut band = Band::new(
                    pixels,
                    METER_X,
                    y + METER_Y,
                    METER_WIDTH,
                    METER_HEIGHT,
                    strip.background(),
                );
                band.meter(y, peaks[i]);
                band.compose(selector);
                lcd.write_rect_buffered(
                    METER_X,
                    y + METER_Y,
                    METER_WIDTH,
                    METER_HEIGHT,
                    band.pixels.as_flattened(),
                )
                .await?;
            }
        }
        if self.strips.is_none() || self.editing != editing {
            let footer = TOP + STRIP_HEIGHT * CHANNELS as u16;
            for y in (footer..SIZE).step_by(BAND) {
                let mut band =
                    Band::new(pixels, 0, y, SIZE, (SIZE - y).min(BAND as u16), BACKGROUND);
                let hint = if editing {
                    "PRESETS ASSIGN       HOME BACK"
                } else {
                    "SELECT EDIT       PRESETS CHANNEL"
                };
                band.label(
                    hint,
                    Point::new(i32::from(SIZE / 2), 223),
                    Alignment::Center,
                    LABEL,
                );
                band.compose(selector);
                lcd.write_rect_buffered(0, y, SIZE, band.height, band.pixels.as_flattened())
                    .await?;
            }
        }
        lcd.flush().await?;
        self.strips = Some(strips);
        self.peaks = peaks;
        self.editing = editing;
        Ok(())
    }
}

impl ChannelStrip {
    fn background(self) -> Rgb565 {
        if self.selected { SELECTED } else { STRIP }
    }
}

struct Band<'a> {
    pixels: &'a mut [[u8; 2]],
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "All coordinates and pixel indexes are bounded by the 240 px panel"
)]
impl<'a> Band<'a> {
    fn new(
        storage: &'a mut [[u8; 2]],
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        background: Rgb565,
    ) -> Self {
        let pixels = &mut storage[..usize::from(width) * usize::from(height)];
        pixels.fill(background.to_be_bytes());
        Self {
            pixels,
            x,
            y,
            width,
            height,
        }
    }

    fn strip(&mut self, index: usize, y: u16, strip: ChannelStrip, peak: u8) {
        let y = i32::from(y);
        self.rect(Point::new(5, y), Size::new(230, 45), strip.background());
        if strip.selected {
            self.rect(Point::new(5, y), Size::new(2, 45), palette::CYAN);
        }
        let active = if strip.active { palette::CYAN } else { LABEL };
        self.title(["1", "2", "3", "4"][index], Point::new(11, y + 7), active);
        self.title(strip.part.name(), Point::new(25, y + 7), palette::WHITE);
        self.meter(y as u16, peak);
        let Strip {
            level,
            reverb,
            delay,
        } = strip.controls;
        for (x, value, max, readout) in [
            (CENTERS[0], level, MAX_LEVEL, decibels(level)),
            (CENTERS[1], reverb, UNITY, percent(reverb)),
            (CENTERS[2], delay, UNITY, percent(delay)),
        ] {
            self.knob(Point::new(x, y + 15), value, max, strip.selected);
            self.label(
                str::from_utf8(&readout).expect("ASCII readout"),
                Point::new(x, y + 30),
                Alignment::Center,
                LABEL,
            );
        }
    }

    fn meter(&mut self, y: u16, peak: u8) {
        for segment in 0..SEGMENTS {
            let fill = if segment >= usize::from(peak) {
                MOTION_OFF
            } else {
                match segment {
                    0..=6 => GREEN,
                    7..=10 => AMBER,
                    _ => RED,
                }
            };
            self.rect(
                Point::new(
                    i32::from(METER_X) + 9 * segment as i32,
                    i32::from(y + METER_Y),
                ),
                Size::new(7, u32::from(METER_HEIGHT)),
                fill,
            );
        }
    }

    fn knob(&mut self, center: Point, value: u8, max: u8, selected: bool) {
        const DIRECTIONS: [(i32, i32); 17] = [
            (-5, 5),
            (-6, 4),
            (-7, 3),
            (-7, 1),
            (-7, 0),
            (-6, -3),
            (-5, -5),
            (-3, -6),
            (0, -7),
            (3, -6),
            (5, -5),
            (6, -3),
            (7, 0),
            (7, 1),
            (7, 3),
            (6, 4),
            (5, 5),
        ];
        let position = usize::from(value) * (DIRECTIONS.len() - 1) / usize::from(max);
        let (x, y) = DIRECTIONS[position];
        let body = PrimitiveStyleBuilder::new()
            .fill_color(KNOB)
            .stroke_color(OUTLINE)
            .stroke_width(1)
            .build();
        Circle::with_center(center, 21)
            .into_styled(body)
            .draw(self)
            .expect("infallible strip");
        let ink = if selected { palette::CYAN } else { POINTER };
        Line::new(center, center + Point::new(x, y))
            .into_styled(PrimitiveStyle::with_stroke(ink, 2))
            .draw(self)
            .expect("infallible strip");
    }

    fn rect(&mut self, point: Point, size: Size, color: Rgb565) {
        Rectangle::new(point, size)
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(self)
            .expect("infallible strip");
    }

    fn title(&mut self, name: &str, point: Point, color: Rgb565) {
        Text::with_baseline(
            name,
            point,
            MonoTextStyle::new(&FONT_7X13_BOLD, color),
            Baseline::Top,
        )
        .draw(self)
        .expect("infallible strip");
    }

    fn label(&mut self, name: &str, point: Point, alignment: Alignment, color: Rgb565) {
        Text::with_text_style(
            name,
            point,
            MonoTextStyle::new(&FONT_6X10, color),
            TextStyleBuilder::new()
                .baseline(Baseline::Top)
                .alignment(alignment)
                .build(),
        )
        .draw(self)
        .expect("infallible strip");
    }

    fn compose(&mut self, list: &Selector) {
        if !list.visible() {
            return;
        }
        let width = usize::from(self.width);
        for (i, pixel) in self.pixels.iter_mut().enumerate() {
            let x = usize::from(self.x) + i % width;
            let y = usize::from(self.y) + i / width;
            let [a, b] = *pixel;
            let color = Rgb565::new(a >> 3, ((a & 7) << 3) | (b >> 5), b & 31);
            let background = list.shade(color);
            *pixel = if (selector::LEFT..selector::RIGHT).contains(&x)
                && (selector::TOP..selector::BOTTOM).contains(&y)
            {
                list.pixel(x, y, background)
            } else {
                background
            }
            .to_be_bytes();
        }
    }
}

impl OriginDimensions for Band<'_> {
    fn size(&self) -> Size {
        Size::new(u32::from(SIZE), u32::from(SIZE))
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Pixel indexes only use the clipped band coordinates"
)]
impl DrawTarget for Band<'_> {
    type Color = Rgb565;
    type Error = Infallible;

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), Infallible> {
        let bounds = Rectangle::new(
            Point::new(i32::from(self.x), i32::from(self.y)),
            Size::new(u32::from(self.width), u32::from(self.height)),
        );
        let clipped = area.intersection(&bounds);
        if clipped.size.width != 0 {
            let x = (clipped.top_left.x - i32::from(self.x)) as usize;
            let y = (clipped.top_left.y - i32::from(self.y)) as usize;
            self.pixels
                .chunks_exact_mut(usize::from(self.width))
                .skip(y)
                .take(clipped.size.height as usize)
                .for_each(|row| row[x..x + clipped.size.width as usize].fill(color.to_be_bytes()));
        }
        Ok(())
    }

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Infallible>
    where
        I: IntoIterator<Item = Pixel<Rgb565>>,
    {
        for Pixel(p, color) in pixels {
            let x = p.x - i32::from(self.x);
            let y = p.y - i32::from(self.y);
            if (0..i32::from(self.width)).contains(&x) && (0..i32::from(self.height)).contains(&y) {
                self.pixels[y as usize * usize::from(self.width) + x as usize] =
                    color.to_be_bytes();
            }
        }
        Ok(())
    }
}

fn decibels(gain: u8) -> [u8; 5] {
    if gain == 0 {
        return *b" MUTE";
    }
    // The UI rounds to a whole dB. DSP retains the full Q7 resolution.
    let level = F32Ext::round(6.020_6 * F32Ext::log2(f32::from(gain) / f32::from(UNITY))) as i8;
    let d = level.unsigned_abs();
    let sign = match level.cmp(&0) {
        core::cmp::Ordering::Less => b'-',
        core::cmp::Ordering::Equal => b' ',
        core::cmp::Ordering::Greater => b'+',
    };
    if d >= 10 {
        [sign, digit(d / 10), digit(d % 10), b'd', b'B']
    } else {
        [b' ', sign, digit(d), b'd', b'B']
    }
}

fn percent(gain: u8) -> [u8; 5] {
    let pct = (u16::from(gain) * 100 + 64) / u16::from(UNITY);
    let pct = u8::try_from(pct).expect("unity send");
    [
        if pct == 100 { b'1' } else { b' ' },
        if pct >= 10 {
            digit((pct / 10) % 10)
        } else {
            b' '
        },
        digit(pct % 10),
        b'%',
        b' ',
    ]
}

fn digit(digit: u8) -> u8 {
    b'0'.saturating_add(digit)
}
