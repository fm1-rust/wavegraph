//! Six fixed sound knobs above the scope and two auxiliary knobs beside the wordmark.

use super::{OXIDE_RED, mixer::Band, selector::Selector};
use crate::{
    bass::{Controls, Knob, cutoff_detent},
    firmware::Labels,
    instrument::Voice,
    panel::Channel,
    sequencer::pattern::Automation,
};
use core::{fmt::Write, str};
use embedded_graphics::{
    Drawable,
    prelude::{Point, Primitive, RgbColor, Size},
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
    text::Alignment,
};
use fm1_graphics::Rgb565;
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Error},
    time::Clock,
};
use heapless::String;

const KNOB_ROW: u16 = 32;
pub(super) const SCOPE_TOP: u16 = KNOB_ROW + CELL_HEIGHT + 1;
pub(super) const METAL: Rgb565 = Rgb565::new(27, 54, 27);
const GRAPHITE: Rgb565 = Rgb565::new(4, 9, 5);
const CAP: Rgb565 = Rgb565::new(8, 17, 9);
const DIRECT_CAP: Rgb565 = Rgb565::new(8, 24, 15);
const FRAME: Rgb565 = Rgb565::new(9, 27, 16);
const NEON: Rgb565 = Rgb565::new(8, 63, 4);
const SHADOW: Rgb565 = Rgb565::new(14, 28, 14);
const IVORY: Rgb565 = Rgb565::new(30, 60, 29);
const LAMP: Rgb565 = Rgb565::new(31, 27, 12);

const CELL_WIDTH: u16 = 40;
const CELL_HEIGHT: u16 = 31;

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
    selected: Option<Knob>,
    status: Status,
    automation: Option<Automation>,
}

#[derive(Eq, PartialEq)]
struct Dial {
    pointer: Point,
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
            selected: self.selected == Some(knob),
            marker,
        }
    }
}

