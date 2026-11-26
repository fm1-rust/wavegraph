//! One BEATS display and control layout for every beat effect.
use super::{BAND, InstrumentStyle, WIDTH, drums, lettering::GRID as FACE, mixer::Band};
use crate::{
    firmware::{Labels, Motion},
    fx::BeatEffect,
    instrument::Part,
    panel::Channel,
};
use fm1_graphics::{
    Rgb565,
    embedded_graphics::{
        prelude::{Point, Size},
        text::Alignment,
    },
};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};
use wavegraph_fx::{BeatDivision, StepPattern};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Panel {
    effect: BeatEffect,
    channel: Channel,
    pub(super) part: Part,
    beats: BeatDivision,
    bpm: u16,
    return_enabled: bool,
    echo_input_open: bool,
    slicer_enabled: bool,
    target: Option<Channel>,
    values: [u8; 3],
    steps: StepPattern,
    playhead: Option<u8>,
}

impl Panel {
    pub(super) fn new(effect: BeatEffect, labels: &Labels, motion: &Motion) -> Self {
        let fx = labels.fx;
        let percent = |v| u8::try_from((u16::from(v) * 100 + 64) / 128).expect("percentage");
        let slicer = fx.slicers[labels.channel.index()];
        let values = match effect {
            BeatEffect::Slicer => [percent(slicer.depth), 0, 0],
            BeatEffect::Echo => [
                percent(labels.mixer.0[labels.channel.index()].delay),
                fx.feedback,
                percent(fx.delay.wet),
            ],
            BeatEffect::Flanger => [
                percent(fx.beat_flanger.mix),
                u8::try_from((u16::from(fx.beat_flanger.feedback) * 75 + 64) / 128)
                    .expect("feedback percent"),
                percent(fx.beat_flanger.depth),
            ],
        };
        Self {
            effect,
            channel: labels.channel,
            part: labels.focus,
            beats: fx.beats,
            bpm: labels.seq.transport.bpm,
            return_enabled: fx.delay.enabled,
            echo_input_open: fx.echo_input_open,
            slicer_enabled: slicer.enabled,
            target: fx.flanger_target,
            values,
            steps: labels.beat_steps,
            playhead: match effect {
                BeatEffect::Echo => motion.position.map(|p| p.step & 15),
                BeatEffect::Flanger | BeatEffect::Slicer => Some(motion.beat_step),
            },
        }
    }

