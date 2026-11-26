//! Eight sound knobs, a waveform slide switch and an Algorithm preset readout.

use super::{mixer::Band, selector::Selector, style::TB303};
use crate::{
    bass::{Control, Controls, Knob, cutoff_detent, presets::PRESETS},
    firmware::Labels,
    instrument::Voice,
    panel::Channel,
    sequencer::pattern::Automation,
};
use core::{fmt::Write, str};
use fm1_graphics::Rgb565;
use fm1_graphics::embedded_graphics::{
    Drawable,
    prelude::{Point, Primitive, Size},
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
    text::Alignment,
};
use fm1_rticx::bsp::display::Error;
use heapless::String;

const KNOB_ROW: u16 = 32;
pub(super) const SCOPE_TOP: u16 = KNOB_ROW + CELL_HEIGHT + 1;
pub(super) const METAL: Rgb565 = TB303.face;
const GRAPHITE: Rgb565 = TB303.ink;
const OXIDE: Rgb565 = TB303.accent;
const SHADOW: Rgb565 = TB303.muted;
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
        let mut status = String::<16>::new();
        write!(
            status,
            "{} ",
            ["CH1", "CH2", "CH3", "CH4"][self.channel.index()]
        )
        .expect("four channel labels");
        if let Some(note) = self.voice.gate.note {
            write!(
                status,
                "{} V{}",
                ascii(&self.voice.note_name()),
                note.velocity
            )
            .expect("channel, MIDI note and velocity fit 16 bytes");
        } else {
            write!(
                status,
                "{} {}",
                if self.voice_tap {
                    "SRC"
                } else if self.half_gain {
                    "1/2"
                } else {
                    "DAC"
                },
                self.transport,
            )
            .expect("channel, scope tap and transport fit 16 bytes");
        }
        band.label(&status, Point::new(2, 0), Alignment::Left, ink);
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Snapshot {
    controls: Controls,
    selected: Option<Control>,
    preset: u8,
    status: Status,
    automation: Option<Automation>,
}

#[derive(Eq, PartialEq)]
struct Dial {
    pointer: Point,
    selected: bool,
    marker: Option<Point>,
}

#[derive(Eq, PartialEq)]
struct WaveSwitch {
    square: bool,
    selected: bool,
    marker: Option<bool>,
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
            selected: self.selected == Some(Control::Knob(knob)),
            marker,
        }
    }

    fn wave_switch(self) -> WaveSwitch {
        WaveSwitch {
            square: self.status.voice.waveform != 0,
            selected: self.selected == Some(Control::Waveform),
            marker: self
                .automation
                .and_then(|a| a.get(crate::bass::CC_WAVE))
                .map(|value| value >= 64),
        }
    }
}

impl WaveSwitch {
    fn draw(&self, band: &mut Band<'_>, selector: &Selector) {
        let shade = |c| selector.shade(c);
        let ink = if self.selected {
            TB303.selected.ink
        } else {
            GRAPHITE
        };
        band.label("SAW", Point::new(2, 0), Alignment::Left, shade(ink));
        band.label("SQR", Point::new(76, 0), Alignment::Right, shade(ink));
        // A recessed slot, a raised ivory actuator and grip grooves match the dials' bevels.
        band.rect(Point::new(24, 1), Size::new(30, 9), shade(SHADOW));
        band.rect(Point::new(25, 2), Size::new(28, 7), shade(GRAPHITE));
        band.rect(Point::new(25, 9), Size::new(29, 1), shade(IVORY));
        let x = if self.square { 42 } else { 26 };
        band.rect(Point::new(x + 1, 3), Size::new(10, 7), shade(SHADOW));
        band.rect(Point::new(x, 2), Size::new(10, 7), shade(IVORY));
        for groove in [3, 6] {
            band.rect(Point::new(x + groove, 3), Size::new(1, 5), shade(GRAPHITE));
        }
        // The actuator retains the panel choice; a witness mark follows clip automation.
        if let Some(square) = self.marker {
            band.rect(
                Point::new(if square { 46 } else { 30 }, 0),
                Size::new(3, 2),
                shade(if self.selected { ink } else { OXIDE }),
            );
        }
    }
}

