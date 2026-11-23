//! Scope and envelope pages for the FM-1 display.

use crate::overload::Severity;
use crate::{
    bass::{Bank, Controls, Knob, envelope as bass_env},
    capture::{HEIGHT, Trace, WIDTH},
    envelope::EnvelopeControls,
    firmware::Frame,
    fx::FxPage,
    instrument::{Mixer, MixerBank, Part},
    sequencer::{Position, SeqPage, Sequencer, Track},
};
use embedded_graphics::pixelcolor::{IntoStorage, raw::ToBytes};
use fm1_graphics::{Rgb565, TextGrid, Waveform, WaveformStyle, palette};
use fm1_rticx::{
    bsp::{
        display::{Display, Error, SIZE},
        time::ticks,
    },
    time::Clock,
};
use static_cell::{ConstStaticCell, StaticCell};

mod selector;
mod sequencer;
mod splash;

use selector::Selector;

const TOP: u16 = (SIZE - HEIGHT as u16) / 2;
const GRID: u8 = 24;
const STYLE: WaveformStyle = WaveformStyle {
    background: Rgb565::new(0, 2, 1),
    grid: Rgb565::new(2, 12, 4),
    center: Rgb565::new(4, 21, 7),
    trace: Rgb565::new(9, 51, 11),
    grid_x: GRID,
    grid_y: GRID,
};
const ENV_STYLE: WaveformStyle = WaveformStyle {
    trace: Rgb565::new(18, 63, 20),
    ..STYLE
};
const SILVER: WaveformStyle = WaveformStyle {
    background: Rgb565::new(24, 49, 24),
    grid: Rgb565::new(21, 43, 21),
    center: Rgb565::new(17, 35, 17),
    trace: Rgb565::new(4, 8, 4),
    grid_x: GRID,
    grid_y: GRID,
};
// Inspired by Korg's modern M1 plug-in: charcoal housing, electric lime
// selection accents and a cool-blue numeric readout. Not a copied skin.
const ORGAN_STYLE: WaveformStyle = WaveformStyle {
    background: Rgb565::new(3, 5, 7),
    grid: Rgb565::new(5, 13, 9),
    center: Rgb565::new(8, 22, 11),
    trace: Rgb565::new(22, 59, 7),
    grid_x: GRID,
    grid_y: GRID,
};
const ORGAN_LIME: Rgb565 = Rgb565::new(25, 63, 5);
const ORGAN_MUTED: Rgb565 = Rgb565::new(12, 26, 18);
const MENU_TICKS: u32 = 100; // One second after the last PRESETS detent.
const OXIDE_RED: Rgb565 = Rgb565::new(15, 6, 3);
const BASS_MUTED: Rgb565 = Rgb565::new(7, 15, 8);
const LANE_INKS: [Rgb565; 3] = [SILVER.trace, OXIDE_RED, Rgb565::new(1, 18, 15)];
const LANE_X: u16 = 42;
type Lane = Waveform<{ bass_env::WIDTH }, { bass_env::HEIGHT as usize }>;

const BAND: usize = 8;
const TRAIL: usize = 32;

