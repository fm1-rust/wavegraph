//! Silver 303 faceplate with stock dials above the modified-voice controls.

use super::{lettering, mixer::Band, selector::Selector, style::TB303};
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
    pixelcolor::IntoStorage,
    prelude::{Point, Primitive, Size},
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
    text::Alignment,
};
use fm1_rticx::bsp::display::Error;
use heapless::String;

const KNOB_ROW: u16 = 32;
const EXTRA_ROW: u16 = KNOB_ROW + CELL_HEIGHT + 1;
pub(super) const SCOPE_TOP: u16 = EXTRA_ROW + CELL_HEIGHT + 1;
const DISPLAY_X: u16 = 82;
// Leave the overload lamp its own silver margin at the upper-right corner.
const DISPLAY_WIDTH: u16 = 146;
const DISPLAY_HEADING: u16 = 64;
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
        band.label(
            &status,
            Point::new(i32::from(DISPLAY_WIDTH - DISPLAY_HEADING) - 3, 0),
            Alignment::Right,
            ink,
        );
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
    fn readout_changed(self, previous: Self) -> bool {
        self.selected != previous.selected
            || match self.selected {
                None => self.preset != previous.preset,
                Some(Control::Muffler) => self.controls.muffler != previous.controls.muffler,
                Some(Control::Waveform) => {
                    self.status.voice.waveform != previous.status.voice.waveform
                }
                Some(Control::Knob(knob)) => {
                    self.controls.detent(knob) != previous.controls.detent(knob)
                }
            }
    }

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
        slider(
            band,
            selector,
            Point::new(24, 1),
            16,
            if self.square { 16 } else { 0 },
        );
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

// One recessed slot and ivory actuator for both physical switches.
fn slider(band: &mut Band<'_>, selector: &Selector, at: Point, travel: u16, offset: u16) {
    let shade = selector.shader();
    let width = u32::from(travel) + 14;
    band.rect(at, Size::new(width, 9), shade(SHADOW));
    band.rect(
        at + Point::new(1, 1),
        Size::new(width - 2, 7),
        shade(GRAPHITE),
    );
    band.rect(at + Point::new(1, 8), Size::new(width - 1, 1), shade(IVORY));
    let thumb = at + Point::new(i32::from(offset) + 2, 1);
    band.rect(thumb + Point::new(1, 1), Size::new(10, 7), shade(SHADOW));
    band.rect(thumb, Size::new(10, 7), shade(IVORY));
    for groove in [3, 6] {
        band.rect(
            thumb + Point::new(groove, 1),
            Size::new(1, 5),
            shade(GRAPHITE),
        );
    }
}

