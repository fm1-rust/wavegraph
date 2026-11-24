//! TB-303 modulation panel. Only rows whose controls changed are transferred.
use super::{BAND, OXIDE_RED, SILVER, WIDTH, mixer::Band};
use crate::{lfo::Controls, panel::Channel};
use core::fmt::Write;
use fm1_graphics::embedded_graphics::{
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};
use heapless::String;
use wavegraph_303::Destination;
use wavegraph_lfo::Shape;

pub(super) async fn paint(
    lcd: &mut Display,
    clock: &mut Clock,
    pixels: &mut [[u8; 2]; BAND * WIDTH],
    channel: Channel,
    c: Controls,
    previous: Option<Controls>,
) -> Result<(), Error> {
    let shape = match c.shape() {
        Shape::Sine => "SINE",
        Shape::Triangle => "TRIANGLE",
        Shape::Saw => "SAW",
        Shape::Square => "SQUARE",
    };
    let target = match c.target() {
        Destination::Pitch => "PITCH",
        Destination::Cutoff => "CUTOFF",
        Destination::Resonance => "RESONANCE",
        Destination::EnvMod => "ENV MOD",
        Destination::Decay => "DECAY",
        Destination::Accent => "ACCENT",
        Destination::Attack => "ATTACK",
        Destination::Overdrive => "OVERDRIVE",
    };
    let mut rate = String::<12>::new();
    if c.sync {
        rate.push_str(c.division_name()).expect("musical division");
    } else {
        let hz = c.tenths_hz();
        write!(rate, "{}.{} Hz", hz / 10, hz % 10).expect("rate label");
    }
    let mut depth = String::<4>::new();
    write!(depth, "{}%", (u16::from(c.depth) * 100 + 63) / 127).expect("depth percent");
    let rows = [
        ("1  WAVE", shape),
        ("2  RATE", rate.as_str()),
        ("3  DEPTH / CC1", depth.as_str()),
        ("4  TARGET", target),
        ("SELECT", if c.sync { "TEMPO" } else { "FREE" }),
        ("SEL", if c.enabled { "ON" } else { "OFF" }),
    ];
    let mut lcd = lcd.as_async(clock)?;
    for y in (0..SIZE).step_by(BAND) {
        let row = usize::from(y.saturating_sub(40) / 32);
        if previous.is_some_and(|p| {
            y < 40
                || match row {
                    0 => p.shape() == c.shape(),
                    1 => {
                        p.sync == c.sync
                            && p.tenths_hz() == c.tenths_hz()
                            && p.division_name() == c.division_name()
                    }
                    2 => p.depth == c.depth,
                    3 => p.target() == c.target(),
                    4 => p.sync == c.sync,
                    _ => p.enabled == c.enabled,
                }
        }) {
            continue;
        }
        let mut band =
            Band::new(pixels, 0, y, SIZE, BAND as u16, SILVER.background).with_panel_type(true);
        if y < 40 {
            band.title("TB-303 LFO", Point::new(10, 8), SILVER.trace);
            band.label(
                ["CH 1", "CH 2", "CH 3", "CH 4"][channel.index()],
                Point::new(220, 11),
                Alignment::Right,
                SILVER.trace,
            );
            band.rect(Point::new(10, 35), Size::new(220, 2), OXIDE_RED);
        } else if let Some(&(label, value)) = rows.get(row) {
            let top = 46 + i32::from(y.saturating_sub(40) / 32) * 32;
            band.label(label, Point::new(10, top), Alignment::Left, SILVER.trace);
            band.label(value, Point::new(229, top), Alignment::Right, OXIDE_RED);
        }
        lcd.write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
            .await?;
    }
    Ok(())
}
