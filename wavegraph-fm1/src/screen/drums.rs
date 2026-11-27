//! Drum faceplates and per-voice activity beneath the live kit scope.

use super::{InstrumentStyle, mixer::Band, selector::Selector, tr808};
use crate::{instrument::Part, panel::Channel, sequencer::DrumLamp};
use fm1_graphics::Rgb565;
use fm1_graphics::embedded_graphics::{
    Drawable,
    pixelcolor::{IntoStorage, raw::ToBytes},
    prelude::{Point, Primitive, Size},
    primitives::{PrimitiveStyle, Rectangle, RoundedRectangle},
    text::Alignment,
};
use fm1_rticx::{
    bsp::display::{Error, SIZE},
    time::Clock,
};
use wavegraph_909::Voice;

pub(super) const GRAPHITE: Rgb565 = super::style::TR909.ink;
pub(super) const HOT: Rgb565 = super::style::TR808.accent;
pub(super) const EMBER: Rgb565 = Rgb565::new(17, 13, 4);

pub(super) const CREAM: Rgb565 = super::style::TR808.ink;
pub(super) const STEP_COLORS_808: [Rgb565; 4] =
    [Rgb565::new(30, 9, 3), HOT, Rgb565::new(31, 40, 3), CREAM];
pub(super) const KEY_FACE_909: Rgb565 = Rgb565::new(30, 60, 28);
pub(super) const DARK_LENS: Rgb565 = Rgb565::new(7, 14, 6);
pub(super) const LIT_LENS: Rgb565 = Rgb565::new(31, 53, 13);

const KEY_W: usize = 39;
const KEY_H: usize = 47;
const KEY_Y: u16 = 189;
const LENS_W: usize = 20;
const LENS_H: usize = 7;
const LENS_X: u16 = 10;
const LENS_Y: u16 = 6;
const _: () = assert!(KEY_W * KEY_H <= super::BAND * crate::capture::WIDTH);

#[derive(Default)]
pub(super) struct Panel {
    previous: Option<[DrumLamp; 6]>,
    controls: Option<wavegraph_808::Controls>,
    status808: Option<(Channel, bool, &'static str)>,
}

impl Panel {
    async fn title(
        lcd: &mut super::PanelDisplay<'_>,
        strip: &mut [[u8; 2]],
        selector: &mut Selector,
        part: Part,
        theme: &InstrumentStyle,
    ) -> Result<(), Error> {
        // The 808 controls repaint from x=114. Keep the whole title tile to their left.
        let (mask, ink) = if part == Part::Tr808 {
            (super::lettering::TR808, theme.ink)
        } else {
            (super::lettering::TR909, Rgb565::new(10, 20, 10))
        };
        let width = mask.width as u16;
        let ink = selector.shade(ink);
        let background = selector.shade(theme.face);
        for y in (0..32).step_by(8) {
            let mut band = Band::new(strip, 0, y, width, 8, background);
            band.mask(Point::zero(), mask, 0, [ink; 2]);
            lcd.write_rect(10, y + 1, width, 8, band.pixels).await?;
        }
        Ok(())
    }

    fn key_face(key: &mut Band<'_>, voice: Voice, slot: usize, selector: &Selector) {
        // Recess, lower bevel and face of the keycap.
        for (x, y, w, h, ink) in [
            (0, 0, 39, 47, GRAPHITE),
            (1, 1, 37, 44, Rgb565::new(23, 46, 21)),
            (2, 1, 35, 35, KEY_FACE_909),
        ] {
            RoundedRectangle::with_equal_corners(
                Rectangle::new(Point::new(x, y), Size::new(w, h)),
                Size::new(3, 3),
            )
            .into_styled(PrimitiveStyle::with_fill(selector.shade(ink)))
            .draw(key)
            .expect("infallible framebuffer");
        }
        let (first, second) = match voice {
            Voice::Kick => ("KICK", ""),
            Voice::Snare => ("SNARE", ""),
            Voice::Clap => ("CLAP", ""),
            Voice::ClosedHat => ("CLSD", "HAT"),
            Voice::OpenHat => ("OPEN", "HAT"),
            Voice::Ride => ("RIDE", ""),
        };
        let ink = selector.shade(GRAPHITE);
        for (label, y) in [
            (first, if second.is_empty() { 20 } else { 16 }),
            (second, 26),
            (["F3", "G3", "A3", "B3", "C4", "D4"][slot], 35),
        ] {
            key.label(label, Point::new(19, y), Alignment::Center, ink);
        }
    }

