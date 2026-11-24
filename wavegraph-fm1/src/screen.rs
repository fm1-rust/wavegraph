//! Scope and envelope pages for the FM-1 display.

use crate::overload::Severity;
use crate::{
    bass::{Controls, Knob, envelope as bass_env},
    capture::{HEIGHT, WIDTH},
    envelope::{EnvelopeControls, EnvelopeTarget},
    firmware::{Frame, Labels, Motion, Timing},
    instrument::Part,
    panel::{CHANNELS, Channel, View, preset_age},
    scope::{Damage, GRID, Palette, Pen, Sweep, TRAIL},
    sequencer::{BankPosition, Position, SeqPage, Sequencer},
};
use core::{
    future::{Future, poll_fn},
    ops::Range,
    pin::{Pin, pin},
    task::{Context, Poll},
};
use fm1_graphics::embedded_graphics::pixelcolor::{IntoStorage, raw::ToBytes};
use fm1_graphics::{Rgb565, WaveformStyle, palette};
use fm1_rticx::{
    bsp::{
        display::{Display, Error, SIZE},
        time::ticks,
    },
    time::Clock,
};
use static_cell::{ConstStaticCell, StaticCell};

mod arp;
mod badge;
mod channel_fx;
mod drums;
mod envelope_plot;
mod grid;
mod jp8080;
mod lettering;
use grid::TextGrid;
mod master_fx;
mod microwave;
mod mixer;
mod selector;
mod sequencer;
mod splash;
mod tb303;
mod tr808;

use envelope_plot::Plot;
use selector::{Menu, Selector};

const TOP: u16 = (SIZE - HEIGHT as u16) / 2;
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
type Lane = Plot<{ bass_env::WIDTH }, { bass_env::HEIGHT as usize }>;

const BAND: usize = 8;

// Keep the envelope DMA state machine out of paint_page's poll function.
// The combined function exceeds the pi32v2 conditional branch range.
#[inline(never)]
fn poll_envelope<F: Future>(page: Pin<&mut F>, cx: &mut Context<'_>) -> Poll<F::Output> {
    page.poll(cx)
}

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
    Timing,
    Scope(Channel, Part),
    Arp(Channel, Part),
    Fx(Channel, Part),
    MasterFx,
    Mixer,
    Sequencer(SeqPage, Channel, Part),
    Envelope(Part, EnvelopeTarget, EnvelopeControls, bool),
    BassEnvelope(Controls, Option<Knob>),
}

pub(super) struct Screen {
    plot: &'static mut Plot<WIDTH, WIDTH>,
    lanes: &'static mut [Lane; 3],
    text: &'static mut TextGrid,
    strip: &'static mut [[u8; 2]; BAND * WIDTH],
    selector: &'static mut Selector,
    drums: drums::Panel,
    bass: tb303::Panel,
    jp: jp8080::Panel,
    wave: microwave::Panel,
    mixer: mixer::Surface,
    rack: master_fx::Rack,
    channel_fx: channel_fx::Rack,
    arp: arp::Panel,
    sequence: sequencer::Grid,
    sweep: Option<Sweep>,
    scope_palette: Option<Palette>,
    dot: Option<Severity>,
    dot_background: Rgb565,
    page: Option<Page>,
}

impl Screen {
    pub(super) fn new() -> Self {
        static PLOT: ConstStaticCell<Plot<WIDTH, WIDTH>> = ConstStaticCell::new(Plot::new());
        static LANES: ConstStaticCell<[Lane; 3]> = ConstStaticCell::new([const { Plot::new() }; 3]);
        // Initialize the overlay buffers in SRAM instead of storing empty pixels in flash.
        static SELECTOR: StaticCell<Selector> = StaticCell::new();
        static TEXT: StaticCell<TextGrid> = StaticCell::new();
        static STRIP: ConstStaticCell<[[u8; 2]; BAND * WIDTH]> =
            ConstStaticCell::new([[0; 2]; BAND * WIDTH]);
        Self {
            plot: PLOT.take(),
            lanes: LANES.take(),
            text: TEXT.init_with(|| TextGrid::with_background(STYLE.background)),
            strip: STRIP.take(),
            selector: SELECTOR.init_with(Selector::new),
            drums: drums::Panel::default(),
            bass: tb303::Panel::default(),
            jp: jp8080::Panel::default(),
            wave: microwave::Panel::default(),
            mixer: mixer::Surface::default(),
            rack: master_fx::Rack::default(),
            channel_fx: channel_fx::Rack::default(),
            arp: arp::Panel::default(),
            sequence: sequencer::Grid::new(),
            page: None,
            dot: None,
            dot_background: STYLE.background,
            sweep: None,
            scope_palette: None,
        }
    }