    pub(super) async fn paint(
        self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]; BAND * WIDTH],
        theme: &InstrumentStyle,
        previous: Option<Self>,
    ) -> Result<(), Error> {
        let mut lcd = lcd.as_async(clock)?;
        for y in (0..SIZE).step_by(BAND) {
            if previous.is_some_and(|old| match y {
                0..32 => (old.effect, old.channel) == (self.effect, self.channel),
                32..80 => old.beats == self.beats,
                80..104 => old.bpm == self.bpm,
                104..152 => old.effect == self.effect && old.values == self.values,
                152..176 => old.effect == self.effect,
                176..208 => true,
                _ => {
                    old.effect == self.effect
                        && old.return_enabled == self.return_enabled
                        && old.echo_input_open == self.echo_input_open
                        && old.slicer_enabled == self.slicer_enabled
                        && old.target == self.target
                        && old.channel == self.channel
                }
            }) {
                continue;
            }
            // Preserve the overload lamp on partial header updates.
            let width = if previous.is_some() && y < 16 {
                SIZE - 12
            } else {
                SIZE
            };
            let mut band = Band::new(pixels, 0, y, width, BAND as u16, theme.face)
                .with_panel_type(super::lettering::panel_type(self.part));
            match y {
                0..32 => {
                    super::badge::draw(&mut band, self.channel, Point::zero());
                    band.title(
                        match self.effect {
                            BeatEffect::Slicer => "BEAT FX / SLICER",
                            BeatEffect::Echo => "BEAT FX / ECHO",
                            BeatEffect::Flanger => "BEAT FX / FLANGER",
                        },
                        Point::new(32, 9),
                        theme.ink,
                    );
                    band.label(
                        self.part.name(),
                        Point::new(226, 11),
                        Alignment::Right,
                        theme.accent,
                    );
                }
                32..80 => {
                    band.rect(Point::new(7, 32), Size::new(226, 48), theme.surface);
                    band.label(
                        "KNOB 1 / BEATS",
                        Point::new(120, 33),
                        Alignment::Center,
                        theme.ink,
                    );
                    beat_value(&mut band, y, self.beats, theme.ink);
                }
                80..104 => {
                    let bpm = heapless::format!(18; "MASTER  {} BPM", self.bpm).expect("tempo");
                    band.label(&bpm, Point::new(120, 86), Alignment::Center, theme.ink);
                }
                104..152 => {
                    if self.effect == BeatEffect::Slicer {
                        band.label(
                            "PER CYCLE",
                            Point::new(155, 109),
                            Alignment::Center,
                            theme.ink,
                        );
                        band.label(
                            "16 STEPS",
                            Point::new(155, 128),
                            Alignment::Center,
                            theme.ink,
                        );
                    }
                    let last = if self.effect == BeatEffect::Echo {
                        "4 RETURN"
                    } else {
                        "4 SWEEP"
                    };
                    for ((x, label), value) in [(42, "2 DEPTH"), (120, "3 FEEDBACK"), (198, last)]
                        .into_iter()
                        .zip(self.values)
                        .take(if self.effect == BeatEffect::Slicer {
                            1
                        } else {
                            3
                        })
                    {
                        band.label(label, Point::new(x, 109), Alignment::Center, theme.ink);
                        let readout = heapless::format!(4; "{}%", value).expect("percentage");
                        band.label(&readout, Point::new(x, 128), Alignment::Center, theme.ink);
                    }
                }
                152..176 => band.label(
                    match self.effect {
                        BeatEffect::Echo => "WHITE KEYS / ECHO SEND STEPS",
                        BeatEffect::Slicer => "WHITE KEYS / PASS STEPS",
                        BeatEffect::Flanger => "LFO SWEEP / ONE CYCLE",
                    },
                    Point::new(120, 155),
                    Alignment::Center,
                    theme.ink,
                ),
                176..208 => (),
                _ => {
                    let routed;
                    let status = match self.effect {
                        BeatEffect::Slicer if self.slicer_enabled => "SLICER ON / SEL BYPASS",
                        BeatEffect::Slicer => "SLICER OFF / SEL APPLY",
                        BeatEffect::Echo if !self.return_enabled => "RETURN MUTED / SEL ENABLE",
                        BeatEffect::Echo if self.echo_input_open => "SEND OPEN / SEL CLOSE",
                        BeatEffect::Echo => "SEND CLOSED / SEL OPEN",
                        BeatEffect::Flanger if self.target == Some(self.channel) => {
                            "FLANGER ON / SEL BYPASS"
                        }
                        BeatEffect::Flanger => match self.target {
                            Some(target) => {
                                routed =
                                    heapless::format!(32; "ON {} / SEL MOVE HERE", target.label())
                                        .expect("fixed channel label");
                                routed.as_str()
                            }
                            None => "SEL APPLY TO THIS CHANNEL",
                        },
                    };
                    band.label(status, Point::new(120, 209), Alignment::Center, theme.ink);
                    band.label(
                        "SELECT FX   PRESETS CH   HOME BACK",
                        Point::new(120, 226),
                        Alignment::Center,
                        theme.ink,
                    );
                }
            }
            lcd.write_rect_buffered(0, y, width, BAND as u16, band.pixels.as_flattened())
                .await?;
        }
        // A moving playhead updates only the old and new keys (220 pixels each).
        for step in 0..16 {
            let state = self.key_state(step);
            if previous.is_some_and(|old| old.key_state(step) == state) {
                continue;
            }
            let x = 10 + 13 * u16::from(step) + 4 * u16::from(step / 4);
            let mut key = Band::new(pixels, 0, 0, 11, 20, theme.face);
            self.step_key(&mut key, theme, step, state);
            lcd.write_rect_buffered(x, 176, 11, 20, key.pixels.as_flattened())
                .await?;
        }
        lcd.flush().await
    }

    fn key_state(&self, step: u8) -> (bool, bool) {
        let current = self.playhead == Some(step);
        let active = if self.effect == BeatEffect::Flanger {
            current
        } else {
            self.steps.active(step)
        };
        (active, current)
    }

    fn step_key(
        &self,
        band: &mut Band<'_>,
        theme: &InstrumentStyle,
        step: u8,
        (active, current): (bool, bool),
    ) {
        let cap = match self.part {
            Part::Tr808 => drums::STEP_COLORS_808[usize::from(step / 4)],
            Part::Tr909 => drums::KEY_FACE_909,
            _ if active => theme.selected.fill,
            _ => theme.surface,
        };
        let rim = if self.part.is_kit() {
            theme.ink
        } else {
            theme.muted
        };
        let lamp = match self.part {
            Part::Tr808 | Part::Tr909 if active => drums::LIT_LENS,
            Part::Tr808 | Part::Tr909 => drums::DARK_LENS,
            _ if active => theme.selected.ink,
            _ => theme.face,
        };
        band.rect(Point::zero(), Size::new(11, 14), rim);
        band.rect(Point::new(1, 1), Size::new(9, 12), cap);
        band.rect(
            Point::new(2, 4),
            Size::new(7, 5),
            if self.part.is_kit() {
                drums::GRAPHITE
            } else {
                rim
            },
        );
        band.rect(Point::new(3, 5), Size::new(5, 3), lamp);
        if current {
            band.rect(Point::new(0, 17), Size::new(11, 3), theme.selected.fill);
        }
    }
}

// This reserved field has the same center and glyph size on every Beat FX page.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "The longest fraction is four 24-pixel glyphs centered on the 240-pixel LCD"
)]
fn beat_value(band: &mut Band<'_>, y: u16, beats: BeatDivision, ink: Rgb565) {
    let label = beats.label();
    let left = (WIDTH - label.len() * 24) / 2;
    for (i, c) in label.bytes().enumerate() {
        let glyph = FACE.glyph(c);
        for (row, pixels) in band.pixels.chunks_exact_mut(WIDTH).enumerate() {
            let Some(dy) = (usize::from(y) + row).checked_sub(44).filter(|&v| v < 32) else {
                continue;
            };
            for x in 0..24 {
                super::lettering::blend(
                    &mut pixels[left + i * 24 + x],
                    ink,
                    glyph.coverage(x / 2, dy / 2),
                );
            }
        }
    }
}