    pub(super) async fn status808(
        &mut self,
        lcd: &mut super::PanelDisplay<'_>,
        strip: &mut [[u8; 2]],
        selector: &Selector,
        status: (Channel, bool, &'static str),
        entering: bool,
    ) -> Result<(), Error> {
        if entering || self.status808 != Some(status) {
            tr808::status(lcd, strip, selector, status).await?;
            self.status808 = Some(status);
        }
        Ok(())
    }

    pub(super) async fn controls(
        &mut self,
        lcd: &mut super::PanelDisplay<'_>,
        strip: &mut [[u8; 2]],
        selector: &Selector,
        c: wavegraph_808::Controls,
        entering: bool,
    ) -> Result<(), Error> {
        let previous = if entering { None } else { self.controls };
        if previous == Some(c) {
            return Ok(());
        }
        tr808::controls(lcd, strip, selector, c, previous).await?;
        self.controls = Some(c);
        Ok(())
    }

    pub(super) async fn paint(
        &mut self,
        lcd: &mut super::PanelDisplay<'_>,
        strip: &mut [[u8; 2]],
        selector: &mut Selector,
        entering: bool,
        (part, theme): (Part, &InstrumentStyle),
        lamps: [DrumLamp; 6],
    ) -> Result<(), Error> {
        if !entering && self.previous == Some(lamps) {
            return Ok(());
        }
        if entering {
            Self::title(lcd, strip, selector, part, theme).await?;
            let rule = selector.shade(theme.accent);
            for y in [52, 185] {
                lcd.fill_rect(0, y, SIZE, 2, rule.into_storage()).await?;
            }
        }
        if part == Part::Tr808 {
            tr808::switches(
                lcd,
                strip,
                selector,
                lamps,
                if entering { None } else { self.previous },
            )
            .await?;
            self.previous = Some(lamps);
            return Ok(());
        }
        for (i, voice) in Voice::ALL.into_iter().enumerate() {
            let lamp = lamps[voice as usize];
            let x = 1 + 40 * u16::try_from(i).expect("six keys");
            if entering {
                let mut key = Band::new(
                    strip,
                    0,
                    0,
                    KEY_W as u16,
                    KEY_H as u16,
                    selector.shade(theme.face),
                )
                .with_panel_type(true);
                Self::key_face(&mut key, voice, i, selector);
                lcd.write_rect(x, KEY_Y, KEY_W as u16, KEY_H as u16, key.pixels)
                    .await?;
            } else if self.previous.is_some_and(|old| old[voice as usize] == lamp) {
                continue;
            }
            let (lens, core) = match lamp {
                DrumLamp::Off => (DARK_LENS, Rgb565::new(10, 19, 8)),
                DrumLamp::Tail => (EMBER, theme.peak),
                DrumLamp::Strike => (super::style::mix(theme.peak, LIT_LENS), LIT_LENS),
            };
            let [rim, lens, core] =
                [GRAPHITE, lens, core].map(|color| selector.shade(color).to_be_bytes());
            let mut pixels: [[_; LENS_W]; LENS_H] = core::array::from_fn(|y| {
                core::array::from_fn(|x| {
                    if y == 0 || y + 1 == LENS_H || x == 0 || x + 1 == LENS_W {
                        rim
                    } else if (2..=3).contains(&y) && (4..LENS_W - 4).contains(&x) {
                        core
                    } else {
                        lens
                    }
                })
            });
            lcd.write_rect(
                x + LENS_X,
                KEY_Y + LENS_Y,
                LENS_W as u16,
                LENS_H as u16,
                pixels.as_flattened_mut(),
            )
            .await?;
        }
        self.previous = Some(lamps);
        Ok(())
    }
}

/// Percussion contours follow individual strikes, independent of key release.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Envelope {
    channel: Channel,
    values: [u8; 4],
    lamps: [DrumLamp; 6],
}

impl Envelope {
    pub(super) fn new(labels: &crate::firmware::Labels, lamps: [DrumLamp; 6]) -> Self {
        let c = labels.drums808;
        let [kick, snare, clap] = labels.decay909;
        let values = if labels.focus == Part::Tr808 {
            [c.kick_tone, c.kick_decay, c.snare_tone, c.snappy]
        } else {
            [kick, snare, clap, 0]
        };
        Self {
            channel: labels.channel,
            values,
            lamps,
        }
    }

