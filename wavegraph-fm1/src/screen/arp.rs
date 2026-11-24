//! Per-instrument arpeggiator panel and a one-octave monitor of outgoing notes.

use super::{BAND, WIDTH, drums, jp8080, microwave, mixer::Band};
use crate::{arp::Controls, firmware::Labels, instrument::Part};
use core::fmt::Write;
use fm1_graphics::embedded_graphics::{
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_graphics::{Rgb565, palette};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};
use heapless::String;
use wavegraph_arp::{Division, Mode};

#[derive(Clone, Copy)]
pub(super) struct Colors {
    pub(super) face: Rgb565,
    ink: Rgb565,
    accent: Rgb565,
}

impl Colors {
    pub(super) fn for_part(part: Part) -> Self {
        let (face, ink, accent) = match part {
            Part::Mono => (super::STYLE.background, palette::WHITE, super::STYLE.trace),
            Part::Organ => (
                super::ORGAN_STYLE.background,
                palette::WHITE,
                super::ORGAN_LIME,
            ),
            Part::Bass => (
                super::SILVER.background,
                super::SILVER.trace,
                super::OXIDE_RED,
            ),
            Part::Tr808 => (drums::STYLE_808.background, drums::CREAM, drums::HOT),
            Part::Tr909 => (drums::STYLE.background, drums::GRAPHITE, drums::ORANGE),
            Part::Supersaw => (jp8080::FACE, jp8080::PRINT, jp8080::AMBER),
            Part::Wavetable => (microwave::FACE, microwave::PRINT, microwave::LIME),
        };
        Self { face, ink, accent }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct State {
    controls: Controls,
    bpm: u16,
    note: Option<u8>,
}

#[derive(Default)]
pub(super) struct Panel {
    previous: Option<State>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]; BAND * WIDTH],
        labels: &Labels,
        note: Option<u8>,
        entering: bool,
    ) -> Result<(), Error> {
        let state = State {
            controls: labels.arp,
            bpm: labels.seq.transport.bpm,
            note,
        };
        let previous = self.previous.filter(|_| !entering);
        if previous == Some(state) {
            return Ok(());
        }
        let colors = Colors::for_part(labels.focus);
        let c = state.controls.config;
        let mode = match c.mode {
            Mode::Up => "Up",
            Mode::Down => "Down",
            Mode::UpDown => "Up / down",
            Mode::AsPlayed => "As played",
            Mode::Random => "Random",
        };
        let rate = match c.division {
            Division::Quarter => "1/4",
            Division::Eighth => "1/8",
            Division::EighthTriplet => "1/8 T",
            Division::Sixteenth => "1/16",
            Division::SixteenthTriplet => "1/16 T",
            Division::ThirtySecond => "1/32",
        };
        let mut gate = String::<5>::new();
        write!(gate, "{}%", c.gate).expect("gate percent");
        let mut tempo = String::<8>::new();
        write!(tempo, "{} BPM", state.bpm).expect("three-digit tempo");
        let mut lcd = lcd.as_async(clock)?;
        for y in (0..SIZE).step_by(BAND) {
            let dirty = previous.is_none_or(|old| match y {
                0..32 => false,
                32..48 => state.bpm != old.bpm,
                48..136 => c != old.controls.config,
                136..200 => note != old.note,
                _ => state.controls != old.controls,
            });
            if !dirty {
                continue;
            }
            let mut band = Band::new(pixels, 0, y, SIZE, BAND as u16, colors.face)
                .with_panel_type(super::lettering::panel_type(labels.focus));
            match y {
                0..48 => {
                    band.title("ARPEGGIATOR", Point::new(10, 7), colors.ink);
                    band.label(
                        ["CH 1", "CH 2", "CH 3", "CH 4"][labels.channel.index()],
                        Point::new(220, 9),
                        Alignment::Right,
                        colors.ink,
                    );
                    band.label(
                        labels.focus.name(),
                        Point::new(10, 31),
                        Alignment::Left,
                        colors.accent,
                    );
                    band.label(&tempo, Point::new(229, 32), Alignment::Right, colors.ink);
                    band.rect(Point::new(10, 45), Size::new(220, 1), colors.accent);
                }
                48..136 => {
                    for (x, top, label, value) in [
                        (10, 53, "1  ORDER", mode),
                        (128, 53, "2  RATE", rate),
                        (
                            10,
                            96,
                            "3  OCTAVES",
                            ["1", "2", "3", "4"][usize::from(c.octaves - 1)],
                        ),
                        (128, 96, "4  GATE", gate.as_str()),
                    ] {
                        band.label(label, Point::new(x, top), Alignment::Left, colors.accent);
                        band.title(value, Point::new(x, top + 14), colors.ink);
                    }
                }
                136..200 => {
                    band.label("MIDI OUT", Point::new(10, 136), Alignment::Left, colors.ink);
                    keyboard(&mut band, note, colors);
                }
                _ => {
                    band.rect(Point::new(10, 201), Size::new(220, 1), colors.accent);
                    let on = state.controls.enabled;
                    band.label(
                        "SEL / ARP",
                        Point::new(10, 206),
                        Alignment::Left,
                        colors.ink,
                    );
                    band.title(
                        if on { "ON" } else { "OFF" },
                        Point::new(10, 219),
                        if on { colors.accent } else { colors.ink },
                    );
                    band.label(
                        "SELECT / LATCH",
                        Point::new(128, 206),
                        Alignment::Left,
                        colors.ink,
                    );
                    band.title(
                        if c.latch { "HOLD" } else { "OFF" },
                        Point::new(128, 219),
                        colors.accent,
                    );
                }
            }
            lcd.write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
                .await?;
        }
        self.previous = Some(state);
        Ok(())
    }
}

fn keyboard(band: &mut Band<'_>, note: Option<u8>, colors: Colors) {
    let pitch = note.map(|n| n % 12);
    for (i, key) in (0..7).zip([0, 2, 4, 5, 7, 9, 11]) {
        let fill = if pitch == Some(key) {
            colors.accent
        } else {
            palette::WHITE
        };
        band.rect(Point::new(11 + 31 * i, 151), Size::new(30, 41), fill);
    }
    for (i, key) in [(1, 1), (2, 3), (4, 6), (5, 8), (6, 10)] {
        let fill = if pitch == Some(key) {
            colors.accent
        } else {
            Rgb565::new(1, 2, 1)
        };
        band.rect(Point::new(2 + 31 * i, 151), Size::new(18, 25), fill);
    }
    if let Some(n) = note {
        let mut label = String::<6>::new();
        let name = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ][usize::from(n % 12)];
        write!(label, "{}{}", name, i16::from(n / 12) - 1).expect("MIDI note label");
        band.label(
            &label,
            Point::new(229, 136),
            Alignment::Right,
            colors.accent,
        );
    }
}
