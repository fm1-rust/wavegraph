//! One BEATS display and control layout for every beat effect.
use super::{
    BAND, WIDTH,
    lettering::GRID as FACE,
    mixer::{BACKGROUND, Band, LABEL, OUTLINE, POINTER, STRIP},
};
use crate::{
    firmware::{Labels, Motion},
    fx::BeatEffect,
    panel::Channel,
};
use fm1_graphics::{
    Rgb565,
    embedded_graphics::{
        pixelcolor::RgbColor,
        prelude::{Point, Size},
        text::Alignment,
    },
    palette,
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
                BeatEffect::Slicer => Some(motion.slicer_step),
                BeatEffect::Flanger => None,
            },
        }
    }

    pub(super) async fn paint(
        self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]; BAND * WIDTH],
        previous: Option<Self>,
    ) -> Result<(), Error> {
        let mut lcd = lcd.as_async(clock)?;
        for y in (0..SIZE).step_by(BAND) {
            if previous.is_some_and(|old| match y {
                0..24 => (old.effect, old.channel) == (self.effect, self.channel),
                24..80 => old.beats == self.beats,
                80..104 => old.bpm == self.bpm,
                104..152 => old.effect == self.effect && old.values == self.values,
                152..208 => {
                    old.effect == self.effect
                        && old.steps == self.steps
                        && old.playhead == self.playhead
                }
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
            let mut band = Band::new(pixels, 0, y, SIZE, BAND as u16, BACKGROUND);
            match y {
                0..24 => {
                    band.title(
                        match self.effect {
                            BeatEffect::Slicer => "BEAT FX / SLICER",
                            BeatEffect::Echo => "BEAT FX / ECHO",
                            BeatEffect::Flanger => "BEAT FX / FLANGER",
                        },
                        Point::new(8, 5),
                        Rgb565::WHITE,
                    );
                    band.label(
                        self.channel.label(),
                        Point::new(219, 7),
                        Alignment::Right,
                        POINTER,
                    );
                }
                24..80 => {
                    band.rect(Point::new(7, 24), Size::new(226, 56), STRIP);
                    band.label(
                        "KNOB 1 / BEATS",
                        Point::new(120, 28),
                        Alignment::Center,
                        palette::CYAN,
                    );
                    beat_value(&mut band, y, self.beats);
                }
                80..104 => {
                    let bpm = heapless::format!(18; "MASTER  {} BPM", self.bpm).expect("tempo");
                    band.label(&bpm, Point::new(120, 86), Alignment::Center, Rgb565::WHITE);
                }
                104..152 => {
                    if self.effect == BeatEffect::Slicer {
                        band.label("PER CYCLE", Point::new(155, 109), Alignment::Center, LABEL);
                        band.label(
                            "16 STEPS",
                            Point::new(155, 128),
                            Alignment::Center,
                            Rgb565::WHITE,
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
                        band.label(label, Point::new(x, 109), Alignment::Center, LABEL);
                        let readout = heapless::format!(4; "{}%", value).expect("percentage");
                        band.label(
                            &readout,
                            Point::new(x, 128),
                            Alignment::Center,
                            Rgb565::WHITE,
                        );
                    }
                }
                152..208 if self.effect != BeatEffect::Flanger => {
                    band.label(
                        if self.effect == BeatEffect::Slicer {
                            "WHITE KEYS / PASS STEPS"
                        } else {
                            "WHITE KEYS / ECHO SEND STEPS"
                        },
                        Point::new(120, 155),
                        Alignment::Center,
                        LABEL,
                    );
                    for step in 0..16u8 {
                        let x = 10 + 13 * i32::from(step) + 4 * i32::from(step / 4);
                        band.rect(
                            Point::new(x, 176),
                            Size::new(11, 13),
                            if self.steps.active(step) {
                                palette::CYAN
                            } else {
                                OUTLINE
                            },
                        );
                        if self.playhead == Some(step) {
                            band.rect(Point::new(x, 192), Size::new(11, 3), Rgb565::WHITE);
                        }
                    }
                }
                152..208 => band.label(
                    "ONE LFO SWEEP PER CYCLE",
                    Point::new(120, 174),
                    Alignment::Center,
                    POINTER,
                ),
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
                    band.label(
                        status,
                        Point::new(120, 209),
                        Alignment::Center,
                        Rgb565::WHITE,
                    );
                    band.label(
                        "SELECT FX   PRESETS CH   HOME BACK",
                        Point::new(120, 226),
                        Alignment::Center,
                        LABEL,
                    );
                }
            }
            lcd.write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
                .await?;
        }
        lcd.flush().await
    }
}

// This reserved field has the same center and glyph size on every Beat FX page.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "The longest fraction is four 24-pixel glyphs centered on the 240-pixel LCD"
)]
fn beat_value(band: &mut Band<'_>, y: u16, beats: BeatDivision) {
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
                    Rgb565::WHITE,
                    glyph.coverage(x / 2, dy / 2),
                );
            }
        }
    }
}
