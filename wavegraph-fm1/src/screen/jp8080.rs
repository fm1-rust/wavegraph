//! JP-8080 rack panel above and below the channel's full-width oscilloscope.

use super::{BAND, mixer::Band, selector::Selector, style::JP8080};
use crate::{
    envelope::EnvelopeTarget, firmware::Labels, instrument::MAX_CUTOFF_STEP, panel::Channel,
};
use core::fmt::Write;
use fm1_graphics::Rgb565;
use fm1_graphics::embedded_graphics::{
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};

pub(super) const FACE: Rgb565 = JP8080.face;
pub(super) const EDGE: Rgb565 = JP8080.muted;
pub(super) const AMBER: Rgb565 = JP8080.accent;
pub(super) const PRINT: Rgb565 = JP8080.ink;
const BLACK: Rgb565 = Rgb565::new(1, 3, 4);
const RIM: Rgb565 = JP8080.surface;
const LCD: Rgb565 = Rgb565::new(12, 43, 24);

#[derive(Default)]
pub(super) struct Panel {
    status: Option<(Channel, bool, [u8; 4])>,
    controls: Option<[u8; 4]>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]],
        selector: &mut Selector,
        labels: Labels,
        entering: bool,
    ) -> Result<(), Error> {
        let ink = selector.shader();
        let mut lcd = lcd.as_async(clock)?;
        if entering {
            for y in (0..32).step_by(BAND) {
                let mut band = Band::new(pixels, 0, y, SIZE, 8, ink(FACE)).with_panel_type(true);
                band.label("SUPER SAW", Point::new(9, 18), Alignment::Left, ink(PRINT));
                // Use the regular font for case-sensitive chord symbols.
                band = band.with_panel_type(false);
                band.label(
                    crate::panel::Polyphony::label(labels.mode),
                    Point::new(92, 18),
                    Alignment::Center,
                    ink(AMBER),
                );
                band.rect(Point::new(0, 30), Size::new(240, 1), ink(EDGE));
                band.rect(Point::new(0, 31), Size::new(240, 1), ink(BLACK));
                lcd.write_rect_buffered(0, y, SIZE, 8, band.pixels.as_flattened())
                    .await?;
            }
            wordmark(pixels, ink(FACE), ink(AMBER));
            lcd.write_rect_buffered(7, 2, 100, 16, pixels[..1600].as_flattened())
                .await?;
        }

        let mut key = labels.voice.note_name();
        if labels.voice.gate.note.is_none() && labels.seq.transport.playing() {
            key = if labels.seq.enabled[labels.channel.index()] {
                *b"SEQ "
            } else {
                *b"MUTE"
            };
        }
        let status = (labels.channel, labels.voice_tap, key);
        if entering || self.status != Some(status) {
            let mut display = Band::new(pixels, 0, 0, 102, 18, ink(BLACK)).with_panel_type(true);
            display.rect(Point::new(1, 1), Size::new(100, 16), ink(RIM));
            display.rect(Point::new(3, 3), Size::new(96, 12), ink(LCD));
            let left = Alignment::Left;
            display.label(
                ["CH1", "CH2", "CH3", "CH4"][labels.channel.index()],
                Point::new(7, 4),
                left,
                ink(BLACK),
            );
            display.label(
                if labels.voice_tap { "OSC" } else { "DAC" },
                Point::new(40, 4),
                left,
                ink(BLACK),
            );
            display.label(
                core::str::from_utf8(&key).expect("ASCII note name"),
                Point::new(70, 4),
                left,
                ink(BLACK),
            );
            lcd.write_rect_buffered(122, 7, 102, 18, display.pixels.as_flattened())
                .await?;
            self.status = Some(status);
        }

        let values = [
            labels.voice.cutoff_step,
            labels.saw.resonance,
            labels.saw.detune,
            labels.saw.mix,
        ];
        for (i, (x, name)) in [
            (0, "1 CUTOFF"),
            (60, "2 RESO"),
            (120, "3 DETUNE"),
            (180, "4 MIX"),
        ]
        .into_iter()
        .enumerate()
        {
            let value = values[i];
            if !entering && self.controls.is_some_and(|old| old[i] == value) {
                continue;
            }
            let left = i32::from(x);
            let mut readout = heapless::String::<6>::new();
            let max = if i == 0 {
                let hz = labels.voice.cutoff_hz();
                if hz < 1000 {
                    write!(readout, "{hz}Hz")
                } else {
                    write!(readout, "{}.{}k", hz / 1000, hz / 100 % 10)
                }
                .expect("five characters");
                MAX_CUTOFF_STEP
            } else {
                write!(readout, "{value}").expect("three digits");
                127
            };
            for y in [200, 220] {
                let mut band = Band::new(pixels, x, y, 60, 20, ink(FACE)).with_panel_type(true);
                band.rect(Point::new(left, 200), Size::new(60, 1), ink(BLACK));
                band.rect(Point::new(left, 201), Size::new(60, 1), ink(EDGE));
                band.rect(Point::new(left + 59, 206), Size::new(1, 29), ink(RIM));
                band.label(
                    name,
                    Point::new(left + 30, 204),
                    Alignment::Center,
                    ink(PRINT),
                );
                band.knob(
                    Point::new(left + 14, 226),
                    value,
                    max,
                    [ink(BLACK), ink(RIM), ink(AMBER)],
                );
                band.label(
                    readout.as_str(),
                    Point::new(left + 41, 222),
                    Alignment::Center,
                    ink(AMBER),
                );
                lcd.write_rect_buffered(x, y, 60, 20, band.pixels.as_flattened())
                    .await?;
            }
        }
        self.controls = Some(values);
        lcd.flush().await
    }
}

