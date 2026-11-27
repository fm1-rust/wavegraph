//! Two channel insert slots above a pre-fader scope, with the shared sends below it.

use super::{
    BAND, InstrumentStyle, Selector, WIDTH,
    mixer::{Band, RackColors, percent},
};
use crate::{
    firmware::Labels,
    fx::{INSERT_SLOTS, Insert, KeySource},
    panel::Channel,
};
use core::{ops::Range, str};
use fm1_graphics::embedded_graphics::{
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};

pub(super) const SCOPE: Range<u16> = 128..224;

#[derive(Clone, Copy, Eq, PartialEq)]
struct Controls {
    channel: Channel,
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
        (theme, entering): (&InstrumentStyle, bool),
        selector: &mut Selector,
    ) -> Result<(), Error> {
        let strip = labels.mixer.0[labels.channel.index()];
        let controls = Controls {
            channel: labels.channel,
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
                    0..16 => old.channel != controls.channel,
                    16..32 => {
                        old.channel != controls.channel || old.half_gain != controls.half_gain
                    }
                    32..72 => {
                        old.inserts[0] != controls.inserts[0] || old.selected != controls.selected
                    }
                    72..112 => {
                        old.inserts[1] != controls.inserts[1] || old.selected != controls.selected
                    }
                    112..128 => old.selected != controls.selected,
                    _ => old.selected != controls.selected || old.sends != controls.sends,
                };
                if !dirty {
                    continue;
                }
            }
            let width = if previous.is_some_and(|old| y < 32 && old.half_gain == controls.half_gain)
            {
                super::lettering::BADGE.width as u16
            } else {
                SIZE
            };
            let mut band = Band::new(pixels, 0, y, width, BAND as u16, theme.face)
                .with_panel_type(super::lettering::panel_type(labels.focus));
            match y {
                0..32 => {
                    band.fx_tabs(labels.channel, labels.view, theme);
                    band.title(labels.focus.name(), Point::new(29, 17), theme.ink);
                    band.label(
                        if controls.half_gain {
                            "OUT HALF"
                        } else {
                            "OUT FULL"
                        },
                        Point::new(232, 19),
                        Alignment::Right,
                        theme.ink,
                    );
                }
                32..72 => insert(
                    &mut band,
                    32,
                    "1",
                    controls.inserts[0],
                    controls.selected == Some(0),
                    theme,
                ),
                72..112 => insert(
                    &mut band,
                    72,
                    "2",
                    controls.inserts[1],
                    controls.selected == Some(1),
                    theme,
                ),
                112..128 => {
                    band.title("POST FX", Point::new(5, 113), theme.accent);
                    band.label(
                        if controls.selected.is_some() {
                            "ALGO FX TYPE"
                        } else {
                            "PRE-FADER"
                        },
                        Point::new(60, 115),
                        Alignment::Left,
                        theme.ink,
                    );
                    band.rect(
                        Point::new(0, 127),
                        Size::new(u32::from(SIZE), 1),
                        theme.muted,
                    );
                    band.label(
                        "TAP FX: BEAT FX",
                        Point::new(235, 115),
                        Alignment::Right,
                        theme.ink,
                    );
                }
                _ => {
                    let (ink, label) = if controls.selected.is_none() {
                        band.rect(Point::new(4, 225), Size::new(232, 14), theme.selected.fill);
                        (theme.selected.ink, "SENDS")
                    } else {
                        (theme.ink, "HOME SENDS")
                    };
                    band.label(label, Point::new(9, 228), Alignment::Left, ink);
                    for ((x, name), value) in [(81, "2 REV"), (158, "3 ECHO")]
                        .into_iter()
                        .zip(controls.sends)
                    {
                        band.label(name, Point::new(x, 228), Alignment::Left, ink);
                        band.label(
                            str::from_utf8(&percent(value)).expect("ASCII percent"),
                            Point::new(x + 40, 228),
                            Alignment::Left,
                            ink,
                        );
                    }
                }
            }
            band.compose(selector);
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
fn insert(
    band: &mut Band<'_>,
    y: i32,
    slot: &str,
    fx: Insert,
    selected: bool,
    theme: &InstrumentStyle,
) {
    let colors = RackColors::channel_row(theme, selected);
    band.rack_row(y, selected, &colors);
    band.label(slot, Point::new(11, y + 7), Alignment::Left, colors.label);
    if fx == Insert::None {
        band.title("NONE", Point::new(26, y + 5), colors.label);
        return;
    }
    let name = match fx {
        Insert::Compressor(_) => "COMP",
        Insert::Saturator(_) => "SAT",
        Insert::Phaser(_) => "PHASER",
        Insert::Flanger(_) => "FLANGER",
        Insert::Chorus(_) => "CHORUS",
        Insert::Eq(_) => "EQ",
        Insert::None => "NONE",
    };
    band.title(name, Point::new(26, y + 5), colors.ink);
    band.label(
        if fx.enabled() { "ON" } else { "BYPASS" },
        Point::new(26, y + 23),
        Alignment::Left,
        colors.label,
    );
    let mut dial = |x, name, position, value: &str| {
        band.rack_dial(Point::new(x, y), name, position, value, &colors);
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
            dial(87, "1 DRIVE", fx.drive, &drive);
            for (x, name, position) in [
                (130, "2 MIX", fx.mix),
                (173, "3 LEVEL", fx.level),
                (216, "4 SHAPE", fx.shape),
            ] {
                dial(
                    x,
                    name,
                    position,
                    str::from_utf8(&percent(position)).expect("ASCII percent"),
                );
            }
        }
        Insert::Eq(fx) => {
            for ((x, name), db) in [(112, "1 LOW"), (164, "2 MID"), (211, "3 HIGH")]
                .into_iter()
                .zip(fx.db)
            {
                let text = heapless::format!(5; "{db:+}dB").expect("signed EQ gain");
                let position = (i16::from(db) + 12) * 128 / 24;
                dial(x, name, u8::try_from(position).expect("Q7 dial"), &text);
            }
        }
        Insert::Chorus(fx) => {
            let mhz = 150 + 12 * u16::from(fx.rate);
            let text =
                heapless::format!(6; "{}.{:02}", mhz / 1000, (mhz % 1000) / 10).expect("rate");
            dial(104, "1 HZ", fx.rate, &text);
            for (x, name, value) in [(143, "2 DEP", fx.depth), (182, "3 TONE", fx.tone)] {
                dial(
                    x,
                    name,
                    value,
                    str::from_utf8(&percent(value)).expect("ASCII percent"),
                );
            }
            let mix = heapless::format!(5; "{}%", fx.mix).expect("percent");
            dial(
                221,
                "4 MIX",
                u8::try_from(u16::from(fx.mix) * 128 / 100).expect("Q7"),
                &mix,
            );
        }
        Insert::Phaser(fx) | Insert::Flanger(fx) => {
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
