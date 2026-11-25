//! Per-instrument arpeggiator panel and a one-octave monitor of outgoing notes.

use super::{BAND, InstrumentStyle, WIDTH, mixer::Band};
use crate::{arp::Controls, firmware::Labels};
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
        (colors, entering): (&InstrumentStyle, bool),
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
                        labels.channel.label(),
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
                        if on { colors.selected.fill } else { colors.ink },
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
                        if c.latch {
                            colors.selected.fill
                        } else {
                            colors.ink
                        },
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

fn keyboard(band: &mut Band<'_>, note: Option<u8>, colors: &InstrumentStyle) {
    let pitch = note.map(|n| n % 12);
    let mut keycap = |key, x, width, height, idle| {
        let down = pitch == Some(key);
        let fill = if down { colors.selected.fill } else { idle };
        band.rect(Point::new(x, 151), Size::new(width, height), fill);
        // The inset stripe stays visible on a white or black key under every palette.
        if down {
            band.rect(
                Point::new(x + 4, 151 + height as i32 - 7),
                Size::new(width - 8, 3),
                colors.selected.ink,
            );
        }
    };
    for (i, key) in (0..7).zip([0, 2, 4, 5, 7, 9, 11]) {
        keycap(key, 11 + 31 * i, 30, 41, palette::WHITE);
    }
    for (i, key) in [(1, 1), (2, 3), (4, 6), (5, 8), (6, 10)] {
        keycap(key, 2 + 31 * i, 18, 25, Rgb565::new(1, 2, 1));
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
            colors.selected.fill,
        );
    }
}
