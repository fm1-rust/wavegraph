//! Channel modulation panel. Only rows whose controls changed are transferred.
use super::{BAND, InstrumentStyle, WIDTH, mixer::Band};
use crate::{firmware::Labels, lfo::Controls, panel::Channel};
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
use wavegraph_lfo::Shape;

pub(super) async fn paint(
    lcd: &mut Display,
    clock: &mut Clock,
    pixels: &mut [[u8; 2]; BAND * WIDTH],
    labels: &Labels,
    theme: &InstrumentStyle,
    previous: Option<(Channel, Controls)>,
) -> Result<(), Error> {
    let c = labels.lfo;
    let shape = match c.shape() {
        Shape::Sine => "SINE",
        Shape::Triangle => "TRIANGLE",
        Shape::Saw => "SAW",
        Shape::Square => "SQUARE",
    };
    let target = c.target(labels.focus).name;
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
        let (x, width) = if let Some((channel, p)) = previous {
            if y < 40 {
                if channel == labels.channel || !(8..24).contains(&y) {
                    continue;
                }
                (196, 25) // The four-character channel label ends at x220.
            } else {
                let unchanged = match row {
                    0 => p.shape() == c.shape(),
                    1 => {
                        p.sync == c.sync
                            && p.tenths_hz() == c.tenths_hz()
                            && p.division_name() == c.division_name()
                    }
                    2 => p.depth == c.depth,
                    3 => p.target(labels.focus).route == c.target(labels.focus).route,
                    4 => p.sync == c.sync,
                    5 => p.enabled == c.enabled,
                    _ => true,
                };
                let row_band = y.saturating_sub(40) % 32;
                if unchanged || row_band >= 16 {
                    continue;
                }
                (110, 120) // Only the right-aligned value changes on a control row.
            }
        } else {
            (0, SIZE)
        };
        let mut band = Band::new(pixels, x, y, width, BAND as u16, theme.face)
            .with_panel_type(super::lettering::panel_type(labels.focus));
        if y < 40 {
            let title = heapless::format!(20; "{} LFO", labels.focus.name()).expect("short title");
            band.title(&title, Point::new(10, 8), theme.ink);
            band.label(
                labels.channel.label(),
                Point::new(220, 11),
                Alignment::Right,
                theme.ink,
            );
            band.rect(Point::new(10, 35), Size::new(220, 2), theme.accent);
        } else if let Some(&(label, value)) = rows.get(row) {
            let top = 46 + i32::from(y.saturating_sub(40) / 32) * 32;
            band.label(label, Point::new(10, top), Alignment::Left, theme.ink);
            band.label(
                value,
                Point::new(229, top),
                Alignment::Right,
                theme.selected.fill,
            );
        }
        lcd.write_rect_buffered(x, y, width, BAND as u16, band.pixels.as_flattened())
            .await?;
    }
    Ok(())
}
