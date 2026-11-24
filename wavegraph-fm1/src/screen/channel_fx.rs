//! Two channel insert slots above a pre-fader scope, with the shared sends below it.

use super::{
    BAND, Selector, WIDTH,
    mixer::{BACKGROUND, Band, LABEL, OUTLINE, POINTER, SELECTED, percent},
};
use crate::{
    firmware::Labels,
    fx::{INSERT_SLOTS, Insert, KeySource},
};
use core::{ops::Range, str};
use embedded_graphics::{
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_graphics::palette;
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};

pub(super) const SCOPE: Range<u16> = 112..224;

#[derive(Clone, Copy, Eq, PartialEq)]
struct Controls {
    inserts: [Insert; INSERT_SLOTS],
    selected: Option<u8>,
    sends: [u8; 2],
    half_gain: bool,
}

#[derive(Default)]
pub(super) struct Rack {
    previous: Option<Controls>,
}

impl Rack {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]; BAND * WIDTH],
        labels: &Labels,
        entering: bool,
        selector: &mut Selector,
    ) -> Result<(), Error> {
        let strip = labels.mixer.0[labels.channel.index()];
        let controls = Controls {
            inserts: labels.inserts,
            selected: labels.insert_slot,
            sends: [strip.reverb, strip.delay],
            half_gain: labels.half_gain,
        };
        let previous = self.previous.filter(|_| !entering);
        if previous == Some(controls) {
            return Ok(());
        }
        let mut lcd = lcd.as_async(clock)?;
        for y in (0..SCOPE.start).chain(SCOPE.end..SIZE).step_by(BAND) {
            if let Some(old) = previous {
                let dirty = match y {
                    0..16 => old.half_gain != controls.half_gain,
                    16..56 => {
                        old.inserts[0] != controls.inserts[0] || old.selected != controls.selected
                    }
                    56..96 => {
                        old.inserts[1] != controls.inserts[1] || old.selected != controls.selected
                    }
                    96..112 => old.selected != controls.selected,
                    _ => old.selected != controls.selected || old.sends != controls.sends,
                };
                if !dirty {
                    continue;
                }
            }
            // A trim change updates the header without erasing the overload lamp.
            let width = if y < 16 && !entering { SIZE - 12 } else { SIZE };
            let mut band = Band::new(pixels, 0, y, width, BAND as u16, BACKGROUND)
                .with_panel_type(super::lettering::panel_type(labels.focus));
            match y {
                0..16 => {
                    let title = heapless::format!(24; "CH{} / {} FX", labels.channel.index() + 1, labels.focus.name())
                        .expect("channel and instrument title");
                    band.title(&title, Point::new(8, 1), palette::WHITE);
                    band.label(
                        if controls.half_gain {
                            "OUT HALF"
                        } else {
                            "OUT FULL"
                        },
                        Point::new(222, 3),
                        Alignment::Right,
                        LABEL,
                    );
                }
                16..56 => insert(
                    &mut band,
                    16,
                    "1",
                    controls.inserts[0],
                    controls.selected == Some(0),
                ),
                56..96 => insert(
                    &mut band,
                    56,
                    "2",
                    controls.inserts[1],
                    controls.selected == Some(1),
                ),
                96..112 => {
                    band.title("POST FX", Point::new(8, 97), POINTER);
                    band.label("PRE-FADER", Point::new(65, 99), Alignment::Left, LABEL);
                    band.rect(Point::new(0, 111), Size::new(u32::from(SIZE), 1), OUTLINE);
                    band.label(
                        if controls.selected.is_some() {
                            "PRESETS FX"
                        } else {
                            "SELECT SLOT"
                        },
                        Point::new(231, 99),
                        Alignment::Right,
                        LABEL,
                    );
                }
                _ => {
                    let ink = if controls.selected.is_some() {
                        LABEL
                    } else {
                        palette::CYAN
                    };
                    if controls.selected.is_none() {
                        band.rect(Point::new(4, 225), Size::new(232, 14), SELECTED);
                    }
                    band.label(
                        if controls.selected.is_some() {
                            "HOME SENDS"
                        } else {
                            "SENDS"
                        },
                        Point::new(9, 228),
                        Alignment::Left,
                        ink,
                    );
                    for ((x, name), value) in [(87, "2 REV"), (165, "3 DLY")]
                        .into_iter()
                        .zip(controls.sends)
                    {
                        band.label(name, Point::new(x, 228), Alignment::Left, ink);
                        band.label(
                            str::from_utf8(&percent(value)).expect("ASCII percent"),
                            Point::new(x + 30, 228),
                            Alignment::Left,
                            ink,
                        );
                    }
                }
            }
            band.compose(selector);
            band.compose_badge(&mut selector.badge);
            lcd.write_rect_buffered(0, y, width, BAND as u16, band.pixels.as_flattened())
                .await?;
        }
        lcd.flush().await?;
        self.previous = Some(controls);
        Ok(())
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Control ranges and both rack rows fit u32 and i32"
)]
fn insert(band: &mut Band<'_>, y: i32, slot: &str, fx: Insert, selected: bool) {
    let ink = band.rack_row(y, selected);
    band.label(slot, Point::new(11, y + 7), Alignment::Left, LABEL);
    if fx == Insert::None {
        band.title("NONE", Point::new(26, y + 5), LABEL);
        return;
    }
    let name = match fx {
        Insert::Compressor(_) => "COMP",
        Insert::Saturator(_) => "SAT",
        Insert::Phaser(_) => "PHASER",
        Insert::None => "NONE",
    };
    band.title(name, Point::new(26, y + 5), palette::WHITE);
    band.label(
        if fx.enabled() { "ON" } else { "BYPASS" },
        Point::new(26, y + 23),
        Alignment::Left,
        if fx.enabled() { ink } else { LABEL },
    );
    let mut dial = |x, name, position, value: &str| {
        band.rack_dial(Point::new(x, y), name, position, value, ink);
    };
    match fx {
        Insert::None => (),
        Insert::Compressor(fx) => {
            let depth = heapless::format!(4; "{}%", fx.depth).expect("percentage");
            let split = heapless::format!(6; "{}Hz", fx.split_hz).expect("crossover frequency");
            let key_max = u32::try_from(KeySource::ALL.len() - 1).expect("detector sources");
            for (x, name, position, value) in [
                (
                    112,
                    "1 KEY",
                    u32::from(fx.key as u8) * 128 / key_max,
                    fx.key.name(),
                ),
                (
                    164,
                    "2 DEPTH",
                    u32::from(fx.depth) * 128 / 100,
                    depth.as_str(),
                ),
                (
                    211,
                    "3 SPLIT",
                    u32::from(fx.split_hz - 40) * 128 / 960,
                    split.as_str(),
                ),
            ] {
                dial(x, name, u8::try_from(position).expect("Q7 dial"), value);
            }
        }
        Insert::Saturator(fx) => {
            let drive = 10 + u16::from(fx.drive) * 70 / 128;
            let drive = heapless::format!(5; "{}.{}x", drive / 10, drive % 10).expect("drive");
            dial(112, "1 DRIVE", fx.drive, &drive);
            for (x, name, position) in [(164, "2 MIX", fx.mix), (211, "3 LEVEL", fx.level)] {
                dial(
                    x,
                    name,
                    position,
                    str::from_utf8(&percent(position)).expect("ASCII percent"),
                );
            }
        }
        Insert::Phaser(fx) => {
            let rate = fx.centihz();
            let rate = heapless::format!(6; "{}.{:02}", rate / 100, rate % 100).expect("rate");
            dial(104, "1 HZ", fx.rate, &rate);
            for (x, name, position) in [
                (143, "2 DEP", fx.depth),
                (182, "3 FB", fx.feedback),
                (221, "4 MIX", fx.mix),
            ] {
                dial(
                    x,
                    name,
                    position,
                    str::from_utf8(&percent(position)).expect("ASCII percent"),
                );
            }
        }
    }
}