impl Dial {
    fn draw(&self, band: &mut Band<'_>, selector: &Selector, knob: Knob, center: Point) {
        let shade = |c| selector.shade(c);
        let encoder = Knob::direct().iter().position(|&k| k == knob);
        let [legend, tick, mark] = if self.selected {
            [TB303.selected.ink; 3]
        } else {
            [GRAPHITE, SHADOW, OXIDE]
        };
        let (skirt, bevel, cap, pointer) = if self.selected {
            (GRAPHITE, LAMP, IVORY, GRAPHITE)
        } else {
            let cap = if encoder.is_some() { SHADOW } else { GRAPHITE };
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
            band.rect(center + Point::new(dx, dy), Size::new(1, 2), shade(tick));
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
            .into_styled(PrimitiveStyle::with_stroke(shade(mark), 1))
            .draw(band)
            .expect("infallible strip");
            band.label(
                ["1", "2", "3", "4"][index],
                Point::new(3, 4),
                Alignment::Left,
                shade(legend),
            );
        }
        // The cap shows the stored setting. The witness mark follows the sequencer.
        if let Some(marker) = self.marker {
            band.rect(center + marker, Size::new(3, 3), shade(mark));
        }
        band.label(
            match knob {
                Knob::Resonance => "RESO",
                Knob::Overdrive => "OD",
                _ => ascii(knob.name()),
            },
            Point::new(center.x, 21),
            Alignment::Center,
            shade(legend),
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
        lcd: &mut super::PanelDisplay<'_>,
        pixels: &mut [[u8; 2]],
        selector: &mut Selector,
        labels: &Labels,
        automation: Option<Automation>,
        entering: bool,
    ) -> Result<(), Error> {
        let state = Snapshot {
            controls: labels.bass,
            selected: labels.bass_control,
            preset: labels.bass_preset,
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
            lcd.write_rect(0, 0, 78, 20, logo.pixels).await?;
        }
        if old.is_none_or(|s| {
            s.controls != state.controls
                || s.selected != state.selected
                || s.preset != state.preset
                || s.status.voice.waveform != state.status.voice.waveform
        }) {
            let mut display =
                Band::new(pixels, 0, 0, 80, 20, selector.shade(GRAPHITE)).with_panel_type(true);
            readout(&mut display, state, selector);
            lcd.write_rect(78, 0, 80, 20, display.pixels).await?;
        }
        let switch = state.wave_switch();
        if old.is_none_or(|s| s.wave_switch() != switch) {
            let fill = if switch.selected {
                selector.shade(TB303.selected.fill)
            } else {
                paper
            };
            let mut band = Band::new(pixels, 0, 0, 78, 11, fill).with_panel_type(true);
            switch.draw(&mut band, selector);
            lcd.write_rect(0, 21, 78, 11, band.pixels).await?;
        }
        if old.is_none_or(|s| s.status != state.status) {
            let mut status = Band::new(pixels, 0, 0, 80, 10, paper).with_panel_type(true);
            state.status.draw(&mut status, selector.shade(GRAPHITE));
            lcd.write_rect(78, 21, 80, 10, status.pixels).await?;
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
            let fill = if dial.selected {
                selector.shade(TB303.selected.fill)
            } else {
                paper
            };
            let mut cell = Band::new(pixels, dx, dy, w, h, fill).with_panel_type(true);
            dial.draw(&mut cell, selector, knob, center);
            lcd.write_rect(x + dx, y + dy, w, h, cell.pixels).await?;
        }
        lcd.flush().await?;
        self.previous = Some(state);
        Ok(())
    }
}

fn ascii(bytes: &[u8]) -> &str {
    str::from_utf8(bytes.trim_ascii_end()).expect("ASCII panel text")
}

fn readout(band: &mut Band<'_>, state: Snapshot, selector: &Selector) {
    let shade = |c| selector.shade(c);
    let knob = match state.selected {
        None => {
            let mut heading = String::<16>::new();
            write!(
                heading,
                "ALG {}/{}",
                u16::from(state.preset) + 1,
                PRESETS.len()
            )
            .expect("small preset bank");
            band.label(&heading, Point::new(2, 0), Alignment::Left, shade(IVORY));
            band.label(
                PRESETS[usize::from(state.preset)].name,
                Point::new(2, 10),
                Alignment::Left,
                shade(LAMP),
            );
            return;
        }
        Some(Control::Waveform) => {
            band.label(
                "ALG WAVEFORM",
                Point::new(2, 0),
                Alignment::Left,
                shade(IVORY),
            );
            band.label(
                if state.status.voice.waveform == 0 {
                    "SAW"
                } else {
                    "SQUARE"
                },
                Point::new(77, 10),
                Alignment::Right,
                shade(LAMP),
            );
            return;
        }
        Some(Control::Knob(knob)) => knob,
    };
    let (name, value, units) = state.controls.readout(knob);
    band.label("ALG", Point::new(2, 0), Alignment::Left, shade(LAMP));
    band.label(
        ascii(name),
        Point::new(77, 0),
        Alignment::Right,
        shade(IVORY),
    );
    let mut number = String::<16>::new();
    if knob == Knob::Tuning {
        write!(number, "{value:+}")
    } else {
        write!(number, "{value}")
    }
    .expect("panel value fits 16 bytes");
    write!(number, " {}", ascii(units)).expect("panel value and units fit 16 bytes");
    band.label(&number, Point::new(77, 10), Alignment::Right, shade(LAMP));
}

fn wordmark(band: &mut Band<'_>, selector: &Selector) {
    band.mask(
        Point::zero(),
        super::lettering::BASS,
        36,
        [GRAPHITE, TB303.accent].map(|c| selector.shade(c)),
    );
}
