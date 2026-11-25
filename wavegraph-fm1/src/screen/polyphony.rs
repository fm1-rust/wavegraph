//! Voice mode selector for the four-voice instruments.

use super::{BAND, ORGAN_LIME, ORGAN_STYLE, WIDTH, jp8080, microwave, mixer::Band};
use crate::{firmware::Labels, instrument::Part};
use fm1_graphics::{
    embedded_graphics::{
        prelude::{Point, Size},
        text::Alignment,
    },
    palette,
};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};
use wavegraph_organ::VoiceMode;

pub(super) async fn paint(
    lcd: &mut Display,
    clock: &mut Clock,
    pixels: &mut [[u8; 2]; BAND * WIDTH],
    labels: &Labels,
    entering: bool,
) -> Result<(), Error> {
    let (face, ink, accent) = match labels.focus {
        Part::Fm => (super::fm::FACE, super::fm::PRINT, super::fm::GREEN),
        Part::Organ => (ORGAN_STYLE.background, palette::WHITE, ORGAN_LIME),
        Part::Supersaw => (jp8080::FACE, jp8080::PRINT, jp8080::AMBER),
        _ => (microwave::FACE, microwave::PRINT, microwave::LIME),
    };
    let (selected, value) = match labels.mode {
        VoiceMode::Poly => (0, "One voice per key"),
        VoiceMode::Chord(chord) => (1, chord.name()),
        VoiceMode::Unison => (2, "Four detuned voices"),
    };
    let mut lcd = lcd.as_async(clock)?;
    for y in (0..SIZE).step_by(BAND) {
        if !entering && !(64..168).contains(&y) {
            continue;
        }
        let mut band = Band::new(pixels, 0, y, SIZE, BAND as u16, face);
        match y {
            0..48 => {
                band.label(
                    labels.focus.name(),
                    Point::new(12, 10),
                    Alignment::Left,
                    accent,
                );
                band.label(
                    ["CH 1", "CH 2", "CH 3", "CH 4"][labels.channel.index()],
                    Point::new(228, 10),
                    Alignment::Right,
                    ink,
                );
                band.title("VOICE MODE", Point::new(12, 29), ink);
            }
            64..104 => {
                for (i, label) in ["POLY", "CHORD", "UNISON"].into_iter().enumerate() {
                    let x = 12 + i32::try_from(i).expect("three modes") * 73;
                    let active = i == selected;
                    band.rect(
                        Point::new(x, 66),
                        Size::new(68, 32),
                        if active { accent } else { ink },
                    );
                    if !active {
                        band.rect(Point::new(x + 1, 67), Size::new(66, 30), face);
                    }
                    band.label(
                        label,
                        Point::new(x + 34, 77),
                        Alignment::Center,
                        if active { face } else { ink },
                    );
                }
            }
            112..168 => {
                if selected == 1 {
                    band.label(
                        "ALGORITHM / KNOB 1   CHORD TYPE",
                        Point::new(12, 117),
                        Alignment::Left,
                        ink,
                    );
                }
                band.title(value, Point::new(12, 143), accent);
            }
            192.. => {
                band.label(
                    "SELECT   Mode      SEL   Next",
                    Point::new(12, 197),
                    Alignment::Left,
                    ink,
                );
                band.label("EDIT  Back", Point::new(12, 216), Alignment::Left, ink);
            }
            _ => (),
        }
        lcd.write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
            .await?;
    }
    Ok(())
}