pub(super) async fn envelope_header(
    lcd: &mut Display,
    clock: &mut Clock,
    pixels: &mut [[u8; 2]],
    target: EnvelopeTarget,
    entering: bool,
) -> Result<(), Error> {
    let mut lcd = lcd.as_async(clock)?;
    if entering {
        wordmark(pixels, FACE, AMBER);
        lcd.write_rect_buffered(7, 2, 100, 16, pixels[..1600].as_flattened())
            .await?;
        let mut band = Band::new(pixels, 0, 18, 108, 14, FACE).with_panel_type(true);
        band.label("ENVELOPES", Point::new(9, 19), Alignment::Left, PRINT);
        lcd.write_rect_buffered(0, 18, 108, 14, band.pixels.as_flattened())
            .await?;
    }
    for y in [0, 16] {
        let mut band = Band::new(pixels, 110, y, 114, 16, FACE).with_panel_type(true);
        band.label("SELECT", Point::new(113, 0), Alignment::Left, PRINT);
        for (x, width, name, page) in [
            (112, 40, "AMP", EnvelopeTarget::Amp),
            (155, 66, "FILTER", EnvelopeTarget::Modulation),
        ] {
            let active = target == page;
            band.rect(
                Point::new(x, 11),
                Size::new(width, 17),
                if active { JP8080.selected.fill } else { RIM },
            );
            if !active {
                band.rect(Point::new(x + 1, 12), Size::new(width - 2, 15), FACE);
            }
            band.label(
                name,
                Point::new(x + (width / 2) as i32, 15),
                Alignment::Center,
                if active { JP8080.selected.ink } else { PRINT },
            );
        }
        lcd.write_rect_buffered(110, y, 114, 16, band.pixels.as_flattened())
            .await?;
    }
    lcd.flush().await
}

fn wordmark(pixels: &mut [[u8; 2]], face: Rgb565, amber: Rgb565) {
    Band::new(pixels, 0, 0, 100, 16, face).mask(Point::zero(), super::lettering::JP, 0, [amber; 2]);
}