    fn change_view(&mut self, style: WaveformStyle) {
        self.selector.badge.change_view(style.background);
        self.dot = None;
        self.dot_background = style.background;
        // A new page can reuse the same colors over different artwork.
        self.plot.invalidate();
        *self.text = TextGrid::with_background(style.background);
        self.sweep = None;
        self.scope_palette = None;
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
        target: EnvelopeTarget,
        controls: EnvelopeControls,
        show_mode_hint: bool,
    ) -> Result<(), Error> {
        let page = Page::Envelope(focus, target, controls, show_mode_hint);
        let previous = self.page.replace(page);
        if previous != Some(page) {
            let entering = !matches!(previous, Some(Page::Envelope(part, ..)) if part == focus);
            let rack = matches!(focus, Part::Supersaw | Part::Wavetable);
            let (style, title, ink) = if focus == Part::Organ {
                (
                    WaveformStyle {
                        trace: ORGAN_LIME,
                        ..ORGAN_STYLE
                    },
                    b"ORGAN VCA ENV".as_slice(),
                    ORGAN_LIME,
                )
            } else if focus == Part::Supersaw {
                (jp8080::STYLE, b"".as_slice(), jp8080::AMBER)
            } else if focus == Part::Wavetable {
                (microwave::STYLE, b"".as_slice(), microwave::LIME)
            } else {
                (ENV_STYLE, b"VCA ENVELOPE".as_slice(), palette::GREEN)
            };
            let (face, label, unit, header) = if focus == Part::Supersaw {
                (jp8080::FACE, jp8080::PRINT, jp8080::EDGE, 32)
            } else if focus == Part::Wavetable {
                (microwave::FACE, microwave::PRINT, microwave::PRINT, 32)
            } else {
                (style.background, palette::WHITE, palette::MUTED, 16)
            };
            if entering {
                self.change_view(style);
                if rack {
                    self.dot_background = face;
                    *self.text = TextGrid::with_background(face);
                }
            }
            self.plot
                .paint(lcd, clock, self.strip, (0, 0), style, &controls.trace())
                .await?;
            if entering {
                // The curve's dirty tiles stay between the two label bands.
                let mut lcd = lcd.as_async(clock)?;
                lcd.fill_rect(0, 0, SIZE, header, face.into_storage())
                    .await?;
                lcd.fill_rect(0, 192, SIZE, SIZE - 192, face.into_storage())
                    .await?;
            }
            if entering
                || !matches!(previous, Some(Page::Envelope(_, old, _, hint)) if old == target && hint == show_mode_hint)
            {
                match focus {
                    Part::Wavetable => {
                        microwave::envelope_header(lcd, clock, self.strip, target, show_mode_hint)
                            .await?
                    }
                    Part::Supersaw => {
                        jp8080::envelope_header(lcd, clock, self.strip, target, entering).await?
                    }
                    _ => self.text.text(0, 0, title, ink),
                }
            }
            let adsr = controls.adsr();
            let colors = [label, ink, unit];
            env_time(self.text, 0, 12, b"A", adsr.attack_us, colors);
            env_time(self.text, 10, 12, b"D", adsr.decay_us, colors);
            self.text.text(0, 14, b"S", label);
            self.text
                .number(1, 14, 4, u32::from(adsr.sustain_percent), ink);
            self.text.text(5, 14, b"/100", unit);
            env_time(self.text, 10, 14, b"R", adsr.release_us, colors);
        }
        self.text.paint_async(lcd, clock).await
    }

