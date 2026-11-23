//! Scope and envelope pages for the FM-1 display.

use crate::overload::Severity;
use crate::{
    bass::{Bank, Controls, Knob, envelope as bass_env},
    capture::{HEIGHT, Trace, WIDTH},
    envelope::EnvelopeControls,
    firmware::{Frame, Labels, Motion},
    fx::FxPage,
    instrument::Part,
    panel::{CHANNELS, Channel, View},
    sequencer::{Position, SeqPage, Sequencer},
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

mod drums;
mod mixer;
mod selector;
mod sequencer;
mod splash;
mod tr808;

use selector::{Menu, Selector};

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
    Scope(Channel, Part),
    Fx(FxPage),
    Mixer,
    Sequencer(SeqPage, Channel, Part),
    Envelope(Part, EnvelopeControls),
    BassEnvelope(Controls),
}

pub(super) struct Screen {
    plot: &'static mut Waveform<WIDTH, WIDTH>,
    lanes: [&'static mut Lane; 3],
    text: &'static mut TextGrid,
    strip: &'static mut [[u8; 2]; BAND * WIDTH],
    selector: &'static mut Selector,
    drums: drums::Panel,
    mixer: mixer::Surface,
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
        static STRIP: ConstStaticCell<[[u8; 2]; BAND * WIDTH]> =
            ConstStaticCell::new([[0; 2]; BAND * WIDTH]);
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
            drums: drums::Panel::new(),
            mixer: mixer::Surface::default(),
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

    async fn paint_mixer(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        labels: Labels,
        peaks: [u8; CHANNELS],
    ) -> Result<(), Error> {
        if self.page.replace(Page::Mixer) != Some(Page::Mixer) {
            self.change_view(WaveformStyle {
                background: self.selector.shade(mixer::BACKGROUND),
                ..STYLE
            });
            self.mixer.reset();
        }
        self.mixer
            .paint(lcd, clock, self.strip, self.selector, labels, peaks)
            .await
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
        part: Part,
        position: Option<Position>,
    ) -> Result<(), Error> {
        let page = Page::Sequencer(view, seq.channel, part);
        if self.page.replace(page) != Some(page) {
            self.change_view(STYLE);
            lcd.fill_rect(0, 0, SIZE, SIZE, STYLE.background.into_storage())?;
            let title: &[u8] = match part {
                Part::Mono => b"SEQ MONO",
                Part::Organ => b"SEQ M1 ORG",
                Part::Bass if seq.channel == Channel::Two => b"SEQ 303 HI",
                Part::Bass => b"SEQ 303 LOW",
                Part::Tr909 => b"SEQ TR-909",
                Part::Tr808 => b"SEQ TR-808",
            };
            self.text.text(0, 0, title, palette::WHITE);
            self.text.text(
                12,
                0,
                [b"C1", b"C2", b"C3", b"C4"][seq.channel.index()],
                palette::CYAN,
            );
            if view == SeqPage::Pattern {
                self.text.text(15, 0, b"B", palette::MUTED);
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
        let enabled = seq.enabled[seq.channel.index()];
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
                .paint(lcd, clock, self.text, self.strip, (seq, part), position)
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
        motion: Motion,
    ) -> Result<(), Error> {
        self.paint_page(lcd, clock, frame, motion).await?;
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
        motion: Motion,
    ) -> Result<(), Error> {
        let labels = frame.labels;
        let menu = if labels
            .menu_since
            .is_some_and(|since| ticks().wrapping_sub(since) < MENU_TICKS)
        {
            match labels.view {
                View::Scope => Some(Menu::Channels {
                    parts: labels.assignments,
                    focus: labels.channel,
                }),
                View::Channel => Some(Menu::Instruments(labels.focus)),
                _ => None,
            }
        } else {
            None
        };
        if self.selector.select(menu) {
            // Restore the palette and any tiles previously covered by the list.
            self.plot.invalidate();
            self.page = None;
        }
        if let Some(page) = labels.seq.page {
            return self
                .paint_sequencer(lcd, clock, page, labels.seq, labels.focus, motion.position)
                .await;
        }
        if labels.view.is_mixer() {
            return self.paint_mixer(lcd, clock, labels, motion.peaks).await;
        }
        if labels.view == View::Envelope {
            match labels.focus {
                Part::Bass => return self.paint_bass_envelope(lcd, clock, labels.bass).await,
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
                Part::Tr808 | Part::Tr909 => (),
            }
        }
        let fx = labels.view == View::Fx;
        let bass = !fx && labels.focus == Part::Bass;
        let organ = !fx && labels.focus == Part::Organ;
        let drums = !fx && labels.focus.is_kit();
        let (style, ink, muted, bright) = if bass {
            (SILVER, OXIDE_RED, BASS_MUTED, SILVER.trace)
        } else if labels.focus == Part::Tr808 && drums {
            (
                drums::STYLE_808,
                drums::HOT,
                drums::CREAM,
                drums::STYLE_808.trace,
            )
        } else if drums {
            (
                drums::STYLE,
                drums::ORANGE,
                drums::GRAPHITE,
                drums::STYLE.trace,
            )
        } else if organ {
            (ORGAN_STYLE, ORGAN_LIME, ORGAN_MUTED, palette::CYAN)
        } else {
            (STYLE, palette::GREEN, palette::MUTED, palette::WHITE)
        };
        let style = self.selector.style(style);
        let [ink, muted, bright] = [ink, muted, bright].map(|color| self.selector.shade(color));
        let page = if fx {
            Page::Fx(labels.fx_page)
        } else {
            Page::Scope(labels.channel, labels.focus)
        };
        let changed_page = self.page.replace(page) != Some(page);
        if changed_page {
            self.change_view(style);
        }
        // Keep traces, their halos and the plot's eight-pixel dirty tiles clear
        // of the controls. Two footer rows begin at LCD y=208.
        let trace = if drums {
            // LCD y=60..179, including the halo and dirty tiles within y=56..183.
            frame.trace.map(|y| (36 + u16::from(y) * 5 / 8) as u8)
        } else if bass {
            frame.trace.map(|y| (25 + u16::from(y) * 3 / 4) as u8)
        } else if organ || fx {
            // Preserve the center at y=120; the bottom tile ends at y=207.
            frame.trace.map(|y| (12 + u16::from(y) * 7 / 8) as u8)
        } else {
            frame.trace
        };
        // While browsing, compose moving trace pixels and the list in the same LCD write.
        if changed_page || menu.is_none() {
            self.plot
                .paint_async(lcd, &trace.map(|y| y + TOP as u8), clock)
                .await?;
        }
        if changed_page {
            let (header, footer) = if drums {
                (56, 56)
            } else if bass || organ || fx {
                (if bass { 48 } else { 16 }, 32)
            } else {
                (16, 16)
            };
            for (y, height) in [(0, header), (SIZE - footer, footer)] {
                lcd.fill_rect(0, y, SIZE, height, style.background.into_storage())?;
            }
        }
        let pulse_track = if fx { Part::Mono } else { labels.focus };
        self.paint_beam(
            lcd,
            clock,
            &trace,
            style,
            pulse_track,
            motion.position.map(|position| position.beat_phase),
        )
        .await?;
        if fx {
            let enabled = labels.fx.enabled(labels.fx_page);
            self.text.text(
                0,
                0,
                match labels.fx_page {
                    FxPage::Reverb => b"FX / REVERB".as_slice(),
                    FxPage::Delay => b"FX / DELAY".as_slice(),
                    FxPage::BassComp => b"303 / COMP".as_slice(),
                },
                ink,
            );
            self.text
                .text(12, 0, if enabled { b"ON " } else { b"OFF" }, bright);
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
                    self.text
                        .number(17, 13, 3, u32::from(labels.fx.reverb.wet), ink);
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
                    self.text
                        .number(17, 14, 3, u32::from(labels.fx.delay.wet), ink);
                }
                FxPage::BassComp => {
                    self.text.text(0, 13, b"1 KEY", muted);
                    self.text.text(6, 13, labels.fx.key.name(), ink);
                    self.text.text(0, 14, b"2 DEP", muted);
                    self.text.number(5, 14, 3, u32::from(labels.fx.depth), ink);
                    self.text.text(8, 14, b"%", muted);
                    self.text.text(10, 14, b"3 EQ", muted);
                    self.text
                        .number(14, 14, 4, u32::from(labels.fx.split_hz), ink);
                    self.text.text(18, 14, b"HZ", muted);
                }
            }
            return self.text.paint_async(lcd, clock).await;
        }
        if drums {
            self.drums
                .paint(
                    lcd,
                    clock,
                    self.selector,
                    changed_page,
                    labels.focus,
                    motion.drums[labels.channel.index()],
                )
                .await?;
            let status = if !labels.seq.transport.playing() {
                "STOP"
            } else if labels.seq.enabled[labels.channel.index()] {
                "PLAY"
            } else {
                "MUTE"
            };
            let source = if labels.voice_tap { b"KIT" } else { b"DAC" };
            if labels.focus == Part::Tr808 {
                self.drums
                    .controls(lcd, self.selector, labels.drums808, changed_page)?;
                self.drums.status808(
                    lcd,
                    self.selector,
                    (labels.channel, labels.voice_tap, status),
                    changed_page,
                )?;
            } else {
                self.text
                    .number(12, 1, 3, u32::from(labels.seq.transport.bpm), muted);
                self.text.text(16, 1, b"BPM", muted);
                self.text.text(1, 2, b"CH", muted);
                self.text
                    .number(4, 2, 1, u32::from(labels.channel as u8) + 1, muted);
                self.text.text(7, 2, source, muted);
                self.text.text(15, 2, status.as_bytes(), ink);
            }
            return self.text.paint_async(lcd, clock).await;
        }
        let part = labels.focus;
        let voice = labels.voice;
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
            self.text.number(4, 0, 5, u32::from(voice.cutoff_hz()), ink);
            self.text.text(9, 0, b"HZ", muted);
        }
        self.text.text(
            12,
            0,
            if labels.half_gain { b"HALF" } else { b"FULL" },
            muted,
        );
        let source = match (labels.voice_tap, part) {
            (false, _) => b"DAC",
            (true, Part::Mono) => b"VCA",
            (true, Part::Organ) => b"ORG",
            (true, Part::Bass) => b"SRC",
            (true, Part::Tr808 | Part::Tr909) => b"KIT",
        };
        self.text.text(16, 0, source, muted);
        self.text.text(0, 14, voice.label(part), ink);
        self.text.text(8, 14, voice.note_name(), bright);
        self.text.text(16, 14, b"V", muted);
        let velocity = voice.gate.note.map_or(0, |note| u32::from(note.velocity));
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
fn phosphor(track: Part, age: usize) -> (Rgb565, Option<Rgb565>) {
    if track.is_kit() {
        return match age {
            0..=3 => (drums::HOT, Some(drums::GLOW)),
            4..=12 => (drums::ORANGE, None),
            13..TRAIL => (drums::EMBER, None),
            _ => (drums::style(track).trace, None),
        };
    }
    if track == Part::Bass {
        return match age {
            0..=3 => (Rgb565::new(24, 10, 3), Some(Rgb565::new(21, 34, 16))),
            4..=9 => (OXIDE_RED, None),
            10..TRAIL => (Rgb565::new(8, 9, 5), None),
            _ => (SILVER.trace, None),
        };
    }
    if track == Part::Organ {
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
        track: Part,
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
                    let (ink, halo) = phosphor(track, age(h, x));
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