fn muffler_switch(band: &mut Band<'_>, selector: &Selector, state: Snapshot) {
    let ink = selector.shade(if state.selected == Some(Control::Muffler) {
        TB303.selected.ink
    } else {
        GRAPHITE
    });
    for (x, label) in [(10, "OFF"), (30, "1"), (50, "2")] {
        band.label(label, Point::new(x, 0), Alignment::Center, ink);
    }
    slider(
        band,
        selector,
        Point::new(4, 10),
        40,
        20 * state.controls.muffler as u16,
    );
    band.label("MUFFLER", Point::new(30, 21), Alignment::Center, ink);
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
        let glass = selector.shade(GRAPHITE);
        if entering {
            let mut logo = Band::new(pixels, 0, 0, 78, 20, paper);
            wordmark(&mut logo, selector);
            lcd.write_rect(0, 0, 78, 20, logo.pixels).await?;
            let mut legend = Band::new(pixels, 0, 0, 90, 18, paper);
            legend.mask(Point::zero(), lettering::BASS_LINE, 0, [glass; 2]);
            legend.overlay_at(selector, 145, EXTRA_ROW + 6);
            lcd.write_rect(145, EXTRA_ROW + 6, 90, 18, legend.pixels)
                .await?;
            lcd.fill_rect(
                DISPLAY_X - 1,
                0,
                DISPLAY_WIDTH + 2,
                30,
                selector.shade(SHADOW).into_storage(),
            )
            .await?;
        }
        if old.is_none_or(|s| state.readout_changed(s)) {
            let mut text = String::new();
            let (heading, value) = readout(state, &mut text);
            let mut band =
                Band::new(pixels, 0, 0, DISPLAY_HEADING, 11, glass).with_panel_type(true);
            band.label(
                heading,
                Point::new(3, 1),
                Alignment::Left,
                selector.shade(IVORY),
            );
            lcd.write_rect(DISPLAY_X, 1, DISPLAY_HEADING, 11, band.pixels)
                .await?;
            // Draw the large value in two strips to reuse the 1,920-pixel DMA buffer.
            for dy in [0, 8] {
                let mut band = Band::new(pixels, 0, dy, DISPLAY_WIDTH, 8, glass);
                band.panel_text(
                    value,
                    Point::new(2, 0),
                    Alignment::Left,
                    selector.shade(LAMP),
                    lettering::GRID,
                );
                lcd.write_rect(DISPLAY_X, 12 + dy, DISPLAY_WIDTH, 8, band.pixels)
                    .await?;
            }
        }
        if old.is_none_or(|s| s.status != state.status) {
            let width = DISPLAY_WIDTH - DISPLAY_HEADING;
            let mut band = Band::new(pixels, 0, 0, width, 11, glass).with_panel_type(true);
            state.status.draw(&mut band, selector.shade(IVORY));
            lcd.write_rect(DISPLAY_X + DISPLAY_HEADING, 1, width, 11, band.pixels)
                .await?;
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
        let selected = state.selected == Some(Control::Muffler);
        if old.is_none_or(|s| {
            (s.selected == Some(Control::Muffler)) != selected
                || s.controls.muffler != state.controls.muffler
        }) {
            let fill = if selected {
                selector.shade(TB303.selected.fill)
            } else {
                paper
            };
            let mut band = Band::new(pixels, 0, 0, 60, CELL_HEIGHT, fill).with_panel_type(true);
            muffler_switch(&mut band, selector, state);
            band.overlay_at(selector, 2 * CELL_WIDTH, EXTRA_ROW);
            lcd.write_rect(2 * CELL_WIDTH, EXTRA_ROW, 60, CELL_HEIGHT, band.pixels)
                .await?;
        }
        for (i, knob) in Knob::PANEL.into_iter().enumerate() {
            let dial = state.dial(knob);
            let previous = old.map(|s| s.dial(knob));
            if previous.as_ref() == Some(&dial) {
                continue;
            }
            let slot = u16::try_from(i).expect("eight knobs");
            let x = CELL_WIDTH * (slot % 6);
            let y = if slot < 6 { KNOB_ROW } else { EXTRA_ROW };
            let center = Point::new(i32::from(CELL_WIDTH / 2), 11);
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
                    Size::new(u32::from(CELL_WIDTH), u32::from(CELL_HEIGHT)),
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
            cell.overlay_at(selector, x + dx, y + dy);
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

fn readout(state: Snapshot, text: &mut String<16>) -> (&str, &str) {
    match state.selected {
        None => {
            write!(
                text,
                "ALG {}/{}",
                u16::from(state.preset) + 1,
                PRESETS.len()
            )
            .expect("small preset bank");
            (text.as_str(), PRESETS[usize::from(state.preset)].name)
        }
        Some(Control::Waveform) => (
            "WAVEFORM",
            if state.status.voice.waveform == 0 {
                "SAW"
            } else {
                "SQUARE"
            },
        ),
        Some(Control::Muffler) => ("MUFFLER", state.controls.muffler_label()),
        Some(Control::Knob(knob)) => {
            let (name, value, units) = state.controls.readout(knob);
            if knob == Knob::Tuning {
                write!(text, "{value:+}")
            } else {
                write!(text, "{value}")
            }
            .expect("panel value fits 16 bytes");
            write!(text, " {}", ascii(units)).expect("panel value and units fit 16 bytes");
            (ascii(name), text.as_str())
        }
    }
}

fn wordmark(band: &mut Band<'_>, selector: &Selector) {
    band.mask(
        Point::zero(),
        lettering::BASS,
        0,
        [selector.shade(GRAPHITE); 2],
    );
}
