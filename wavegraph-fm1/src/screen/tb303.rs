//! Six fixed sound knobs below the scope and two auxiliary knobs beside the wordmark.

use super::{OXIDE_RED, mixer::Band, selector::Selector};
use crate::assets::fonts::FONT_6X10;
use crate::{
    bass::{Bank, Controls, Knob, cutoff_detent},
    firmware::Labels,
    instrument::Voice,
    panel::Channel,
    sequencer::pattern::Automation,
};
use core::{fmt::Write, str};
use embedded_graphics::{
    Drawable,
    framebuffer::Framebuffer,
    mono_font::MonoTextStyle,
    pixelcolor::{
        BinaryColor,
        raw::{BigEndian, RawU1},
    },
    prelude::{Point, Primitive, Size},
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
    text::{Alignment, Baseline, Text},
};
use fm1_graphics::Rgb565;
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Error},
    time::Clock,
};
use heapless::String;

pub(super) const MARGIN: u16 = 32;
pub(super) const METAL: Rgb565 = Rgb565::new(27, 54, 27);
const GRAPHITE: Rgb565 = Rgb565::new(4, 9, 5);
const CAP: Rgb565 = Rgb565::new(8, 17, 9);
const SHADOW: Rgb565 = Rgb565::new(14, 28, 14);
const IVORY: Rgb565 = Rgb565::new(30, 60, 29);
const LAMP: Rgb565 = Rgb565::new(31, 27, 12);

const KNOBS: [(Knob, &str); 8] = [
    (Knob::Tuning, "TUNING"),
    (Knob::Cutoff, "CUTOFF"),
    (Knob::Resonance, "RESO"),
    (Knob::EnvMod, "ENVMOD"),
    (Knob::Decay, "DECAY"),
    (Knob::Accent, "ACCENT"),
    (Knob::Attack, "ATTACK"),
    (Knob::Overdrive, "DRIVE"),
];

#[derive(Clone, Copy, Eq, PartialEq)]
struct Status {
    channel: Channel,
    voice: Voice,
    voice_tap: bool,
    half_gain: bool,
    transport: &'static str,
}

