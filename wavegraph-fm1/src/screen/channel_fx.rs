//! Two channel insert slots above a pre-fader scope, with the shared sends below it.

use super::{
    BAND, WIDTH,
    mixer::{BACKGROUND, Band, LABEL, OUTLINE, POINTER, SELECTED, percent},
};
use crate::{
    firmware::Labels,
    fx::{BassFx, KeySource},
    instrument::Part,
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
    compressor: Option<BassFx>,
    editing: bool,
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
        badge: &mut super::badge::ChannelBadge,
    ) -> Result<(), Error> {
        let strip = labels.mixer.0[labels.channel.index()];
        let controls = Controls {
            compressor: (labels.focus == Part::Bass).then_some(labels.bass_fx),
            editing: labels.bass_insert,
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
                        old.compressor != controls.compressor || old.editing != controls.editing
                    }
                    56..96 => false,
                    96..112 => old.editing != controls.editing,
                    _ => old.editing != controls.editing || old.sends != controls.sends,
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
                16..56 => insert(&mut band, 16, "1", controls.compressor, controls.editing),
                56..96 => insert(&mut band, 56, "2", None, false),
                96..112 => {
                    band.title("POST FX", Point::new(8, 97), POINTER);
                    band.label("PRE-FADER", Point::new(65, 99), Alignment::Left, LABEL);
                    band.rect(Point::new(0, 111), Size::new(u32::from(SIZE), 1), OUTLINE);
                    if controls.compressor.is_some() {
                        band.label(
                            if controls.editing {
                                "SEL BYPASS"
                            } else {
                                "SELECT INSERT"
                            },
                            Point::new(231, 99),
                            Alignment::Right,
                            LABEL,
                        );
                    }
                }
                _ => {
                    let ink = if controls.editing {
                        LABEL
                    } else {
                        palette::CYAN
                    };
                    if !controls.editing {
                        band.rect(Point::new(4, 225), Size::new(232, 14), SELECTED);
                    }
                    band.label(
                        if controls.editing {
                            "SELECT SENDS"
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
            band.compose_badge(badge);
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
fn insert(band: &mut Band<'_>, y: i32, slot: &str, fx: Option<BassFx>, selected: bool) {
    let ink = band.rack_row(y, selected);
    band.label(slot, Point::new(11, y + 7), Alignment::Left, LABEL);
    let Some(fx) = fx else {
        band.title("NONE", Point::new(26, y + 5), LABEL);
        return;
    };
    band.title("303 COMP", Point::new(26, y + 5), palette::WHITE);
    band.label(
        if fx.enabled { "ON" } else { "BYPASS" },
        Point::new(26, y + 23),
        Alignment::Left,
        if fx.enabled { ink } else { LABEL },
    );
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
            u32::from(fx.split_hz - 40) * 128 / (1000 - 40),
            split.as_str(),
        ),
    ] {
        band.rack_dial(
            Point::new(x, y),
            name,
            u8::try_from(position).expect("Q7 dial"),
            value,
            ink,
        );
    }
}