fn paper(style: WaveformStyle, x: usize, y: usize) -> Rgb565 {
    if y == WIDTH / 2 {
        style.center
    } else if x.is_multiple_of(usize::from(GRID)) || y.is_multiple_of(usize::from(GRID)) {
        style.grid
    } else {
        style.background
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Page {
    Scope(Part),
    Fx(FxPage),
    Mixer,
    Sequencer(SeqPage, Track),
    Envelope(Part, EnvelopeControls),
    BassEnvelope(Controls),
}

pub(super) struct Screen {
    plot: &'static mut Waveform<WIDTH, WIDTH>,
    lanes: [&'static mut Lane; 3],
    text: &'static mut TextGrid,
    strip: &'static mut [[u8; 2]; BAND * HEIGHT],
    selector: &'static mut Selector,
    sequence: sequencer::Grid,
    beam: Beam,
    dot: Option<Severity>,
    dot_background: Rgb565,
    page: Option<Page>,
}

impl Screen {
    pub(super) fn new() -> Self {
        static PLOT: StaticCell<Waveform<WIDTH, WIDTH>> = StaticCell::new();
        static LANES: [StaticCell<Lane>; 3] = [const { StaticCell::new() }; 3];
        static SELECTOR: ConstStaticCell<Selector> = ConstStaticCell::new(Selector::new());
        static TEXT: StaticCell<TextGrid> = StaticCell::new();
        static STRIP: ConstStaticCell<[[u8; 2]; BAND * HEIGHT]> =
            ConstStaticCell::new([[0; 2]; BAND * HEIGHT]);
        Self {
            plot: PLOT.init_with(|| Waveform::new(0, 0).expect("panel-sized scope")),
            lanes: core::array::from_fn(|i| {
                LANES[i].init_with(|| {
                    let mut lane =
                        Waveform::new(LANE_X, [64, 112, 160][i]).expect("303 envelope lane");
                    lane.set_style(WaveformStyle {
                        trace: LANE_INKS[i],
                        grid_y: 0,
                        center: SILVER.grid,
                        ..SILVER
                    });
                    lane
                })
            }),
            text: TEXT.init_with(|| TextGrid::with_background(STYLE.background)),
            strip: STRIP.take(),
            selector: SELECTOR.take(),
            sequence: sequencer::Grid::new(),
            page: None,
            dot: None,
            dot_background: STYLE.background,
            beam: Beam {
                old_head: None,
                old_trace: None,
            },
        }
    }

    fn change_view(&mut self, style: WaveformStyle) {
        self.dot = None;
        self.dot_background = style.background;
        self.plot.set_style(style);
        // Pages can share a palette; the previous pulse may still be on the LCD.
        self.plot.invalidate();
        *self.text = TextGrid::with_background(style.background);
        self.beam.old_head = None;
        self.beam.old_trace = None;
        self.sequence.invalidate();
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "All mixer rows and fader bars fit within the 20 by 15 text grid"
    )]
    async fn paint_mixer(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        mixer: Mixer,
        bank: MixerBank,
    ) -> Result<(), Error> {
        let background = STYLE.background;
        if self.page.replace(Page::Mixer) != Some(Page::Mixer) {
            self.change_view(STYLE);
            lcd.fill_rect(0, 0, SIZE, SIZE, background.into_storage())?;
        }
        self.text.text(0, 0, b"MIXER", palette::WHITE);
        self.text.text(
            11,
            0,
            match bank {
                MixerBank::Volume => b"LEVEL ".as_slice(),
                MixerBank::Reverb => b"REVERB".as_slice(),
                MixerBank::Delay => b"DELAY ".as_slice(),
            },
            palette::CYAN,
        );
        let names: [&[u8]; 4] = [b"1 MONOSYNTH", b"2 M1 ORGAN", b"3 TB-303", b"4 TR-909"];
        let colors = [palette::GREEN, ORGAN_LIME, palette::RED, palette::CYAN];
        let values = match bank {
            MixerBank::Volume => mixer.volume,
            MixerBank::Reverb => mixer.reverb,
            MixerBank::Delay => mixer.delay,
        };
        for (track, (&name, &ink)) in names.iter().zip(&colors).enumerate() {
            let y = 2 + 3 * track;
            self.text.text(0, y, name, ink);
            self.text.number(17, y, 3, u32::from(values[track]), ink);
            let filled = usize::from(values[track]) * 16 / 128;
            self.text
                .text(2, y + 1, &b"################"[..filled], ink);
            self.text.text(
                2 + filled,
                y + 1,
                &b"----------------"[..16 - filled],
                palette::MUTED,
            );
        }
        self.text.text(12, 14, b"GLO EXIT", palette::MUTED);
        self.text.paint_async(lcd, clock).await
    }

    async fn paint_envelope(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        focus: Part,
        controls: EnvelopeControls,
    ) -> Result<(), Error> {
        let page = Page::Envelope(focus, controls);
        let previous = self.page.replace(page);
        if previous != Some(page) {
            let entering = !matches!(previous, Some(Page::Envelope(part, _)) if part == focus);
            let (style, title, ink) = if focus == Part::Organ {
                (
                    WaveformStyle {
                        trace: ORGAN_LIME,
                        ..ORGAN_STYLE
                    },
                    b"ORGAN VCA ENV".as_slice(),
                    ORGAN_LIME,
                )
            } else {
                (ENV_STYLE, b"VCA ENVELOPE".as_slice(), palette::GREEN)
            };
            if entering {
                self.change_view(style);
            }
            self.plot.paint_async(lcd, &controls.trace(), clock).await?;
            if entering {
                // The curve's dirty tiles stay between the two label bands.
                let background = style.background.into_storage();
                lcd.fill_rect(0, 0, SIZE, 16, background)?;
                lcd.fill_rect(0, 192, SIZE, SIZE - 192, background)?;
            }
            self.text.text(0, 0, title, ink);
            let adsr = controls.adsr();
            env_time(self.text, 0, 12, b"A", adsr.attack_us, ink);
            env_time(self.text, 10, 12, b"D", adsr.decay_us, ink);
            self.text.text(0, 14, b"S", palette::WHITE);
            self.text
                .number(1, 14, 4, u32::from(adsr.sustain_percent), ink);
            self.text.text(5, 14, b"/100", palette::MUTED);
            env_time(self.text, 10, 14, b"R", adsr.release_us, ink);
        }
        self.text.paint_async(lcd, clock).await
    }

    fn bass_panel(&mut self, controls: Controls) {
        let (ink, muted) = (
            self.selector.shade(OXIDE_RED),
            self.selector.shade(BASS_MUTED),
        );
        self.text.text(0, 0, b"TB-303", ink);
        let bank = controls.bank;
        self.text.text(
            7,
            0,
            match bank {
                Bank::Filter => b"B1/2",
                Bank::Voice => b"B2/2",
            },
            ink,
        );
        for (i, knob) in bank.knobs().into_iter().enumerate() {
            // The right column has room for the full RESONANCE and OVERDRIVE labels.
            let (x, width) = if i % 2 == 0 { (0, 7) } else { (9, 9) };
            let y = 1 + i / 2;
            let color = if knob == controls.last() { ink } else { muted };
            self.text.text(x, y, [b"1234"[i]], ink);
            self.text.text(x + 2, y, &knob.name()[..width], color);
        }
        let (name, value, units) = controls.readout();
        self.text.text(0, 13, name, muted);
        if controls.last() == Knob::Tuning {
            self.text.signed_number(9, 13, 5, value, ink);
        } else {
            self.text.number(9, 13, 5, value.unsigned_abs(), ink);
        }
        self.text.text(15, 13, units, muted);
    }

    async fn paint_bass_envelope(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        controls: Controls,
    ) -> Result<(), Error> {
        let page = Page::BassEnvelope(controls);
        let previous = self.page.replace(page);
        if previous != Some(page) {
            if !matches!(previous, Some(Page::BassEnvelope(_))) {
                self.change_view(SILVER);
                // The three lane plots occupy part of the full-size scope's pixel area.
                self.plot.invalidate();
                lcd.fill_rect(0, 0, SIZE, SIZE, SILVER.background.into_storage())?;
                for lane in &mut self.lanes {
                    lane.invalidate();
                }
                self.text.text(12, 0, b"ENV", OXIDE_RED);
                for (i, x) in bass_env::STRIKES.into_iter().enumerate() {
                    let label: &[u8] = if i == 0 { b"PLAIN" } else { b"A" };
                    self.text
                        .text((usize::from(LANE_X) + x) / 12, 3, label, BASS_MUTED);
                }
                for (i, label) in [b"VCF", b"ACC", b"VCA"].into_iter().enumerate() {
                    self.text.text(0, 5 + i * 3, label, LANE_INKS[i]);
                }
                self.text.text(0, 14, b"PREVIEW    96 MS/DIV", BASS_MUTED);
            }
            for (lane, trace) in self.lanes.iter_mut().zip(&bass_env::traces(controls)) {
                lane.paint_async(lcd, trace, clock).await?;
            }
            self.bass_panel(controls);
        }
        self.text.paint_async(lcd, clock).await
    }

    async fn paint_sequencer(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        view: SeqPage,
        seq: Sequencer,
        position: Option<Position>,
    ) -> Result<(), Error> {
        let page = Page::Sequencer(view, seq.track);
        if self.page.replace(page) != Some(page) {
            self.change_view(STYLE);
            lcd.fill_rect(0, 0, SIZE, SIZE, STYLE.background.into_storage())?;
            let title: &[u8] = match seq.track {
                Track::Mono => b"SEQ MONO",
                Track::Organ => b"SEQ M1 ORGAN",
                Track::Bass => b"SEQ TB-303",
                Track::Drums => b"SEQ TR-909",
            };
            self.text.text(0, 0, title, palette::WHITE);
            if view == SeqPage::Pattern {
                self.text.text(14, 0, b"B", palette::MUTED);
                self.text.text(16, 1, b"S", palette::MUTED);
            } else {
                self.text.text(0, 4, b"TEMPO 2/2", palette::MUTED);
                self.text.text(0, 6, b"MASTER TEMPO", palette::WHITE);
                self.text.text(0, 8, b"KNOB 3", palette::GREEN);
                self.text.text(0, 14, b"SELECT FOR PATTERN", palette::MUTED);
            }
            self.text.text(8, 1, b"BPM", palette::MUTED);
        }
        self.text.text(
            0,
            1,
            if seq.transport.playing() {
                b"PLAY"
            } else {
                b"STOP"
            },
            if seq.transport.playing() {
                palette::GREEN
            } else {
                palette::RED
            },
        );
        self.text
            .number(5, 1, 3, u32::from(seq.transport.bpm), palette::WHITE);
        let enabled = seq.enabled[seq.track as usize];
        self.text.text(
            12,
            1,
            if enabled { b"ON " } else { b"OFF" },
            if enabled {
                palette::GREEN
            } else {
                palette::MUTED
            },
        );
        if view == SeqPage::Pattern {
            self.text.text(
                0,
                13,
                if seq.transport.playing() {
                    b"PRESETS TRACK FOLLOW"
                } else {
                    b"PRESETS TRACK K2 BAR"
                },
                palette::MUTED,
            );
            self.text.text(
                0,
                14,
                if enabled {
                    b"SEL OFF  SELECT BPM"
                } else {
                    b"SEL ON   SELECT BPM"
                },
                palette::MUTED,
            );
            self.sequence
                .paint(lcd, clock, self.text, self.strip, seq, position)
                .await?;
        }
        self.text.paint_async(lcd, clock).await
    }

    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        frame: &Frame,
        severity: Option<Severity>,
        position: Option<Position>,
    ) -> Result<(), Error> {
        self.paint_page(lcd, clock, frame, position).await?;
        if severity != self.dot {
            let color = match severity {
                None => self.dot_background,
                Some(Severity::Amber) => Rgb565::new(31, 40, 0),
                Some(Severity::Red) => palette::RED,
            };
            // Seven pixels in the reserved final header cell. The rectangle is
            // rewritten only when the lamp or page changes.
            let pixels = [0x1c_u8, 0x3e, 0x7f, 0x7f, 0x7f, 0x3e, 0x1c].map(|row| {
                core::array::from_fn::<_, 7, _>(|x| {
                    if row & (1 << x) != 0 {
                        color
                    } else {
                        self.dot_background
                    }
                    .to_be_bytes()
                })
            });
            lcd.write_rect_buffered(SIZE - 10, 4, 7, 7, pixels.as_flattened().as_flattened())?;
            self.dot = severity;
        }
        Ok(())
    }

    async fn paint_page(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        frame: &Frame,
        position: Option<Position>,
    ) -> Result<(), Error> {
        let labels = frame.labels;
        let browsing = labels.seq.page.is_none()
            && !labels.env_mode
            && !labels.fx_view
            && !labels.mixer_view
            && labels
                .menu_since
                .is_some_and(|since| ticks().wrapping_sub(since) < MENU_TICKS);
        if self.selector.select(browsing.then_some(labels.focus)) {
            // Restore the palette and any tiles previously covered by the list.
            self.plot.invalidate();
            self.page = None;
        }
        if let Some(page) = labels.seq.page {
            return self
                .paint_sequencer(lcd, clock, page, labels.seq, position)
                .await;
        }
        if labels.mixer_view {
            return self
                .paint_mixer(lcd, clock, labels.mixer, labels.mixer_bank)
                .await;
        }
        if labels.env_mode && !labels.fx_view {
            match labels.focus {
                Part::Bass => {
                    return self.paint_bass_envelope(lcd, clock, labels.bass).await;
                }
                Part::Mono | Part::Organ => {
                    let controls = if labels.focus == Part::Organ {
                        labels.organ.envelope
                    } else {
                        labels.mono_envelope
                    };
                    return self
                        .paint_envelope(lcd, clock, labels.focus, controls)
                        .await;
                }
            }
        }
        let bass = !labels.fx_view && labels.focus == Part::Bass;
        let organ = !labels.fx_view && labels.focus == Part::Organ;
        let (style, ink, muted, bright) = if bass {
            (SILVER, OXIDE_RED, BASS_MUTED, SILVER.trace)
        } else if organ {
            (ORGAN_STYLE, ORGAN_LIME, ORGAN_MUTED, palette::CYAN)
        } else {
            (STYLE, palette::GREEN, palette::MUTED, palette::WHITE)
        };
        let style = self.selector.style(style);
        let [ink, muted, bright] = [ink, muted, bright].map(|color| self.selector.shade(color));
        let page = if labels.fx_view {
            Page::Fx(labels.fx_page)
        } else {
            Page::Scope(labels.focus)
        };
        let changed_page = self.page.replace(page) != Some(page);
        if changed_page {
            self.change_view(style);
        }
        // Keep traces, their halos and the plot's eight-pixel dirty tiles clear
        // of the controls. Two footer rows begin at LCD y=208.
        let trace = if bass {
            frame.trace.map(|y| (25 + u16::from(y) * 3 / 4) as u8)
        } else if organ || labels.fx_view {
            // Preserve the center at y=120; the bottom tile ends at y=207.
            frame.trace.map(|y| (12 + u16::from(y) * 7 / 8) as u8)
        } else {
            frame.trace
        };
        // While browsing, compose moving trace pixels and the list in the same LCD write.
        if changed_page || !browsing {
            self.plot
                .paint_async(lcd, &trace.map(|y| y + TOP as u8), clock)
                .await?;
        }
        if changed_page {
            let (header, footer) = if bass || organ || labels.fx_view {
                (if bass { 48 } else { 16 }, 32)
            } else {
                (16, 16)
            };
            for (y, height) in [(0, header), (SIZE - footer, footer)] {
                lcd.fill_rect(0, y, SIZE, height, style.background.into_storage())?;
            }
        }
        let pulse_part = if labels.fx_view {
            Part::Mono
        } else {
            labels.focus
        };
        self.paint_beam(
            lcd,
            clock,
            &trace,
            style,
            pulse_part,
            position.map(|position| position.beat_phase),
        )
        .await?;
        if labels.fx_view {
            let output = labels.fx.output(labels.fx_page);
            self.text.text(
                0,
                0,
                match labels.fx_page {
                    FxPage::Reverb => b"FX / REVERB".as_slice(),
                    FxPage::Delay => b"FX / DELAY".as_slice(),
                },
                ink,
            );
            self.text
                .text(12, 0, if output.enabled { b"ON " } else { b"OFF" }, bright);
            self.text.text(
                15,
                0,
                if labels.half_gain { b"HALF" } else { b"FULL" },
                muted,
            );
            match labels.fx_page {
                FxPage::Reverb => {
                    self.text.text(0, 13, b"1 DECAY", muted);
                    self.text.number(8, 13, 3, u32::from(labels.fx.decay), ink);
                    self.text.text(12, 13, b"2 WET", muted);
                    self.text.number(17, 13, 3, u32::from(output.wet), ink);
                    self.text.text(0, 14, b"SENDS IN GLO MIXER", muted);
                }
                FxPage::Delay => {
                    self.text.text(0, 13, b"1 TIME", muted);
                    self.text
                        .number(7, 13, 4, u32::from(labels.fx.time_ms), ink);
                    self.text.text(12, 13, b"MS", muted);
                    self.text.text(0, 14, b"2 FDBK", muted);
                    self.text
                        .number(7, 14, 2, u32::from(labels.fx.feedback), ink);
                    self.text.text(11, 14, b"3 WET", muted);
                    self.text.number(17, 14, 3, u32::from(output.wet), ink);
                }
            }
            return self.text.paint_async(lcd, clock).await;
        }
        if bass {
            self.bass_panel(labels.bass);
        } else if organ {
            self.text.text(0, 0, b"M1 ORGAN", ink);
            let voices = labels
                .organ_keys
                .iter()
                .filter(|key| key.note.is_some())
                .count();
            self.text.text(9, 0, b"V", muted);
            self.text
                .number(10, 0, 1, u32::try_from(voices).unwrap_or(0), bright);
            let (name, value, units) = labels.organ.readout();
            self.text.text(0, 13, name, muted);
            self.text.number(10, 13, 4, u32::from(value), bright);
            self.text.text(15, 13, units, muted);
        } else {
            self.text.text(0, 0, b"CUT", muted);
            self.text
                .number(4, 0, 5, u32::from(labels.voice.cutoff_hz()), ink);
            self.text.text(9, 0, b"HZ", muted);
        }
        self.text.text(
            12,
            0,
            if labels.half_gain { b"HALF" } else { b"FULL" },
            muted,
        );
        let source = match (labels.voice_tap, labels.focus) {
            (false, _) => b"DAC",
            (true, Part::Mono) => b"VCA",
            (true, Part::Organ) => b"ORG",
            (true, Part::Bass) => b"SRC",
        };
        self.text.text(16, 0, source, muted);
        self.text.text(0, 14, labels.voice.label(labels.focus), ink);
        self.text.text(8, 14, labels.voice.note_name(), bright);
        self.text.text(16, 14, b"V", muted);
        let velocity = labels
            .voice
            .gate
            .note
            .map_or(0, |note| u32::from(note.velocity));
        self.text.number(17, 14, 3, velocity, ink);
        self.text.paint_async(lcd, clock).await
    }
}

fn env_time(text: &mut TextGrid, x: usize, y: usize, label: &[u8], us: u32, ink: Rgb565) {
    let (value, unit) = if us != 0 && us.is_multiple_of(1_000_000) {
        (us / 1_000_000, b"S ")
    } else if us.is_multiple_of(1000) {
        (us / 1000, b"MS")
    } else {
        (us, b"US")
    };
    text.text(x, y, label, palette::WHITE);
    text.number(x + 1, y, 5, value, ink);
    text.text(x + 6, y, unit, palette::MUTED);
}

struct Beam {
    old_head: Option<usize>,
    old_trace: Option<Trace>,
}

fn segment(trace: &Trace, x: usize) -> (usize, usize) {
    let a = usize::from(trace[x.saturating_sub(1)]);
    let b = usize::from(trace[x]);
    (a.min(b), a.max(b))
}

fn age(head: usize, x: usize) -> usize {
    (head + WIDTH - x) % WIDTH
}

// The brightest point leads. Its one-pixel halo disappears before the tail.
fn phosphor(part: Part, age: usize) -> (Rgb565, Option<Rgb565>) {
    if part == Part::Bass {
        return match age {
            0..=3 => (Rgb565::new(24, 10, 3), Some(Rgb565::new(21, 34, 16))),
            4..=9 => (OXIDE_RED, None),
            10..TRAIL => (Rgb565::new(8, 9, 5), None),
            _ => (SILVER.trace, None),
        };
    }
    if part == Part::Organ {
        return match age {
            0..=4 => (ORGAN_LIME, Some(palette::CYAN)),
            5..=12 => (Rgb565::new(19, 53, 9), None),
            13..TRAIL => (Rgb565::new(11, 37, 7), None),
            _ => (ORGAN_STYLE.trace, None),
        };
    }
    match age {
        0..=3 => (Rgb565::new(26, 63, 23), Some(Rgb565::new(9, 40, 10))),
        4..=9 => (Rgb565::new(18, 61, 17), Some(Rgb565::new(5, 28, 7))),
        10..=19 => (Rgb565::new(14, 58, 14), Some(Rgb565::new(3, 20, 5))),
        20..TRAIL => (Rgb565::new(11, 54, 12), None),
        _ => (STYLE.trace, None),
    }
}

impl Screen {
    async fn paint_beam(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        trace: &Trace,
        style: WaveformStyle,
        part: Part,
        beat_phase: Option<u8>,
    ) -> Result<(), Error> {
        let beam = &mut self.beam;
        let selector = &*self.selector;
        let head = beat_phase.map(|phase| usize::from(phase) * WIDTH / 256);
        if head.is_none() && beam.old_head.is_none() && !selector.visible() {
            return Ok(());
        }
        let mut lcd = lcd.as_async(clock)?;
        for x0 in (0..WIDTH).step_by(BAND) {
            // Include the old halo when a note or its gain changes under the beam.
            let mut top = HEIGHT;
            let mut bottom = 0;
            if selector.visible()
                && beam.old_trace.is_none()
                && x0 < selector::RIGHT
                && x0 + BAND > selector::LEFT
            {
                top = selector::TOP - usize::from(TOP);
                bottom = selector::BOTTOM - 1 - usize::from(TOP);
            }
            for x in x0..x0 + BAND {
                if selector.visible()
                    && let Some(previous) = &beam.old_trace
                {
                    let old = segment(previous, x);
                    let new = segment(trace, x);
                    if old != new {
                        top = top.min(old.0).min(new.0);
                        bottom = bottom.max(old.1).max(new.1);
                    }
                }
                for (which_head, which_trace) in [
                    (head, trace),
                    (beam.old_head, beam.old_trace.as_ref().unwrap_or(trace)),
                ] {
                    if which_head.is_some_and(|h| age(h, x) < TRAIL) {
                        let (a, b) = segment(which_trace, x);
                        top = top.min(a.saturating_sub(1));
                        bottom = bottom.max((b + 1).min(HEIGHT - 1));
                    }
                }
            }
            if top == HEIGHT {
                continue;
            }
            let height = bottom - top + 1;
            let pixels = &mut self.strip[..BAND * height];
            for dx in 0..BAND {
                let x = x0 + dx;
                let (a, b) = segment(trace, x);
                let (ink, halo) = head.map_or((style.trace, None), |h| {
                    let (ink, halo) = phosphor(part, age(h, x));
                    (selector.shade(ink), halo.map(|color| selector.shade(color)))
                });
                for y in top..=bottom {
                    let paper = paper(style, x, usize::from(TOP) + y);
                    let color = if (a..=b).contains(&y) {
                        ink
                    } else if y + 1 == a || y == b + 1 {
                        halo.unwrap_or(paper)
                    } else {
                        paper
                    };
                    pixels[(y - top) * BAND + dx] =
                        selector.pixel(x, usize::from(TOP) + y, color).to_be_bytes();
                }
            }
            lcd.write_rect_buffered(
                x0 as u16,
                TOP + top as u16,
                BAND as u16,
                height as u16,
                pixels.as_flattened(),
            )
            .await?;
        }
        lcd.flush().await?;
        beam.old_head = head;
        beam.old_trace = Some(*trace);
        Ok(())
    }
}