    fn bass_envelope_labels(&mut self, controls: Controls, selected: Option<Knob>) {
        let (ink, muted) = (
            self.selector.shade(OXIDE_RED),
            self.selector.shade(BASS_MUTED),
        );
        self.text.text(0, 0, b"TB-303", ink);
        self.text
            .text(7, 0, if selected.is_some() { b"ALG" } else { b"   " }, ink);
        let focus = selected.unwrap_or(controls.last);
        for (i, &knob) in Knob::direct().iter().enumerate() {
            let (x, width) = if i % 2 == 0 { (0, 7) } else { (9, 9) };
            let y = 1 + i / 2;
            let color = if knob == focus { ink } else { muted };
            self.text.text(x, y, [b"1234"[i]], ink);
            self.text.text(x + 2, y, &knob.name()[..width], color);
        }
        let (name, value, units) = controls.readout(focus);
        self.text
            .text(0, 13, name, if selected.is_some() { ink } else { muted });
        if focus == Knob::Tuning {
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
        selected: Option<Knob>,
    ) -> Result<(), Error> {
        let page = Page::BassEnvelope(controls, selected);
        let previous = self.page.replace(page);
        if previous != Some(page) {
            if !matches!(previous, Some(Page::BassEnvelope(..))) {
                self.change_view(SILVER);
                lcd.as_async(clock)?
                    .fill_rect(0, 0, SIZE, SIZE, SILVER.background.into_storage())
                    .await?;
                for lane in &mut *self.lanes {
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
            if !matches!(previous, Some(Page::BassEnvelope(old, _)) if old == controls) {
                for (i, (lane, trace)) in self
                    .lanes
                    .iter_mut()
                    .zip(&bass_env::traces(controls))
                    .enumerate()
                {
                    let style = WaveformStyle {
                        trace: LANE_INKS[i],
                        grid_y: 0,
                        center: SILVER.grid,
                        ..SILVER
                    };
                    lane.paint(
                        lcd,
                        clock,
                        self.strip,
                        (LANE_X, [64, 112, 160][i]),
                        style,
                        trace,
                    )
                    .await?;
                }
            }
            self.bass_envelope_labels(controls, selected);
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
        playback: (Option<Position>, BankPosition),
    ) -> Result<(), Error> {
        let (position, bank) = playback;
        let style = sequencer::style(part);
        let ink = sequencer::ink(part);
        let muted = if part == Part::Tr909 {
            drums::GRAPHITE
        } else {
            palette::MUTED
        };
        let green = if part == Part::Tr909 {
            Rgb565::new(3, 25, 7)
        } else {
            palette::GREEN
        };
        let page = Page::Sequencer(view, seq.channel, part);
        if self.page.replace(page) != Some(page) {
            self.change_view(style);
            self.selector
                .badge
                .fill_rows(&mut lcd.as_async(clock)?, 0..SIZE, style.background)
                .await?;
            self.text.text(
                12,
                0,
                [b"C1", b"C2", b"C3", b"C4"][seq.channel.index()],
                ink,
            );
            if view != SeqPage::Tempo {
                self.text.text(4, 2, b"1   2   3   4", muted);
                self.text.text(15, 0, b"B", muted);
                self.text.text(16, 1, b"S", muted);
            } else {
                self.text.text(0, 4, b"TEMPO 2/2", muted);
                self.text.text(0, 6, b"MASTER TEMPO", ink);
                self.text.text(0, 8, b"KNOB 3", green);
                self.text.text(0, 14, b"SELECT FOR PATTERN", muted);
            }
            self.text.text(8, 1, b"BPM", muted);
        }
        let title: &[u8] = match part {
            Part::Mono => b"SEQ MONO",
            Part::Organ => b"SEQ M1 ORG",
            Part::Bass if seq.channel == Channel::Two => b"SEQ 303 HI",
            Part::Bass => b"SEQ 303 LOW",
            Part::Tr909 => b"SEQ TR-909",
            Part::Tr808 => b"SEQ TR-808",
            Part::Supersaw => b"SEQ JP-8080",
            Part::Wavetable => b"SEQ WAVE",
        };
        self.selector.badge.header_text(self.text, 0, title, ink);
        self.selector.badge.header_text(
            self.text,
            1,
            if view == SeqPage::Record {
                b"REC "
            } else if view == SeqPage::Step {
                b"EDIT"
            } else if seq.transport.playing() {
                b"PLAY"
            } else {
                b"STOP"
            },
            if view == SeqPage::Record {
                palette::RED
            } else if seq.transport.playing() {
                green
            } else {
                palette::RED
            },
        );
        self.text.number(5, 1, 3, u32::from(seq.transport.bpm), ink);
        let enabled = seq.enabled[seq.channel.index()];
        self.text.text(
            12,
            1,
            if enabled { b"ON " } else { b"OFF" },
            if enabled { green } else { palette::MUTED },
        );
        if view != SeqPage::Tempo {
            self.sequence
                .paint(
                    lcd,
                    clock,
                    self.text,
                    self.strip,
                    (seq, part),
                    position,
                    bank,
                )
                .await?;
        }
        self.text.paint_async(lcd, clock).await
    }

    pub(super) async fn timing(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        t: Timing,
    ) -> Result<(), Error> {
        // These leaf routines only read the linker-bounded, painted CPU0 stacks.
        unsafe extern "C" {
            safe fn wavegraph_usp_free() -> u32;
            safe fn wavegraph_ssp_free() -> u32;
        }
        if self.page.replace(Page::Timing) != Some(Page::Timing) {
            self.selector.select(None);
            self.change_view(STYLE);
            lcd.as_async(clock)?
                .fill_rect(0, 0, SIZE, SIZE, STYLE.background.into_storage())
                .await?;
        }
        let title = match t.boot {
            0 => "CPU1 BOOT TIMEOUT   ",
            1 if t.starving => "AUDIO STARVATION    ",
            1 => "CPU0 UI / CPU1 DSP  ",
            2 => "CPU1 BAD CONTEXT    ",
            3 => "CPU1 BAD REPLY      ",
            4 => "CPU1 FAULT          ",
            5 => "CPU1 STALLED        ",
            _ => "CPU1 UNKNOWN STATUS ",
        };
        self.text.text(
            0,
            0,
            title,
            if t.boot == 1 && !t.starving {
                palette::GREEN
            } else {
                palette::RED
            },
        );
        // Peak request-to-ready time, then minimum slack on ordinary and scanned blocks.
        self.text.text(0, 1, "R", palette::MUTED);
        self.text
            .number(1, 1, 5, t.ready.div_ceil(24).min(99_999), palette::GREEN);
        for (x, label, value) in [(7, "N", t.margin), (14, "W", t.scan_margin)] {
            self.text.text(x, 1, label, palette::MUTED);
            if let Some(ticks) = value {
                let color = if ticks < 0 {
                    palette::RED
                } else {
                    palette::GREEN
                };
                // Floor toward a miss, including a result one TIMER4 tick late.
                self.text.signed_number(
                    x + 1,
                    1,
                    5,
                    ticks.div_euclid(24).clamp(-9999, 9999),
                    color,
                );
            } else {
                self.text.text(x + 1, 1, "    -", palette::MUTED);
            }
        }
        for (i, (label, value)) in [
            ("USP FREE B", wavegraph_usp_free()),
            ("SSP FREE B", wavegraph_ssp_free()),
            ("REFILL US", t.refill.div_ceil(24)),
            ("PLAY US", t.playing_refill.div_ceil(24)),
            ("WRITE US", t.commit.div_ceil(24)),
            ("ALNK US", t.irq.div_ceil(24)),
            ("WORKER US", t.worker.div_ceil(24)),
            ("OVERRUNS", t.overruns),
            ("SILENCES", t.silences),
            ("DISCARDS", t.discards),
            ("DROP BLOCK", t.last_discard),
            ("EDIT BLOCK", t.last_edit),
        ]
        .into_iter()
        .enumerate()
        {
            self.text.text(0, 2 + i, label, palette::MUTED);
            self.text.number(10, 2 + i, 10, value, palette::GREEN);
        }
        self.text.text(0, 14, "C1 FREE B", palette::MUTED);
        if let Some(bytes) = t.worker_stack {
            self.text.number(10, 14, 10, bytes, palette::GREEN);
        } else {
            self.text.text(10, 14, "         -", palette::GREEN);
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
            let mut lcd = lcd.as_async(clock)?;
            lcd.write_rect_buffered(SIZE - 10, 4, 7, 7, pixels.as_flattened().as_flattened())
                .await?;
            lcd.flush().await?;
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
        let elapsed = labels
            .preset_since
            .and_then(|since| preset_age(since, ticks()));
        self.selector
            .badge
            .select(
                lcd,
                clock,
                self.strip,
                self.text,
                (labels.view == View::Scope && elapsed.is_some()).then_some(labels.channel),
            )
            .await?;
        let menu = if elapsed.is_some_and(|age| age < MENU_TICKS) {
            match labels.view {
                View::Scope => Some(Menu::Channels {
                    parts: labels.assignments,
                    focus: labels.channel,
                }),
                View::Channel => Some(Menu::Instruments(labels.focus)),
                View::Fx => labels
                    .insert_slot
                    .map(|slot| Menu::Effects(labels.inserts[usize::from(slot)].kind())),
                _ => None,
            }
        } else {
            None
        };
        if self.selector.select(menu) {
            // Repaint the instrument with the new palette and list mask.
            self.page = None;
        }
        if let Some(page) = labels.seq.page {
            return self
                .paint_sequencer(
                    lcd,
                    clock,
                    page,
                    labels.seq,
                    labels.focus,
                    (motion.position, motion.banks[labels.seq.channel.index()]),
                )
                .await;
        }
        if labels.view == View::Arp {
            let page = Page::Arp(labels.channel, labels.focus);
            let entering = self.page.replace(page) != Some(page);
            if entering {
                self.change_view(WaveformStyle {
                    background: arp::Colors::for_part(labels.focus).face,
                    ..STYLE
                });
            }
            return self
                .arp
                .paint(
                    lcd,
                    clock,
                    self.strip,
                    &labels,
                    motion.arp_notes[labels.channel.index()],
                    entering,
                )
                .await;
        }
        if let View::MasterFx(selected) = labels.view {
            if self.page.replace(Page::MasterFx) != Some(Page::MasterFx) {
                self.change_view(WaveformStyle {
                    background: mixer::BACKGROUND,
                    ..STYLE
                });
                self.rack = master_fx::Rack::default();
            }
            return self
                .rack
                .paint(
                    lcd,
                    clock,
                    self.strip,
                    (labels.fx, selected),
                    motion.reduction,
                    &mut self.selector.badge,
                )
                .await;
        }
        if labels.view.is_mixer() {
            return self.paint_mixer(lcd, clock, labels, motion.peaks).await;
        }
        let head = motion
            .position
            .map(|p| usize::from(p.beat_phase) * WIDTH / 256);
        if labels.view == View::Fx {
            let page = Page::Fx(labels.channel, labels.focus);
            let entering = self.page.replace(page) != Some(page);
            let style = self.selector.style(STYLE);
            if entering {
                self.change_view(style);
                self.dot_background = self.selector.shade(mixer::BACKGROUND);
            }
            self.channel_fx
                .paint(lcd, clock, self.strip, &labels, entering, self.selector)
                .await?;
            let rows = channel_fx::SCOPE;
            let height = rows.end - rows.start;
            // Two pixels of margin keep the phosphor halo inside the plot at full scale.
            let trace = frame
                .trace
                .map(|y| (rows.start + 2 + u16::from(y) * (height - 4) / HEIGHT as u16) as u8);
            let center = rows.start + height / 2;
            return self
                .paint_scope(
                    lcd,
                    clock,
                    Sweep { trace, head },
                    style,
                    Part::Mono,
                    (rows, center),
                )
                .await;
        }
        if labels.view == View::Envelope {
            match labels.focus {
                Part::Bass => {
                    return self
                        .paint_bass_envelope(lcd, clock, labels.bass, labels.bass_knob)
                        .await;
                }
                Part::Mono | Part::Organ | Part::Supersaw | Part::Wavetable => {
                    let (target, controls) = match labels.focus {
                        Part::Organ => (EnvelopeTarget::Amp, labels.organ.envelope),
                        Part::Supersaw if labels.envelope_target == EnvelopeTarget::Modulation => {
                            (labels.envelope_target, labels.saw.filter_envelope)
                        }
                        Part::Wavetable if labels.envelope_target == EnvelopeTarget::Modulation => {
                            (labels.envelope_target, labels.wave.position_envelope)
                        }
                        _ => (EnvelopeTarget::Amp, labels.voice_envelope),
                    };
                    let show_mode_hint = labels.focus == Part::Wavetable
                        && target == EnvelopeTarget::Modulation
                        && labels.wave.scan.mode != wavegraph_wavetable::Mode::Envelope;
                    let mut page = pin!(self.paint_envelope(
                        lcd,
                        clock,
                        labels.focus,
                        target,
                        controls,
                        show_mode_hint
                    ));
                    return poll_fn(|cx| poll_envelope(page.as_mut(), cx)).await;
                }
                Part::Tr808 | Part::Tr909 => (),
            }
        }
        let bass = labels.focus == Part::Bass;
        let organ = labels.focus == Part::Organ;
        let drums = labels.focus.is_kit();
        let saw = labels.focus == Part::Supersaw;
        let wave = labels.focus == Part::Wavetable;
        let (style, ink, muted, bright) = if bass {
            (SILVER, OXIDE_RED, BASS_MUTED, SILVER.trace)
        } else if saw {
            (
                jp8080::STYLE,
                jp8080::AMBER,
                jp8080::EDGE,
                jp8080::STYLE.trace,
            )
        } else if wave {
            (
                microwave::STYLE,
                microwave::PRINT,
                microwave::FACE,
                microwave::STYLE.trace,
            )
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
        let page = Page::Scope(labels.channel, labels.focus);
        let changed_page = self.page.replace(page) != Some(page);
        if changed_page {
            self.change_view(style);
        }
        // Leave room for the halo above and below the connected trace.
        let trace = if drums {
            frame.trace.map(|y| (60 + u16::from(y) * 5 / 8) as u8)
        } else if saw || wave {
            frame.trace.map(|y| (44 + u16::from(y) * 4 / 5) as u8)
        } else if bass {
            frame
                .trace
                .map(|y| (tb303::SCOPE_TOP + 2 + u16::from(y) * 9 / 10) as u8)
        } else if organ {
            frame.trace.map(|y| (36 + u16::from(y) * 7 / 8) as u8)
        } else {
            frame.trace.map(|y| y + TOP as u8)
        };
        let (header, footer) = if drums {
            (56, 56)
        } else if bass {
            (tb303::SCOPE_TOP, 0)
        } else if saw || wave {
            (32, 40)
        } else if organ {
            (16, 32)
        } else {
            (16, 16)
        };
        if changed_page {
            if bass {
                self.dot_background = self.selector.shade(tb303::METAL);
            } else if saw {
                self.dot_background = self.selector.shade(jp8080::FACE);
            } else if wave {
                self.dot_background = self.selector.shade(microwave::FACE);
            }
            let mut lcd = lcd.as_async(clock)?;
            for (y, height) in [(0, header), (SIZE - footer, footer)] {
                self.selector
                    .badge
                    .fill_rows(&mut lcd, y..y + height, self.dot_background)
                    .await?;
            }
            if bass {
                self.selector
                    .badge
                    .fill_rows(&mut lcd, header - 1..header, muted)
                    .await?;
            }
        }
        let sweep = Sweep { trace, head };
        let center = if bass {
            header + (SIZE - header) / 2
        } else {
            SIZE / 2
        };
        self.paint_scope(
            lcd,
            clock,
            sweep,
            style,
            labels.focus,
            (header..SIZE - footer, center),
        )
        .await?;
        if bass {
            let automation = motion
                .bass_automation
                .filter(|(channel, _)| *channel == labels.channel)
                .map(|(_, values)| values);
            let mut lcd = lcd.as_async(clock)?;
            return self
                .bass
                .paint(
                    &mut lcd,
                    self.strip,
                    self.selector,
                    &labels,
                    automation,
                    changed_page,
                )
                .await;
        }
        if wave {
            let mut lcd = lcd.as_async(clock)?;
            return self
                .wave
                .paint(
                    &mut lcd,
                    self.strip,
                    self.selector,
                    &labels,
                    motion.wave_frames[labels.channel.index()],
                    changed_page,
                )
                .await;
        }
        if saw {
            return self
                .jp
                .paint(lcd, clock, self.strip, self.selector, labels, changed_page)
                .await;
        }
        if drums {
            let mut display = lcd.as_async(clock)?;
            self.drums
                .paint(
                    &mut display,
                    self.strip,
                    self.selector,
                    changed_page,
                    labels.focus,
                    motion.drums[labels.channel.index()],
                )
                .await?;
            let status = transport_status(&labels);
            let source = if labels.voice_tap { b"KIT" } else { b"DAC" };
            if labels.focus == Part::Tr808 {
                self.drums
                    .controls(
                        &mut display,
                        self.strip,
                        self.selector,
                        labels.drums808,
                        changed_page,
                    )
                    .await?;
                self.drums
                    .status808(
                        &mut display,
                        self.strip,
                        self.selector,
                        (labels.channel, labels.voice_tap, status),
                        changed_page,
                    )
                    .await?;
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
            display.flush().await?;
            drop(display);
            return self.text.paint_async(lcd, clock).await;
        }
        let part = labels.focus;
        let voice = labels.voice;
        if organ {
            self.selector
                .badge
                .header_text(self.text, 0, b"M1 ORGAN", ink);
            let voices = labels
                .poly_keys
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
            self.selector.badge.header_text(self.text, 0, b"CUT", muted);
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
            (true, Part::Bass | Part::Supersaw | Part::Wavetable) => b"SRC",
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

fn transport_status(labels: &Labels) -> &'static str {
    if !labels.seq.transport.playing() {
        "STOP"
    } else if labels.seq.enabled[labels.channel.index()] {
        "PLAY"
    } else {
        "MUTE"
    }
}

fn env_time(
    text: &mut TextGrid,
    x: usize,
    y: usize,
    label: &[u8],
    us: u32,
    [print, ink, muted]: [Rgb565; 3],
) {
    let (value, unit) = if us != 0 && us.is_multiple_of(1_000_000) {
        (us / 1_000_000, b"S ")
    } else if us.is_multiple_of(1000) {
        (us / 1000, b"MS")
    } else {
        (us, b"US")
    };
    text.text(x, y, label, print);
    text.number(x + 1, y, 5, value, ink);
    text.text(x + 6, y, unit, muted);
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
    if track == Part::Wavetable {
        return match age {
            0..=3 => (palette::WHITE, Some(microwave::STYLE.trace)),
            4..=12 => (microwave::PRINT, None),
            _ => (microwave::STYLE.trace, None),
        };
    }
    if track == Part::Supersaw {
        return match age {
            0..=3 => (palette::WHITE, Some(jp8080::EDGE)),
            4..=12 => (Rgb565::new(24, 63, 28), None),
            _ => (jp8080::STYLE.trace, None),
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
    async fn paint_scope(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        sweep: Sweep,
        style: WaveformStyle,
        track: Part,
        (rows, center): (Range<u16>, u16),
    ) -> Result<(), Error> {
        if self.sweep.as_ref() == Some(&sweep) {
            return Ok(());
        }
        let list = &mut *self.selector;
        let palette = self.scope_palette.get_or_insert_with(|| {
            Palette::new(
                style.background.to_be_bytes(),
                style.grid.to_be_bytes(),
                style.center.to_be_bytes(),
                core::array::from_fn(|age| {
                    let (ink, halo) = phosphor(track, age);
                    Pen {
                        ink: list.shade(ink).to_be_bytes(),
                        halo: halo.map(|color| list.shade(color).to_be_bytes()),
                    }
                }),
            )
        });
        let damage = if let Some(old) = self.sweep.take() {
            use crate::scope::Stripes;
            let stripes = match track {
                Part::Bass | Part::Organ | Part::Supersaw => Stripes::Fine,
                Part::Tr808 | Part::Tr909 => Stripes::Wide,
                Part::Mono | Part::Wavetable => Stripes::Normal,
            };
            sweep.damage(&old, palette, stripes)
        } else {
            Damage::full(
                usize::from(rows.start)..usize::from(rows.end),
                list.bounds(),
            )
        };
        let mut lcd = lcd.as_async(clock)?;
        for window in damage.windows() {
            for w in window.strips(self.strip.len()) {
                let pixels = &mut self.strip[..w.width * w.height];
                sweep.render(w, palette, usize::from(center), pixels);
                list.overlay(w, pixels);
                list.badge.compose(w.x, w.y, w.width, pixels);
                lcd.write_rect_buffered(
                    w.x as u16,
                    w.y as u16,
                    w.width as u16,
                    w.height as u16,
                    pixels.as_flattened(),
                )
                .await?;
            }
        }
        lcd.flush().await?;
        self.sweep = Some(sweep);
        Ok(())
    }
}