impl Status {
    fn draw(self, band: &mut Band<'_>, ink: Rgb565) {
        let mut label = |x, s: &str| band.label(s, Point::new(x, 0), Alignment::Left, ink);
        label(3, ["CH1", "CH2", "CH3", "CH4"][self.channel.index()]);
        label(
            27,
            if self.voice.waveform == 0 {
                "SAW"
            } else {
                "SQR"
            },
        );
        label(51, if self.voice_tap { "SRC" } else { "DAC" });
        label(
            75,
            if self.voice_tap {
                "AUTO"
            } else if self.half_gain {
                "HALF"
            } else {
                "FULL"
            },
        );
        if let Some(note) = self.voice.gate.note {
            let mut key = String::<12>::new();
            write!(key, "{} V{}", ascii(&self.voice.note_name()), note.velocity)
                .expect("MIDI note and velocity fit 12 bytes");
            label(104, &key);
        } else {
            label(104, self.transport);
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Snapshot {
    controls: Controls,
    status: Status,
    automation: Option<Automation>,
}

#[derive(Eq, PartialEq)]
struct Dial {
    pointer: Point,
    encoder: Option<usize>,
    selected: bool,
    marker: Option<Point>,
}

impl Snapshot {
    fn dial(self, knob: Knob) -> Dial {
        let (min, max) = knob.limits();
        let marker = self.automation.and_then(|a| {
            let cc = crate::bass::CONTROLLERS[knob as usize];
            let midi = a.get(cc)?;
            let value = if knob == Knob::Cutoff {
                let wavegraph_303::Message::CutoffHz(hz) = crate::bass::midi_parameter(cc, midi)?
                else {
                    return None;
                };
                cutoff_detent(hz as u16)
            } else {
                ((u16::from(midi) * u16::from(max - min) + 63) / 127) as u8
            };
            let tip = Band::knob_offset(value, max - min);
            Some(Point::new(tip.x * 13 / 7 - 1, tip.y * 11 / 7))
        });
        Dial {
            pointer: Band::knob_offset(
                self.controls.detent(knob).saturating_sub(min),
                max.saturating_sub(min),
            ),
            encoder: self.controls.bank.knobs().iter().position(|&k| k == knob),
            selected: self.controls.last() == knob,
            marker,
        }
    }
}

impl Dial {
    fn draw(&self, band: &mut Band<'_>, selector: &Selector, name: &str, center: Point) {
        let shade = |c| selector.shade(c);
        let ink = if self.selected { OXIDE_RED } else { GRAPHITE };
        // The skirt casts a shadow below the cap. Its upper-left bevel catches the light.
        for (offset, diameter, color) in [
            (Point::new(1, 1), 22, SHADOW),
            (Point::zero(), 21, GRAPHITE),
            (Point::new(-1, -1), 17, SHADOW),
            (Point::zero(), 16, CAP),
        ] {
            Circle::with_center(center + offset, diameter)
                .into_styled(PrimitiveStyle::with_fill(shade(color)))
                .draw(band)
                .expect("infallible strip");
        }
        for (dx, dy) in [(-13, 3), (-11, -6), (0, -11), (11, -6), (13, 3)] {
            band.rect(center + Point::new(dx, dy), Size::new(1, 2), shade(SHADOW));
        }
        let pointer = if self.selected { LAMP } else { IVORY };
        Line::new(center + self.pointer / 3, center + self.pointer)
            .into_styled(PrimitiveStyle::with_stroke(shade(pointer), 2))
            .draw(band)
            .expect("infallible strip");
        if let Some(index) = self.encoder {
            band.label(
                ["1", "2", "3", "4"][index],
                Point::new(0, 4),
                Alignment::Left,
                shade(ink),
            );
        }
        // The cap shows the stored setting. The red witness mark follows the sequencer.
        if let Some(marker) = self.marker {
            band.rect(center + marker, Size::new(3, 3), shade(OXIDE_RED));
        }
        band.label(
            name,
            Point::new(center.x, 22),
            Alignment::Center,
            shade(ink),
        );
    }
}

#[derive(Default)]
pub(super) struct Panel {
    previous: Option<Snapshot>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        pixels: &mut [[u8; 2]],
        selector: &mut Selector,
        labels: &Labels,
        automation: Option<Automation>,
        entering: bool,
    ) -> Result<(), Error> {
        let state = Snapshot {
            controls: labels.bass,
            status: Status {
                channel: labels.channel,
                voice: labels.voice,
                voice_tap: labels.voice_tap,
                half_gain: labels.half_gain,
                transport: super::transport_status(labels),
            },
            automation,
        };
        let old = self.previous.filter(|_| !entering);
        if old == Some(state) {
            return Ok(());
        }
        let paper = selector.shade(METAL);
        if entering {
            let mut logo = Band::new(pixels, 0, 0, 78, 20, paper);
            wordmark(&mut logo, selector);
            logo.compose_badge(&mut selector.badge);
            lcd.write_rect_buffered(0, 0, 78, 20, logo.pixels.as_flattened())
                .await?;
        }
        if old.is_none_or(|s| s.controls != state.controls) {
            let mut display = Band::new(pixels, 0, 0, 80, 20, selector.shade(GRAPHITE));
            readout(&mut display, state.controls, selector);
            lcd.write_rect_buffered(78, 0, 80, 20, display.pixels.as_flattened())
                .await?;
        }
        if old.is_none_or(|s| s.status != state.status) {
            let mut status = Band::new(pixels, 0, 0, 158, 10, paper);
            state.status.draw(&mut status, selector.shade(GRAPHITE));
            selector.badge.compose(0, 21, 158, status.pixels);
            lcd.write_rect_buffered(0, 21, 158, 10, status.pixels.as_flattened())
                .await?;
        }
        for (i, (knob, name)) in KNOBS.into_iter().enumerate() {
            let dial = state.dial(knob);
            let previous = old.map(|s| s.dial(knob));
            if previous.as_ref() == Some(&dial) {
                continue;
            }
            let slot = u16::try_from(i).expect("eight knobs");
            let (x, y, width): (u16, u16, u16) = if slot < 6 {
                (40 * slot, 209, 40)
            } else {
                (158 + 36 * (slot - 6), 0, 36)
            };
            let center = Point::new(i32::from(width / 2), 11);
            // A moving sequencer marker only damages its old and new footprints.
            // Draw the final dial clipped to that area to restore the underlying bevel and ticks.
            let area = if old.is_some_and(|s| s.controls == state.controls)
                && let (Some(a), Some(b)) = (previous.and_then(|d| d.marker), dial.marker)
            {
                Rectangle::with_corners(
                    center + a.component_min(b),
                    center + a.component_max(b) + Point::new(2, 2),
                )
            } else {
                Rectangle::new(Point::zero(), Size::new(u32::from(width), 31))
            };
            let dx = area.top_left.x as u16;
            let dy = area.top_left.y as u16;
            let w = area.size.width as u16;
            let h = area.size.height as u16;
            let mut cell = Band::new(pixels, dx, dy, w, h, paper);
            dial.draw(&mut cell, selector, name, center);
            lcd.write_rect_buffered(x + dx, y + dy, w, h, cell.pixels.as_flattened())
                .await?;
        }
        lcd.flush().await?;
        self.previous = Some(state);
        Ok(())
    }
}

fn ascii(bytes: &[u8]) -> &str {
    str::from_utf8(bytes.trim_ascii_end()).expect("ASCII panel text")
}

fn readout(band: &mut Band<'_>, controls: Controls, selector: &Selector) {
    let (name, value, units) = controls.readout();
    let bank = match controls.bank {
        Bank::Filter => "1/2",
        Bank::Voice => "2/2",
    };
    band.label(
        bank,
        Point::new(2, 0),
        Alignment::Left,
        selector.shade(IVORY),
    );
    band.label(
        ascii(name),
        Point::new(77, 0),
        Alignment::Right,
        selector.shade(IVORY),
    );
    let mut number = String::<16>::new();
    if controls.last() == Knob::Tuning {
        write!(number, "{value:+}")
    } else {
        write!(number, "{value}")
    }
    .expect("panel value fits 16 bytes");
    write!(number, " {}", ascii(units)).expect("panel value and units fit 16 bytes");
    band.label(
        &number,
        Point::new(77, 10),
        Alignment::Right,
        selector.shade(LAMP),
    );
}

fn wordmark(band: &mut Band<'_>, selector: &Selector) {
    let mut mask = Framebuffer::<BinaryColor, RawU1, BigEndian, 40, 10, 50>::new();
    Text::with_baseline(
        "TB-303",
        Point::zero(),
        MonoTextStyle::new(&FONT_6X10, BinaryColor::On),
        Baseline::Top,
    )
    .draw(&mut mask)
    .expect("infallible mask");
    let [model, digits] = [GRAPHITE, OXIDE_RED].map(|c| selector.shade(c));
    for (y, row) in (0..10).zip(mask.data().chunks_exact(5)) {
        for x in 0..36 {
            if row[x / 8] & (0x80 >> (x % 8)) != 0 {
                let slant = (9 - y) / 3;
                let left = 2 + slant + 2 * i32::try_from(x).expect("36 columns");
                band.rect(
                    Point::new(left, y * 2),
                    Size::new(2, 2),
                    if x < 18 { model } else { digits },
                );
            }
        }
    }
}