    pub(super) async fn paint(
        self,
        lcd: &mut fm1_rticx::bsp::display::Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]],
        labels: &crate::firmware::Labels,
        theme: &InstrumentStyle,
        old: Option<Self>,
    ) -> Result<(), Error> {
        if old == Some(self) {
            return Ok(());
        }
        let is_808 = labels.focus == Part::Tr808;
        let controls: &[(&str, Voice)] = if is_808 {
            &[
                ("KICK TONE", Voice::Kick),
                ("KICK DECAY", Voice::Kick),
                ("SNARE TONE", Voice::Snare),
                ("SNAPPY", Voice::Snare),
            ]
        } else {
            &[
                ("KICK DECAY", Voice::Kick),
                ("SNARE DECAY", Voice::Snare),
                ("CLAP DECAY", Voice::Clap),
            ]
        };
        let mut lcd = lcd.as_async(clock)?;
        for y in (0..SIZE).step_by(super::BAND) {
            let row = usize::from(y.saturating_sub(48) / 40);
            let control = (48..208).contains(&y).then(|| controls.get(row)).flatten();
            let header_dirty = y < 24 && old.is_some_and(|old| old.channel != self.channel);
            if !header_dirty
                && old.is_some_and(|old| {
                    control.is_none_or(|&(_, voice)| {
                        old.values[row] == self.values[row]
                            && old.lamps[voice as usize] == self.lamps[voice as usize]
                    })
                })
            {
                continue;
            }
            let (x, width) = if header_dirty { (196, 28) } else { (0, SIZE) };
            let mut band = Band::new(pixels, x, y, width, super::BAND as u16, theme.face)
                .with_panel_type(true);
            if y < 48 {
                band.title(
                    if is_808 { "TR-808 ENV" } else { "TR-909 ENV" },
                    Point::new(10, 7),
                    theme.ink,
                );
                band.label(
                    labels.channel.label(),
                    Point::new(222, 9),
                    Alignment::Right,
                    theme.muted,
                );
                band.label(
                    "ONE-SHOT VOICES",
                    Point::new(10, 28),
                    Alignment::Left,
                    theme.accent,
                );
            } else if let Some(&(name, voice)) = control {
                let top = 48 + i32::try_from(row).expect("four rows") * 40;
                let value = self.values[row];
                let unit = if is_808 { "/127" } else { "%" };
                let readout = heapless::format!(8; "{value}{unit}").expect("short drum value");
                band.rect(Point::new(6, top), Size::new(228, 36), theme.surface);
                band.knob(
                    Point::new(22, top + 18),
                    value,
                    if is_808 { 127 } else { 100 },
                    [theme.face, theme.muted, theme.accent],
                );
                band.label(
                    ["1", "2", "3", "4"][row],
                    Point::new(40, top + 12),
                    Alignment::Left,
                    theme.muted,
                );
                band.title(name, Point::new(54, top + 4), theme.ink);
                band.label(
                    match self.lamps[voice as usize] {
                        DrumLamp::Strike => "HIT",
                        DrumLamp::Tail => "TAIL",
                        DrumLamp::Off => "IDLE",
                    },
                    Point::new(54, top + 21),
                    Alignment::Left,
                    theme.muted,
                );
                band.label(
                    &readout,
                    Point::new(223, top + 13),
                    Alignment::Right,
                    theme.accent,
                );
            } else if y >= 208 {
                band.label(
                    if is_808 {
                        "CLOSED HAT CHOKES OPEN HAT"
                    } else {
                        "HATS / RIDE  -  SAMPLE TAILS"
                    },
                    Point::new(10, 221),
                    Alignment::Left,
                    theme.ink,
                );
            }
            lcd.write_rect_buffered(x, y, width, super::BAND as u16, band.pixels.as_flattened())
                .await?;
        }
        lcd.flush().await
    }
}
