//! The four global effects share one rack. Repaint only the rows that changed.

use super::{
    BAND, WIDTH,
    mixer::{BACKGROUND, Band, LABEL, OUTLINE, POINTER, SELECTED, STRIP},
};
use crate::fx::{Device, Fx, MAX_DELAY_MS};
use core::fmt::Write;
use fm1_graphics::embedded_graphics::{
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_graphics::palette;
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};
use heapless::String;
use micromath::F32Ext;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Unit {
    Raw,
    Wet,
    Ms,
    Percent,
    Db,
    Ratio,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Dial {
    label: &'static str,
    value: i16,
    min: i16,
    max: i16,
    unit: Unit,
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Dial ranges are bounded, nonempty and fit i32"
)]
impl Dial {
    fn control(
        label: &'static str,
        value: impl Into<i16>,
        min: i16,
        max: i16,
        unit: Unit,
    ) -> Option<Self> {
        Some(Self {
            label,
            value: value.into(),
            min,
            max,
            unit,
        })
    }

    fn position(self) -> u8 {
        let value = i32::from(self.value - self.min) * 128 / i32::from(self.max - self.min);
        u8::try_from(value).expect("in-range rack control")
    }

    fn readout(self) -> String<12> {
        let mut text = String::new();
        let v = self.value;
        match self.unit {
            Unit::Raw => write!(text, "{v}"),
            Unit::Ms => write!(text, "{v}ms"),
            Unit::Wet => write!(text, "{}%", (v * 100 + 64) / 128),
            Unit::Percent => write!(text, "{v}%"),
            Unit::Db if v > 0 => write!(text, "+{v}dB"),
            Unit::Db => write!(text, "{v}dB"),
            Unit::Ratio if v % 2 == 0 => write!(text, "{}:1", v / 2),
            Unit::Ratio => write!(text, "{}.5:1", v / 2),
        }
        .expect("short control readout");
        text
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Row {
    device: Device,
    dials: [Option<Dial>; 4],
    on: bool,
    selected: bool,
}

impl Row {
    fn new(device: Device, fx: Fx, selected: Option<Device>) -> Self {
        use Unit::{Db, Ms, Percent, Ratio, Raw, Wet};
        let dials = match device {
            Device::Reverb => [
                Dial::control("DECAY", fx.decay, 0, 127, Raw),
                Dial::control("RETURN", fx.reverb.wet, 0, 128, Wet),
                None,
                None,
            ],
            Device::Delay => [
                Dial::control(
                    "TIME",
                    i16::try_from(fx.time_ms).expect("delay time"),
                    5,
                    i16::try_from(MAX_DELAY_MS).expect("delay range"),
                    Ms,
                ),
                Dial::control("FDBK", fx.feedback, 0, 95, Percent),
                Dial::control("RETURN", fx.delay.wet, 0, 128, Wet),
                None,
            ],
            Device::Eq => [
                Dial::control("LOW", fx.eq[0], -12, 12, Db),
                Dial::control("MID", fx.eq[1], -12, 12, Db),
                Dial::control("HIGH", fx.eq[2], -12, 12, Db),
                None,
            ],
            Device::Dynamics => {
                let d = fx.dynamics;
                [
                    Dial::control("THRESH", d.threshold_db, -36, 0, Db),
                    Dial::control("RATIO", d.ratio_x2, 2, 16, Ratio),
                    Dial::control("MAKEUP", d.makeup_db, 0, 12, Db),
                    Dial::control("CEIL", d.ceiling_db, -12, 0, Db),
                ]
            }
        };
        Self {
            device,
            dials,
            selected: selected == Some(device),
            on: fx.enabled(device),
        }
    }

    fn status(self, band: &mut Band<'_>, y: i32, reduction: u8) {
        let ink = if self.selected {
            palette::CYAN
        } else {
            POINTER
        };
        let point = Point::new(11, y.saturating_add(23));
        if self.on && self.device == Device::Dynamics {
            let mut text = String::<12>::new();
            write!(text, "GR {reduction}dB").expect("short reduction readout");
            band.label(&text, point, Alignment::Left, ink);
        } else {
            band.label(
                if self.on { "ON" } else { "BYPASS" },
                point,
                Alignment::Left,
                if self.on { ink } else { LABEL },
            );
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "All coordinates fit within one rack row"
    )]
    fn draw(self, band: &mut Band<'_>, y: i32, reduction: u8) {
        let ink = band.rack_row(y, self.selected);
        band.title(
            ["REVERB", "DELAY", "3-BAND EQ", "COMP/LIM"][self.device as usize],
            Point::new(10, y + 5),
            palette::WHITE,
        );
        self.status(band, y, reduction);
        for (x, dial) in [91, 131, 171, 211].into_iter().zip(self.dials) {
            if let Some(d) = dial {
                band.rack_dial(Point::new(x, y), d.label, d.position(), &d.readout(), ink);
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Rack {
    previous: Option<(Fx, Option<Device>)>,
    gr_db: u8,
}

impl Rack {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "All bands align to the 240-pixel display"
    )]
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]; BAND * WIDTH],
        state: (Fx, Option<Device>),
        reduction: u8,
        badge: &mut super::badge::ChannelBadge,
    ) -> Result<(), Error> {
        let db = gr_db(reduction);
        if self.previous == Some(state) && self.gr_db == db {
            return Ok(());
        }
        let (fx, selected) = state;
        let editing = selected.is_some();
        let rows = Device::ALL.map(|d| Row::new(d, fx, selected));
        let dirty = self
            .previous
            .map(|(old, focus)| Device::ALL.map(|d| Row::new(d, old, focus) != rows[d as usize]));
        let footer_changed = self
            .previous
            .is_none_or(|(_, focus)| focus.is_some() != editing);
        let mut lcd = lcd.as_async(clock)?;
        for y in (0..SIZE).step_by(BAND) {
            let row = match y {
                32..72 => Some((0, 32)),
                72..112 => Some((1, 72)),
                128..168 => Some((2, 128)),
                168..208 => Some((3, 168)),
                _ => None,
            };
            if let Some(dirty) = dirty {
                match row {
                    Some((i, _)) if dirty[i] => (),
                    None if y >= 208 && footer_changed => (),
                    _ => continue,
                }
            }
            let mut band = Band::new(pixels, 0, y, SIZE, BAND as u16, BACKGROUND);
            if let Some((i, top)) = row {
                rows[i].draw(&mut band, top, db);
            } else if y < 32 {
                band.title("GLO / MASTER FX", Point::new(8, 2), palette::WHITE);
                band.label("SEND RETURNS", Point::new(8, 19), Alignment::Left, POINTER);
                band.rect(Point::new(84, 24), Size::new(150, 1), OUTLINE);
            } else if y < 128 {
                band.label("MASTER BUS", Point::new(8, 115), Alignment::Left, POINTER);
                band.rect(Point::new(74, 120), Size::new(160, 1), OUTLINE);
            } else {
                let (top, bottom) = if editing {
                    ("KNOBS EDIT     SEL BYPASS", "SELECT DEVICE     HOME BACK")
                } else {
                    ("SELECT DEVICE", "FX MIXER     GLO CHANNEL FX")
                };
                band.label(top, Point::new(120, 211), Alignment::Center, POINTER);
                band.label(bottom, Point::new(120, 225), Alignment::Center, LABEL);
            }
            band.compose_badge(badge);
            lcd.write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
                .await?;
        }
        if dirty.is_some_and(|d| !d[3]) && fx.dynamics_enabled && self.gr_db != db {
            let face = if rows[3].selected { SELECTED } else { STRIP };
            let mut band = Band::new(pixels, 10, 191, 61, 10, face);
            rows[3].status(&mut band, 168, db);
            lcd.write_rect_buffered(10, 191, 61, 10, band.pixels.as_flattened())
                .await?;
        }
        lcd.flush().await?;
        self.previous = Some(state);
        self.gr_db = db;
        Ok(())
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "The bounded Q7 gain maps to 0–42 dB of attenuation"
)]
fn gr_db(reduction: u8) -> u8 {
    let gain = f32::from(128_u8.saturating_sub(reduction).max(1)) / 128.0;
    F32Ext::round(-6.020_6 * F32Ext::log2(gain)) as u8
}
