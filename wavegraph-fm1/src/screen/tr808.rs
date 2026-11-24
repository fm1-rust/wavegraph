//! Charcoal 808 faceplate with panel-linked dials and four illuminated rhythm switches.

use super::{
    drums::{CREAM, HOT, STYLE_808},
    mixer::Band,
    selector::Selector,
};
use crate::{panel::Channel, sequencer::DrumLamp};
use embedded_graphics::{
    pixelcolor::RgbColor,
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_graphics::Rgb565;
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Error},
    time::Clock,
};
use wavegraph_808::Controls;
use wavegraph_909::Voice;

const RIM: Rgb565 = Rgb565::new(15, 29, 13);
const RUBBER: Rgb565 = Rgb565::new(5, 10, 5);
const PANEL: Rgb565 = STYLE_808.background;

pub(super) async fn status(
    lcd: &mut AsyncDisplay<'_, Clock>,
    pixels: &mut [[u8; 2]],
    selector: &Selector,
    (channel, kit_scope, transport): (Channel, bool, &str),
) -> Result<(), Error> {
    let mut window = Band::new(pixels, 0, 0, 103, 15, selector.shade(RUBBER));
    let ink = selector.shade(CREAM);
    let left = Alignment::Left;
    window.label(
        ["CH1", "CH2", "CH3", "CH4"][channel.index()],
        Point::new(3, 3),
        left,
        ink,
    );
    window.label(
        if kit_scope { "KIT" } else { "DAC" },
        Point::new(29, 3),
        left,
        ink,
    );
    let lamp = match transport {
        "PLAY" => Rgb565::new(13, 56, 10),
        "MUTE" => HOT,
        _ => Rgb565::new(22, 10, 4),
    };
    window.rect(Point::new(58, 6), Size::new(5, 4), selector.shade(lamp));
    window.label(transport, Point::new(69, 3), left, ink);
    lcd.write_rect_buffered(8, 34, 103, 15, window.pixels.as_flattened())
        .await
}

pub(super) async fn controls(
    lcd: &mut AsyncDisplay<'_, Clock>,
    pixels: &mut [[u8; 2]],
    selector: &Selector,
    c: Controls,
    previous: Option<Controls>,
) -> Result<(), Error> {
    let paper = selector.shade(PANEL);
    let ink = selector.shade(CREAM);
    if previous.is_none() {
        for (x, title) in [(114, "BASS DRUM"), (174, "SNARE")] {
            let mut group = Band::new(pixels, 0, 0, 60, 10, paper);
            group.label(title, Point::new(30, 0), Alignment::Center, ink);
            lcd.write_rect_buffered(x, 0, 60, 10, group.pixels.as_flattened())
                .await?;
        }
    }
    let knobs = |c: Controls| [c.kick_tone, c.kick_decay, c.snare_tone, c.snappy];
    let old = previous.map(knobs);
    for (i, (name, value)) in ["TONE", "DECAY", "TONE", "SNAP"]
        .into_iter()
        .zip(knobs(c))
        .enumerate()
    {
        if old.is_some_and(|v| v[i] == value) {
            continue;
        }
        let mut dial = Band::new(pixels, 0, 0, 30, 41, paper);
        dial.knob(
            Point::new(15, 10),
            value,
            127,
            [selector.shade(RUBBER), selector.shade(RIM), ink],
        );
        dial.label(name, Point::new(15, 21), Alignment::Center, ink);
        let digits = [
            b'0' + value / 100,
            b'0' + value / 10 % 10,
            b'0' + value % 10,
        ];
        dial.label(
            core::str::from_utf8(&digits).expect("ASCII digits"),
            Point::new(15, 31),
            Alignment::Center,
            selector.shade(HOT),
        );
        lcd.write_rect_buffered(114 + 30 * i as u16, 10, 30, 41, dial.pixels.as_flattened())
            .await?;
    }
    Ok(())
}

// An unlit switch retains the pigment of its molded cap. A strike lights the
// lens and depresses the cap; the tail holds the lamp at an intermediate level.
const SWITCHES: [(Voice, &str, Rgb565); 4] = [
    (Voice::Kick, "BASS DRUM", Rgb565::new(26, 9, 5)),
    (Voice::Snare, "SNARE", Rgb565::new(29, 23, 3)),
    (Voice::ClosedHat, "CLOSED HH", Rgb565::new(30, 37, 3)),
    (Voice::OpenHat, "OPEN HH", Rgb565::new(30, 51, 8)),
];

pub(super) async fn switches(
    lcd: &mut AsyncDisplay<'_, Clock>,
    pixels: &mut [[u8; 2]],
    selector: &Selector,
    lamps: [DrumLamp; 6],
    previous: Option<[DrumLamp; 6]>,
) -> Result<(), Error> {
    for (i, (voice, name, color)) in SWITCHES.into_iter().enumerate() {
        let x = 3 + 60 * i as u16;
        if previous.is_none() {
            let mut legend = Band::new(pixels, 0, 0, 54, 12, selector.shade(PANEL));
            legend.label(
                name,
                Point::new(27, 0),
                Alignment::Center,
                selector.shade(CREAM),
            );
            lcd.write_rect_buffered(x, 189, 54, 12, legend.pixels.as_flattened())
                .await?;
        }
        let lamp = lamps[voice as usize];
        if previous.is_some_and(|old| old[voice as usize] == lamp) {
            continue;
        }
        let (travel, face, lens) = match lamp {
            DrumLamp::Off => (0, color, Rgb565::new(7, 4, 2)),
            DrumLamp::Tail => (0, highlight(color, 2), Rgb565::new(26, 19, 4)),
            DrumLamp::Strike => (2, highlight(color, 5), Rgb565::new(31, 59, 22)),
        };
        let mut cap = Band::new(pixels, 0, 0, 54, 33, selector.shade(PANEL));
        for (left, top, w, h, ink) in [
            (0, 0, 54, 33, RUBBER),
            (1, 1, 52, 31, Rgb565::BLACK),
            (3, 2 + travel, 48, 28 - travel as u32, shadow(color)),
            (4, 2 + travel, 46, 23, face),
            (4, 2 + travel, 46, 2, highlight(face, 3)),
            (17, 6 + travel, 20, 7, shadow(color)),
            (18, 7 + travel, 18, 5, lens),
            (7, 20 + travel, 40, 1, shadow(face)),
            (7, 21 + travel, 40, 1, highlight(face, 2)),
        ] {
            cap.rect(Point::new(left, top), Size::new(w, h), selector.shade(ink));
        }
        lcd.write_rect_buffered(x, 202, 54, 33, cap.pixels.as_flattened())
            .await?;
    }
    Ok(())
}

fn shadow(c: Rgb565) -> Rgb565 {
    Rgb565::new(c.r() / 2, c.g() / 2, c.b() / 2)
}

fn highlight(c: Rgb565, amount: u8) -> Rgb565 {
    Rgb565::new(
        (c.r() + amount).min(31),
        (c.g() + 2 * amount).min(63),
        (c.b() + amount).min(31),
    )
}
