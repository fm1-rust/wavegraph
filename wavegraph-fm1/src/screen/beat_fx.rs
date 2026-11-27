//! One BEATS display and control layout for every beat effect.
use super::{BAND, InstrumentStyle, WIDTH, drums, lettering::GRID as FACE, mixer::Band};
use crate::{
    firmware::{Labels, Motion},
    fx::{BeatEffect, FilterSettings},
    instrument::Part,
    panel::{Channel, View},
};
use core::ops::Range;
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
use wavegraph_fx::{BeatDivision, FILTER_PATTERNS, FilterMode, StepPattern};

pub(super) const SCOPE: Range<u16> = 216..SIZE;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Panel {
    pub(super) effect: BeatEffect,
    channel: Channel,
    pub(super) part: Part,
    beats: BeatDivision,
    bpm: u16,
    return_enabled: bool,
    echo_input_open: bool,
    slicer_enabled: bool,
    filter: FilterSettings,
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
            BeatEffect::Filter => [0; 3],
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
            filter: fx.filters[labels.channel.index()],
            target: fx.flanger_target,
            values,
            steps: labels.beat_steps,
            playhead: match effect {
                BeatEffect::Echo => motion.position.map(|p| p.step & 15),
                BeatEffect::Flanger | BeatEffect::Slicer | BeatEffect::Filter => {
                    Some(motion.beat_step)
                }
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
        for y in (0..SCOPE.start).step_by(BAND) {
            if previous.is_some_and(|old| match y {
                0..16 => old.channel == self.channel,
                16..32 => {
                    old.effect == self.effect
                        && old.channel == self.channel
                        && (self.effect != BeatEffect::Filter
                            || old.filter.pick.edited() == self.filter.pick.edited())
                }
                32..80 => old.beats == self.beats,
                80..104 => {
                    old.bpm == self.bpm
                        && (old.effect == BeatEffect::Filter) == (self.effect == BeatEffect::Filter)
                }
                104..176 if self.effect == BeatEffect::Filter => {
                    old.effect == self.effect
                        && old.filter.pick.rhythm == self.filter.pick.rhythm
                        && old.filter.controls() == self.filter.controls()
                }
                104..152 => old.effect == self.effect && old.values == self.values,
                152..176 => old.effect == self.effect,
                176..200 => true,
                _ => {
                    old.effect == self.effect
                        && old.return_enabled == self.return_enabled
                        && old.echo_input_open == self.echo_input_open
                        && old.slicer_enabled == self.slicer_enabled
                        && old.filter.pick.enabled == self.filter.pick.enabled
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
                    band.fx_tabs(self.channel, View::BeatFx(self.effect), theme);
                    band.title(
                        match self.effect {
                            BeatEffect::Filter if self.filter.pick.edited() => {
                                "SELECT: PCF / EDITED"
                            }
                            BeatEffect::Filter => "SELECT: PCF",
                            BeatEffect::Slicer => "SELECT: SLICER",
                            BeatEffect::Echo => "SELECT: SHARED ECHO",
                            BeatEffect::Flanger => "SELECT: FLANGER",
                        },
                        Point::new(29, 17),
                        theme.ink,
                    );
                    band.label(
                        self.part.name(),
                        Point::new(235, 19),
                        Alignment::Right,
                        theme.accent,
                    );
                }
                32..80 => {
                    band.rect(Point::new(7, 32), Size::new(226, 48), theme.surface);
                    band.label(
                        "KNOB 1 / SHARED BEAT TIME",
                        Point::new(120, 33),
                        Alignment::Center,
                        theme.ink,
                    );
                    beat_value(&mut band, y, self.beats, theme.ink);
                }
                80..104 => {
                    let bpm = heapless::format!(18; "MASTER  {} BPM", self.bpm).expect("tempo");
                    let line = if self.effect == BeatEffect::Filter {
                        band.label(
                            "WHITE: 16 STEPS    BLACK 1-8: RHYTHMS",
                            Point::new(120, 93),
                            Alignment::Center,
                            theme.ink,
                        );
                        81
                    } else {
                        86
                    };
                    band.label(&bpm, Point::new(8, line), Alignment::Left, theme.ink);
                    band.label(
                        "TAP FX: CHANNEL FX",
                        Point::new(232, line),
                        Alignment::Right,
                        theme.ink,
                    );
                }
                104..176 if self.effect == BeatEffect::Filter => {
                    self.filter_panel(&mut band, y, theme)
                }
                104..152 => {
                    if self.effect == BeatEffect::Slicer {
                        band.label(
                            "LIT: PASS",
                            Point::new(155, 109),
                            Alignment::Center,
                            theme.ink,
                        );
                        band.label(
                            "DARK: ATTENUATE",
                            Point::new(155, 128),
                            Alignment::Center,
                            theme.ink,
                        );
                    }
                    let (first, last) = match self.effect {
                        BeatEffect::Echo => ("2 SEND", "4 RETURN"),
                        BeatEffect::Flanger => ("2 MIX", "4 SWEEP"),
                        BeatEffect::Slicer => ("2 DEPTH", ""),
                        BeatEffect::Filter => unreachable!(),
                    };
                    for ((x, label), value) in [(42, first), (120, "3 FEEDBACK"), (198, last)]
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
                        BeatEffect::Slicer => "16 WHITE KEYS: TOGGLE STEPS",
                        BeatEffect::Flanger => "LFO SWEEP / ONE CYCLE",
                        BeatEffect::Filter => unreachable!(),
                    },
                    Point::new(120, 155),
                    Alignment::Center,
                    theme.ink,
                ),
                176..200 => (),
                _ => {
                    let routed;
                    let status = match self.effect {
                        BeatEffect::Filter if self.filter.pick.enabled => "ON / SEL BYPASS",
                        BeatEffect::Filter => "OFF / SEL APPLY",
                        BeatEffect::Slicer if self.slicer_enabled => "ON / SEL BYPASS",
                        BeatEffect::Slicer => "OFF / SEL APPLY",
                        BeatEffect::Echo if !self.return_enabled => "RETURN MUTED / SEL ENABLE",
                        BeatEffect::Echo if self.echo_input_open => "SEND OPEN / SEL CLOSE",
                        BeatEffect::Echo => "SEND CLOSED / SEL OPEN",
                        BeatEffect::Flanger if self.target == Some(self.channel) => {
                            "ON / SEL BYPASS"
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
                        match self.effect {
                            BeatEffect::Echo => "ECHO RETURN",
                            BeatEffect::Flanger => "FLANGER OUT",
                            BeatEffect::Slicer => "SLICER OUT / CYCLE",
                            BeatEffect::Filter => "PCF OUT",
                        },
                        Point::new(4, 201),
                        Alignment::Left,
                        theme.accent,
                    );
                    band.label(status, Point::new(236, 201), Alignment::Right, theme.ink);
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

    fn filter_panel(&self, band: &mut Band<'_>, y: u16, theme: &InstrumentStyle) {
        let f = self.filter;
        match y {
            104..128 => {
                let controls = f.controls();
                let cutoff = heapless::format!(7; "{}Hz", controls.cutoff_hz).expect("cutoff");
                let percent = |v| (u16::from(v) * 100 + 64) / 128;
                let q = heapless::format!(4; "{}%", percent(f.resonance)).expect("resonance");
                let env = heapless::format!(4; "{}%", percent(f.amount)).expect("amount");
                for (x, name, value) in [
                    (40, "2 CUTOFF", cutoff.as_str()),
                    (120, "3 RES", q.as_str()),
                    (200, "4 ENV", env.as_str()),
                ] {
                    band.label(name, Point::new(x, 104), Alignment::Center, theme.ink);
                    band.label(value, Point::new(x, 116), Alignment::Center, theme.ink);
                }
            }
            128..160 => {
                let row = usize::from((y - 128) / 16);
                let top = 128 + i32::try_from(row * 16).expect("two rows");
                for col in 0..4 {
                    let index = row * 4 + col;
                    let left = i32::try_from(col * 60).expect("four columns");
                    let ink = if index == usize::from(f.pick.rhythm) {
                        band.rect(
                            Point::new(left, top),
                            Size::new(60, 16),
                            theme.selected.fill,
                        );
                        theme.selected.ink
                    } else {
                        theme.ink
                    };
                    band.label(
                        FILTER_PATTERNS[index].name,
                        Point::new(left + 30, top + 3),
                        Alignment::Center,
                        ink,
                    );
                }
            }
            _ => {
                let mode = match f.pick.mode {
                    FilterMode::LowPass => "LP",
                    FilterMode::BandPass => "BP",
                };
                let detail =
                    heapless::format!(40; "BLACK 9/10: {}   ALG DECAY {}ms", mode, f.decay_ms)
                        .expect("filter legend");
                band.label(&detail, Point::new(120, 162), Alignment::Center, theme.ink);
            }
        }
    }

    fn key_state(&self, step: u8) -> (u8, bool) {
        let current = self.playhead == Some(step);
        if self.effect == BeatEffect::Filter {
            return (self.filter.pick.accent(step), current);
        }
        let active = if self.effect == BeatEffect::Flanger {
            current
        } else {
            self.steps.active(step)
        };
        (if active { 127 } else { 0 }, current)
    }

    fn step_key(
        &self,
        band: &mut Band<'_>,
        theme: &InstrumentStyle,
        step: u8,
        (strength, current): (u8, bool),
    ) {
        let active = strength != 0;
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
        let width = if active {
            u32::from(strength).div_ceil(26)
        } else {
            5
        };
        band.rect(Point::new(3, 5), Size::new(width, 3), lamp);
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