impl Dial {
    fn draw(&self, band: &mut Band<'_>, selector: &Selector, knob: Knob, center: Point) {
        let shade = |c| selector.shade(c);
        let encoder = Knob::direct().iter().position(|&k| k == knob);
        let (skirt, bevel, cap, pointer) = if self.selected {
            (Rgb565::BLACK, NEON, NEON, Rgb565::BLACK)
        } else {
            let cap = if encoder.is_some() { DIRECT_CAP } else { CAP };
            (GRAPHITE, SHADOW, cap, IVORY)
        };
        // The skirt casts a shadow below the cap. Its upper-left bevel catches the light.
        for (offset, diameter, color) in [
            (Point::new(1, 1), 22, SHADOW),
            (Point::zero(), 21, skirt),
            (Point::new(-1, -1), 17, bevel),
            (Point::zero(), 16, cap),
        ] {
            Circle::with_center(center + offset, diameter)
                .into_styled(PrimitiveStyle::with_fill(shade(color)))
                .draw(band)
                .expect("infallible strip");
        }
        for (dx, dy) in [(-13, 3), (-11, -6), (0, -11), (11, -6), (13, 3)] {
            band.rect(center + Point::new(dx, dy), Size::new(1, 2), shade(SHADOW));
        }
        Line::new(center + self.pointer / 3, center + self.pointer)
            .into_styled(PrimitiveStyle::with_stroke(shade(pointer), 2))
            .draw(band)
            .expect("infallible strip");
        if let Some(index) = encoder {
            // Each clipped dial cell paints its part of one continuous frame.
            let x = 1 - i32::try_from(index).expect("four encoders") * i32::from(CELL_WIDTH);
            Rectangle::new(
                Point::new(x, 0),
                Size::new(u32::from(CELL_WIDTH) * 4 - 2, u32::from(CELL_HEIGHT)),
            )
            .into_styled(PrimitiveStyle::with_stroke(shade(FRAME), 1))
            .draw(band)
            .expect("infallible strip");
            band.label(
                ["1", "2", "3", "4"][index],
                Point::new(3, 4),
                Alignment::Left,
                shade(GRAPHITE),
            );
        }
        // The cap shows the stored setting. The red witness mark follows the sequencer.
        if let Some(marker) = self.marker {
            band.rect(center + marker, Size::new(3, 3), shade(OXIDE_RED));
        }
        band.label(
            match knob {
                Knob::Resonance => "RESO",
                Knob::Overdrive => "DRIVE",
                _ => ascii(knob.name()),
            },
            Point::new(center.x, 21),
            Alignment::Center,
            shade(GRAPHITE),
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
            selected: labels.bass_knob,
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
            let mut logo = Band::new(pixels, 0, 0, 78, 20, paper).with_panel_type(true);
            wordmark(&mut logo, selector);
            logo.compose_badge(&mut selector.badge);
            lcd.write_rect_buffered(0, 0, 78, 20, logo.pixels.as_flattened())
                .await?;
        }
        if old.is_none_or(|s| s.controls != state.controls || s.selected != state.selected) {
            let mut display =
                Band::new(pixels, 0, 0, 80, 20, selector.shade(GRAPHITE)).with_panel_type(true);
            readout(&mut display, state.controls, state.selected, selector);
            lcd.write_rect_buffered(78, 0, 80, 20, display.pixels.as_flattened())
                .await?;
        }
        if old.is_none_or(|s| s.status != state.status) {
            let mut status = Band::new(pixels, 0, 0, 158, 10, paper).with_panel_type(true);
            state.status.draw(&mut status, selector.shade(GRAPHITE));
            selector.badge.compose(0, 21, 158, status.pixels);
            lcd.write_rect_buffered(0, 21, 158, 10, status.pixels.as_flattened())
                .await?;
        }
        for (i, knob) in Knob::PANEL.into_iter().enumerate() {
            let dial = state.dial(knob);
            let previous = old.map(|s| s.dial(knob));
            if previous.as_ref() == Some(&dial) {
                continue;
            }
            let slot = u16::try_from(i).expect("eight knobs");
            let (x, y, width): (u16, u16, u16) = if slot < 6 {
                (CELL_WIDTH * slot, KNOB_ROW, CELL_WIDTH)
            } else {
                (158 + 36 * (slot - 6), 0, 36)
            };
            let center = Point::new(i32::from(width / 2), 11);
            // A moving sequencer marker only damages its old and new footprints.
            // Draw the final dial clipped to that area to restore the underlying bevel and ticks.
            let area = if let Some(previous) = previous
                && previous.pointer == dial.pointer
                && previous.selected == dial.selected
                && let (Some(a), Some(b)) = (previous.marker, dial.marker)
            {
                Rectangle::with_corners(
                    center + a.component_min(b),
                    center + a.component_max(b) + Point::new(2, 2),
                )
            } else {
                Rectangle::new(
                    Point::zero(),
                    Size::new(u32::from(width), u32::from(CELL_HEIGHT)),
                )
            };
            let dx = area.top_left.x as u16;
            let dy = area.top_left.y as u16;
            let w = area.size.width as u16;
            let h = area.size.height as u16;
            let mut cell = Band::new(pixels, dx, dy, w, h, paper).with_panel_type(true);
            dial.draw(&mut cell, selector, knob, center);
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

fn readout(band: &mut Band<'_>, controls: Controls, selected: Option<Knob>, selector: &Selector) {
    let knob = selected.unwrap_or(controls.last);
    let (name, value, units) = controls.readout(knob);
    if selected.is_some() {
        band.label(
            "ALG",
            Point::new(2, 0),
            Alignment::Left,
            selector.shade(NEON),
        );
    }
    band.label(
        ascii(name),
        Point::new(77, 0),
        Alignment::Right,
        selector.shade(IVORY),
    );
    let mut number = String::<16>::new();
    if knob == Knob::Tuning {
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
    band.mask(
        Point::zero(),
        super::lettering::BASS,
        36,
        [GRAPHITE, OXIDE_RED].map(|c| selector.shade(c)),
    );
}
