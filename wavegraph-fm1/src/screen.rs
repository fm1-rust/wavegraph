//! Scope and envelope pages for the FM-1 display.

use crate::overload::Severity;
use crate::{
    bass::{Control, Controls, Knob, envelope as bass_env, presets::PRESETS},
    capture::{HEIGHT, WIDTH},
    envelope::{EnvelopeControls, EnvelopeTarget},
    firmware::{Frame, Labels, Motion, Timing},
    instrument::Part,
    panel::{Channel, PresetRole, View, preset_age},
    scope::{Damage, GRID, Palette, Pen, Sweep},
    sequencer::{BankPosition, Position, SeqPage},
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
mod beat_fx;
mod channel_fx;
mod drums;
mod envelope_plot;
mod fm;
mod grid;
mod hoover;
mod jp8080;
mod lettering;
mod lfo;
mod polyphony;
use grid::TextGrid;
mod master_fx;
mod microwave;
mod mixer;
mod sampler;
mod selector;
mod sequencer;
mod splash;
mod style;
use style::InstrumentStyle;
mod tb303;
mod tr808;
mod viewport;
use viewport::{PanelDisplay, Viewport};

use envelope_plot::Plot;
use selector::{Menu, Selector};

const TOP: u16 = (SIZE - HEIGHT as u16) / 2;
// The timing monitor uses the original green CRT.
const STYLE: WaveformStyle = style::MONO.scope;
const MENU_TICKS: u32 = 100;
const LANE_X: u16 = 42;
type Lane = Plot<{ bass_env::WIDTH }, { bass_env::HEIGHT as usize }>;

const BAND: usize = 8;

// Keep large panel DMA state machines out of paint_page's poll function.
// The combined function exceeds the pi32v2 conditional branch range.
#[inline(never)]
fn poll_page<F: Future>(page: Pin<&mut F>, cx: &mut Context<'_>) -> Poll<F::Output> {
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
    BeatFx(beat_fx::Panel),
    Clipboard(
        Channel,
        Part,
        crate::clipboard::Options,
        crate::clipboard::Status,
        bool,
    ),
    Scope(Channel, Part, wavegraph_organ::VoiceMode, u8),
    Arp(Channel, Part),
    VoiceMode(Channel, Part, wavegraph_organ::VoiceMode),
    Lfo(Channel, Part, crate::lfo::Controls),
    Fx(Channel, Part),
    MasterFx,
    Mixer,
    Sequencer(SeqPage, Channel, Part),
    Envelope(Channel, Part, EnvelopeTarget, EnvelopeControls, bool, i16),
    HooverEnvelope(
        Channel,
        wavegraph_hoover::Patch,
        crate::instrument::HooverPage,
    ),
    BassEnvelope(Channel, Controls, Option<Control>, u8, u8),
    DrumEnvelope(Channel, Part, drums::Envelope),
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
    fm: fm::Panel,
    sampler: sampler::Panel,
    mixer: mixer::Surface,
    rack: master_fx::Rack,
    channel_fx: channel_fx::Rack,
    arp: arp::Panel,
    sequence: sequencer::Grid,
    sweep: Option<Sweep>,
    scope_palette: Option<Palette>,
    badge: Option<badge::Mode>,
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
            fm: fm::Panel::default(),
            sampler: sampler::Panel::default(),
            mixer: mixer::Surface::default(),
            rack: master_fx::Rack::default(),
            channel_fx: channel_fx::Rack::default(),
            arp: arp::Panel::default(),
            sequence: sequencer::Grid::new(),
            page: None,
            badge: None,
            dot: None,
            dot_background: STYLE.background,
            sweep: None,
            scope_palette: None,
        }
    }

    fn change_view(&mut self, style: WaveformStyle) {
        self.dot = None;
        self.dot_background = style.background;
        // A new page can reuse the same colors over different artwork.
        self.plot.invalidate();
        *self.text = TextGrid::with_background(style.background);
        self.sweep = None;
        self.scope_palette = None;
        self.sequence.invalidate();
    }

    async fn paint_global(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        labels: Labels,
        motion: &Motion,
    ) -> Result<(), Error> {
        if labels.view == View::Clipboard {
            let page = Page::Clipboard(
                labels.channel,
                labels.focus,
                labels.clip_options,
                motion.clipboard,
                labels.seq.transport.playing(),
            );
            if self.page != Some(page) {
                self.change_view(WaveformStyle {
                    background: mixer::BACKGROUND,
                    ..STYLE
                });
                let mut draw = pin!(sampler::clipboard(
                    lcd,
                    clock,
                    self.strip,
                    &labels,
                    motion.clipboard
                ));
                poll_fn(|cx| poll_page(draw.as_mut(), cx)).await?;
                self.page = Some(page);
            }
            return Ok(());
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
                )
                .await;
        }
        if self.page.replace(Page::Mixer) != Some(Page::Mixer) {
            self.change_view(WaveformStyle {
                background: self.selector.shade(mixer::BACKGROUND),
                ..STYLE
            });
            self.mixer.reset();
        }
        self.mixer
            .paint(lcd, clock, self.strip, self.selector, labels, motion.peaks)
            .await
    }

    async fn paint_envelope(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        labels: &Labels,
        theme: &InstrumentStyle,
        target: EnvelopeTarget,
        controls: EnvelopeControls,
    ) -> Result<(), Error> {
        let focus = labels.focus;
        let show_mode_hint = focus == Part::Wavetable
            && target == EnvelopeTarget::Modulation
            && labels.wave.scan.mode != wavegraph_wavetable::Mode::Envelope;
        let cents = if target == EnvelopeTarget::Pitch {
            labels.fm.patch.pitch_cents
        } else {
            0
        };
        let page = Page::Envelope(
            labels.channel,
            focus,
            target,
            controls,
            show_mode_hint,
            cents,
        );
        let previous = self.page.replace(page);
        if previous != Some(page) {
            let entering = !matches!(previous, Some(Page::Envelope(ch, part, ..)) if (ch, part) == (labels.channel, focus));
            let style = theme.scope;
            let (face, label, ink, unit) = (theme.face, theme.ink, theme.accent, theme.muted);
            let header = if matches!(focus, Part::Organ | Part::Mono) {
                16
            } else {
                32
            };
            if entering {
                self.change_view(theme.panel());
            }
            let mut trace = controls.trace();
            if target == EnvelopeTarget::Pitch {
                // The center line is concert pitch. The signed depth labels the peak.
                for y in &mut trace {
                    let height = i16::from(172u8.saturating_sub(*y)) / 2;
                    *y = (120 - cents.signum() * height) as u8;
                }
            }
            self.plot
                .paint(lcd, clock, self.strip, (0, 0), style, &trace)
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
                || !matches!(previous, Some(Page::Envelope(_, _, old, _, hint, depth)) if old == target && hint == show_mode_hint && depth == cents)
            {
                match focus {
                    Part::Sampler => {
                        let title = if target == EnvelopeTarget::Amp {
                            b"SAMPLER AMP ENV"
                        } else {
                            b"SAMPLER VCF ENV"
                        };
                        self.text.text(0, 0, title, ink);
                        self.text.text(0, 1, b"SEL  AMP / FILTER", label);
                    }
                    Part::Fm => {
                        self.text.text(
                            0,
                            0,
                            match target {
                                EnvelopeTarget::Amp => b"FM OP1 CARRIER",
                                EnvelopeTarget::Modulation => b"FM OP2 MOD    ",
                                EnvelopeTarget::Pitch => b"FM PITCH      ",
                            },
                            ink,
                        );
                        self.text.text(0, 1, b"SEL NEXT           ", label);
                        if target == EnvelopeTarget::Pitch {
                            self.text.text(9, 1, b"ALG", ink);
                            self.text.signed_number(12, 1, 5, i32::from(cents), label);
                            self.text.text(17, 1, b"ct", label);
                        }
                    }
                    Part::Wavetable => {
                        microwave::envelope_header(lcd, clock, self.strip, target, show_mode_hint)
                            .await?
                    }
                    Part::Supersaw => {
                        jp8080::envelope_header(lcd, clock, self.strip, target, entering).await?
                    }
                    Part::Organ => self.text.text(0, 0, b"ORGAN VCA ENV", ink),
                    _ => self.text.text(0, 0, b"VCA ENVELOPE", ink),
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

    fn bass_envelope_labels(&mut self, labels: &Labels, theme: &InstrumentStyle) {
        let (controls, selected) = (labels.bass, labels.bass_control);
        let (ink, muted) = (theme.accent, theme.muted);
        self.text.text(0, 0, b"TB-303", ink);
        self.text.text(7, 0, b"ALG", ink);
        let focus = match selected {
            Some(Control::Knob(knob)) => Some(knob),
            _ => None,
        };
        // Auxiliary knobs have no numbered field. Their selection follows the full readout.
        let mut field = (13, 0..TextGrid::COLUMNS);
        for (i, &knob) in Knob::direct().iter().enumerate() {
            let (x, width) = if i % 2 == 0 { (0, 7) } else { (9, 9) };
            let y = 1 + i / 2;
            if Some(knob) == focus {
                field = (y, x..x + 2 + width);
            }
            self.text.text(x, y, [b"1234"[i]], ink);
            self.text.text(x + 2, y, &knob.name()[..width], muted);
        }
        self.text.highlight(field.0, field.1, theme.selected);
        self.text.text(0, 13, b"                    ", muted);
        if selected == Some(Control::Muffler) {
            self.text.text(0, 13, b"MUFFLER", ink);
            self.text
                .text(10, 13, controls.muffler_label().as_bytes(), ink);
            return;
        }
        if selected == Some(Control::Waveform) {
            self.text.text(0, 13, b"WAVEFORM", ink);
            self.text.text(10, 13, labels.voice.label(Part::Bass), ink);
            return;
        }
        let Some(focus) = focus else {
            self.text
                .number(0, 13, 1, u32::from(labels.bass_preset) + 1, ink);
            self.text.text(
                2,
                13,
                PRESETS[usize::from(labels.bass_preset)].name.as_bytes(),
                ink,
            );
            return;
        };
        let (name, value, units) = controls.readout(focus);
        self.text.text(0, 13, name, ink);
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
        labels: &Labels,
        theme: &InstrumentStyle,
    ) -> Result<(), Error> {
        let (controls, selected) = (labels.bass, labels.bass_control);
        let lanes = style::ACID_LANES;
        let page = Page::BassEnvelope(
            labels.channel,
            controls,
            selected,
            labels.voice.waveform,
            labels.bass_preset,
        );
        let previous = self.page.replace(page);
        if previous != Some(page) {
            let entering =
                !matches!(previous, Some(Page::BassEnvelope(ch, ..)) if ch == labels.channel);
            if entering {
                self.change_view(theme.panel());
                lcd.as_async(clock)?
                    .fill_rect(0, 0, SIZE, SIZE, theme.face.into_storage())
                    .await?;
                for lane in &mut *self.lanes {
                    lane.invalidate();
                }
                self.text.text(12, 0, b"ENV", theme.accent);
                for (i, x) in bass_env::STRIKES.into_iter().enumerate() {
                    let label: &[u8] = if i == 0 { b"PLAIN" } else { b"A" };
                    self.text
                        .text((usize::from(LANE_X) + x) / 12, 3, label, theme.muted);
                }
                for (i, label) in [b"VCF", b"ACC", b"VCA"].into_iter().enumerate() {
                    self.text.text(0, 5 + i * 3, label, lanes[i]);
                }
                self.text.text(0, 14, b"PREVIEW    96 MS/DIV", theme.muted);
            }
            if entering
                || !matches!(previous, Some(Page::BassEnvelope(_, old, ..)) if old == controls)
            {
                for (i, (lane, trace)) in self
                    .lanes
                    .iter_mut()
                    .zip(&bass_env::traces(controls))
                    .enumerate()
                {
                    let style = WaveformStyle {
                        trace: lanes[i],
                        grid_y: 0,
                        center: theme.scope.grid,
                        ..theme.scope
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
            self.bass_envelope_labels(labels, theme);
        }
        self.text.paint_async(lcd, clock).await
    }

    async fn paint_sequencer(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        view: SeqPage,
        labels: &Labels,
        theme: &InstrumentStyle,
        playback: (Option<Position>, BankPosition),
    ) -> Result<(), Error> {
        let (position, bank) = playback;
        let (seq, part) = (labels.seq, labels.focus);
        let style = theme.panel();
        let (ink, muted, active) = (theme.ink, theme.muted, theme.selected.fill);
        let page = Page::Sequencer(view, seq.channel, part);
        if self.page.replace(page) != Some(page) {
            self.change_view(style);
            lcd.as_async(clock)?
                .fill_rect(0, 0, SIZE, SIZE, style.background.into_storage())
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
                self.text.text(0, 8, b"KNOB 3", active);
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
            Part::Hoover => b"SEQ HOOVER",
            Part::Fm => b"SEQ FM",
            Part::Sampler => b"SEQ SAMPLER",
        };
        self.text.text(0, 0, title, ink);
        self.text.text(
            0,
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
                active
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
            if enabled { active } else { muted },
        );
        if view != SeqPage::Tempo {
            self.sequence
                .paint(
                    lcd,
                    clock,
                    self.text,
                    self.strip,
                    (labels, theme),
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
        if self.page.replace(Page::Timing) != Some(Page::Timing) {
            self.badge = None;
            self.selector.select(None, None);
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
        let (user_free, supervisor_free) = wavegraph_fm1_core::peer::cpu0_stack_free();
        for (i, (label, value)) in [
            ("USP FREE B", user_free),
            ("SSP FREE B", supervisor_free),
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
        let labels = &frame.labels;
        let theme = match labels.view {
            View::Mixer | View::Channel | View::Clipboard | View::MasterFx(_) => None,
            View::Scope
            | View::SampleEdit
            | View::VoiceMode
            | View::Envelope
            | View::Arp
            | View::Lfo
            | View::Fx
            | View::BeatFx(_)
            | View::Sequencer => Some(InstrumentStyle::for_part(labels.focus)),
        };
        let feedback = labels
            .preset_since
            .and_then(|since| preset_age(since, ticks()));
        let badge = match labels.preset_role {
            PresetRole::Audition(selected) => Some(badge::Mode::InstrumentSwap(badge::Swap {
                channel: labels.channel,
                live: labels.focus,
                selected,
            })),
            PresetRole::Channel
                if feedback.is_some() && !matches!(labels.view, View::Mixer | View::BeatFx(_)) =>
            {
                Some(badge::Mode::Channel)
            }
            _ => None,
        };
        let was_swap = matches!(self.badge, Some(badge::Mode::InstrumentSwap(_)));
        let swap = matches!(badge, Some(badge::Mode::InstrumentSwap(_)));
        let changed = self.badge != badge;
        let picker_was_visible = self.selector.visible();
        let previous_dot = self.dot;
        if changed && self.badge.is_some() && !(was_swap && swap) {
            self.page = None; // Restore the instrument artwork under the old overlay.
        }
        self.badge = badge;
        self.paint_page(lcd, clock, frame, motion, theme, feedback)
            .await?;
        match badge {
            Some(badge::Mode::InstrumentSwap(state)) => {
                self.dot_background = InstrumentStyle::for_part(state.live).face;
                if changed || picker_was_visible != self.selector.visible() {
                    badge::swap(lcd, clock, self.strip, state, self.selector, !was_swap).await?;
                    self.dot = None;
                } else {
                    // Instrument redraws leave the overload lamp in the reserved header intact.
                    self.dot = previous_dot;
                }
            }
            Some(badge::Mode::Channel) => {
                badge::paint(lcd, clock, self.strip, labels.channel).await?
            }
            None => (),
        }
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

    fn viewport(&self) -> Viewport {
        match self.badge {
            Some(badge::Mode::InstrumentSwap(_)) if self.selector.visible() => {
                Viewport::SwapWithPicker
            }
            Some(badge::Mode::InstrumentSwap(_)) => Viewport::Swap,
            _ => Viewport::Full,
        }
    }

    async fn paint_page(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        frame: &Frame,
        motion: Motion,
        theme: Option<&InstrumentStyle>,
        elapsed: Option<u32>,
    ) -> Result<(), Error> {
        let labels = frame.labels;
        let menu = if elapsed.is_some_and(|age| age < MENU_TICKS) {
            match labels.preset_role {
                PresetRole::Channel if labels.view == View::Scope => Some(Menu::Channels {
                    parts: labels.assignments,
                    focus: labels.channel,
                }),
                PresetRole::Instrument => Some(Menu::Instruments(labels.focus)),
                PresetRole::Audition(selected) => Some(Menu::Instruments(selected)),
                PresetRole::Insert(slot) => {
                    Some(Menu::Effects(labels.inserts[usize::from(slot)].kind()))
                }
                PresetRole::Channel => None,
            }
        } else {
            None
        };
        let audition = matches!(labels.preset_role, PresetRole::Audition(_));
        let previous = self.selector.instrument();
        if self
            .selector
            .select(menu, if audition { None } else { theme })
        {
            if let Some(Menu::Instruments(selected)) = menu
                && audition
            {
                self.selector
                    .paint_instruments(lcd, clock, self.strip, previous, selected)
                    .await?;
                if previous.is_none() {
                    self.page = None;
                }
            } else {
                self.page = None;
            }
        }
        let Some(theme) = theme else {
            let mut page = pin!(self.paint_global(lcd, clock, labels, &motion));
            return poll_fn(|cx| poll_page(page.as_mut(), cx)).await;
        };
        if let View::BeatFx(effect) = labels.view {
            let state = beat_fx::Panel::new(effect, &labels, &motion);
            let previous = match self.page {
                Some(Page::BeatFx(old)) if old.part == labels.focus => Some(old),
                _ => None,
            };
            if previous != Some(state) {
                if previous.is_none() {
                    self.change_view(theme.panel());
                }
                let mut draw = pin!(state.paint(lcd, clock, self.strip, theme, previous));
                poll_fn(|cx| poll_page(draw.as_mut(), cx)).await?;
                self.page = Some(Page::BeatFx(state));
            }
            return Ok(());
        }
        if let Some(page) = labels.seq.page {
            return self
                .paint_sequencer(
                    lcd,
                    clock,
                    page,
                    &labels,
                    theme,
                    (motion.position, motion.banks[labels.seq.channel.index()]),
                )
                .await;
        }
        if labels.view == View::VoiceMode {
            let page = Page::VoiceMode(labels.channel, labels.focus, labels.mode);
            if self.page == Some(page) {
                return Ok(());
            }
            let entering = !matches!(self.page, Some(Page::VoiceMode(channel, part, _)) if (channel, part) == (labels.channel, labels.focus));
            if entering {
                self.change_view(theme.panel());
            }
            let mut draw = pin!(polyphony::paint(
                lcd, clock, self.strip, &labels, theme, entering
            ));
            poll_fn(|cx| poll_page(draw.as_mut(), cx)).await?;
            self.page = Some(page);
            return Ok(());
        }
        if labels.view == View::Lfo {
            let page = Page::Lfo(labels.channel, labels.focus, labels.lfo);
            if self.page == Some(page) {
                return Ok(());
            }
            let previous = match self.page {
                Some(Page::Lfo(channel, part, c))
                    if (channel, part) == (labels.channel, labels.focus) =>
                {
                    Some(c)
                }
                _ => {
                    self.change_view(theme.panel());
                    None
                }
            };
            let mut draw = pin!(lfo::paint(lcd, clock, self.strip, &labels, theme, previous));
            poll_fn(|cx| poll_page(draw.as_mut(), cx)).await?;
            self.page = Some(page);
            return Ok(());
        }
        if labels.view == View::Arp {
            let page = Page::Arp(labels.channel, labels.focus);
            let entering = self.page.replace(page) != Some(page);
            if entering {
                self.change_view(theme.panel());
            }
            return self
                .arp
                .paint(
                    lcd,
                    clock,
                    self.strip,
                    &labels,
                    motion.arp_notes[labels.channel.index()],
                    (theme, entering),
                )
                .await;
        }
        let head = motion
            .position
            .map(|p| usize::from(p.beat_phase) * WIDTH / 256);
        if labels.view == View::Fx {
            let mut labels = labels;
            labels.inserts[0].add_shape(motion.shape_offset(labels.channel));
            let page = Page::Fx(labels.channel, labels.focus);
            let entering = self.page.replace(page) != Some(page);
            let style = self.selector.style(theme.scope);
            if entering {
                self.change_view(style);
                self.dot_background = self.selector.shade(theme.face);
            }
            {
                let mut draw = pin!(self.channel_fx.paint(
                    lcd,
                    clock,
                    self.strip,
                    &labels,
                    (theme, entering),
                    self.selector
                ));
                poll_fn(|cx| poll_page(draw.as_mut(), cx)).await?;
            }
            let rows = channel_fx::SCOPE;
            let height = rows.end - rows.start;
            // Two pixels of margin keep the phosphor halo inside the plot at full scale.
            let trace = frame
                .trace
                .map(|y| (rows.start + 2 + u16::from(y) * (height - 4) / HEIGHT as u16) as u8);
            let center = rows.start + height / 2;
            return self
                .paint_scope(lcd, clock, Sweep { trace, head }, theme, (rows, center))
                .await;
        }
        if labels.view == View::Envelope {
            match labels.focus {
                Part::Hoover => {
                    let mut page = pin!(self.paint_hoover_env(lcd, clock, &labels));
                    return poll_fn(|cx| poll_page(page.as_mut(), cx)).await;
                }
                Part::Bass => {
                    return self.paint_bass_envelope(lcd, clock, &labels, theme).await;
                }
                Part::Mono
                | Part::Organ
                | Part::Supersaw
                | Part::Wavetable
                | Part::Fm
                | Part::Sampler => {
                    let (target, controls) = match labels.focus {
                        Part::Organ => (EnvelopeTarget::Amp, labels.organ.envelope),
                        Part::Sampler => {
                            let p = labels.sampler.timbre;
                            let shape = if labels.envelope_target == EnvelopeTarget::Amp {
                                p.amp
                            } else {
                                p.filter
                            };
                            (labels.envelope_target, EnvelopeControls::new(shape.adsr()))
                        }
                        Part::Fm => (
                            labels.envelope_target,
                            labels.fm.envelope(labels.envelope_target),
                        ),
                        Part::Supersaw if labels.envelope_target == EnvelopeTarget::Modulation => {
                            (labels.envelope_target, labels.saw.filter_envelope)
                        }
                        Part::Wavetable if labels.envelope_target == EnvelopeTarget::Modulation => {
                            (labels.envelope_target, labels.wave.position_envelope)
                        }
                        _ => (EnvelopeTarget::Amp, labels.voice_envelope),
                    };
                    let mut page =
                        pin!(self.paint_envelope(lcd, clock, &labels, theme, target, controls));
                    return poll_fn(|cx| poll_page(page.as_mut(), cx)).await;
                }
                Part::Tr808 | Part::Tr909 => {
                    let state = drums::Envelope::new(&labels, motion.drums[labels.channel.index()]);
                    let page = Page::DrumEnvelope(labels.channel, labels.focus, state);
                    let previous = match self.page.replace(page) {
                        Some(Page::DrumEnvelope(ch, part, old))
                            if (ch, part) == (labels.channel, labels.focus) =>
                        {
                            Some(old)
                        }
                        _ => {
                            self.change_view(theme.panel());
                            None
                        }
                    };
                    let mut draw =
                        pin!(state.paint(lcd, clock, self.strip, &labels, theme, previous));
                    return poll_fn(|cx| poll_page(draw.as_mut(), cx)).await;
                }
            }
        }
        // Poll the instrument face separately to keep pi32v2 branches in range.
        let mut page = pin!(self.paint_instrument(lcd, clock, frame, motion, head, theme));
        poll_fn(|cx| poll_page(page.as_mut(), cx)).await
    }

    async fn paint_instrument(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        frame: &Frame,
        motion: Motion,
        head: Option<usize>,
        theme: &InstrumentStyle,
    ) -> Result<(), Error> {
        let labels = frame.labels;
        let viewport = self.viewport();
        let bass = labels.focus == Part::Bass;
        let organ = labels.focus == Part::Organ;
        let drums = labels.focus.is_kit();
        let sampler = labels.focus == Part::Sampler;
        let saw = labels.focus == Part::Supersaw;
        let wave = labels.focus == Part::Wavetable;
        let hoover = labels.focus == Part::Hoover;
        let fm = labels.focus == Part::Fm;
        let style = self.selector.style(theme.scope);
        let [ink, muted, bright] =
            [theme.accent, theme.muted, theme.ink].map(self.selector.shader());
        let page = Page::Scope(
            labels.channel,
            labels.focus,
            labels.mode,
            if hoover { labels.voice.waveform } else { 0 },
        );
        let changed_page = self.page.replace(page) != Some(page);
        if changed_page {
            self.change_view(style);
        }
        // Leave room for the halo above and below the connected trace.
        let trace = if sampler {
            frame
                .trace
                .map(|y| (58 + u16::from(y) * 100 / HEIGHT as u16) as u8)
        } else if drums {
            frame.trace.map(|y| (60 + u16::from(y) * 5 / 8) as u8)
        } else if saw || wave || fm || hoover {
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
        let (header, footer) = if sampler {
            (56, 80)
        } else if drums {
            (56, 56)
        } else if bass {
            (tb303::SCOPE_TOP, 0)
        } else if fm || hoover {
            (40, 40)
        } else if saw || wave {
            (32, 40)
        } else if organ {
            (16, 32)
        } else {
            (16, 16)
        };
        if changed_page {
            self.dot_background = self.selector.shade(theme.face);
            let mut lcd = PanelDisplay::new(lcd, clock, viewport)?;
            for (y, height) in [(0, header), (SIZE - footer, footer)] {
                if height > 0 {
                    lcd.fill_rect(0, y, SIZE, height, self.dot_background.into_storage())
                        .await?;
                }
            }
            if bass {
                lcd.fill_rect(0, header - 1, SIZE, 1, muted.into_storage())
                    .await?;
            }
        }
        let sweep = Sweep { trace, head };
        let center = if sampler {
            108
        } else if bass {
            header + (SIZE - header) / 2
        } else {
            SIZE / 2
        };
        self.paint_scope(lcd, clock, sweep, theme, (header..SIZE - footer, center))
            .await?;
        let mut display = PanelDisplay::new(lcd, clock, viewport)?;
        if bass {
            let automation = motion
                .bass_automation
                .filter(|(channel, _)| *channel == labels.channel)
                .map(|(_, values)| values);
            return self
                .bass
                .paint(
                    &mut display,
                    self.strip,
                    self.selector,
                    &labels,
                    automation,
                    changed_page,
                )
                .await;
        }
        if hoover {
            if changed_page {
                hoover::header(&mut display, self.strip, self.selector, &labels).await?;
            }
            self.text.text(0, 13, b"CUT  BEND PWM  CHOR", muted);
            let p = labels.hoover;
            for (x, v) in [p.cutoff, p.sweep, p.pwm, p.chorus].into_iter().enumerate() {
                self.text.number(x * 5, 14, 3, u32::from(v), bright);
            }
            return self.text.paint_panel(&mut display).await;
        }
        if fm {
            let mut page = pin!(self.fm.paint(
                &mut display,
                self.strip,
                self.selector,
                &labels,
                changed_page
            ));
            return poll_fn(|cx| poll_page(page.as_mut(), cx)).await;
        }
        if sampler {
            let mut draw = pin!(self.sampler.paint(
                &mut display,
                self.strip,
                self.selector,
                &labels,
                (changed_page, motion.clipboard.take)
            ));
            return poll_fn(|cx| poll_page(draw.as_mut(), cx)).await;
        }
        if wave {
            return self
                .wave
                .paint(
                    &mut display,
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
                .paint(
                    &mut display,
                    self.strip,
                    self.selector,
                    labels,
                    changed_page,
                )
                .await;
        }
        if drums {
            self.drums
                .paint(
                    &mut display,
                    self.strip,
                    self.selector,
                    changed_page,
                    (labels.focus, theme),
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
            return self.text.paint_panel(&mut display).await;
        }
        let part = labels.focus;
        let voice = labels.voice;
        if organ {
            self.text.text(0, 0, b"M1 ORGAN", ink);
            if changed_page {
                // The grid folds lowercase to uppercase. Preserve the exact chord symbol.
                let mut band = mixer::Band::new(self.strip, 144, 224, 48, 16, style.background);
                band.label(
                    crate::panel::Polyphony::label(labels.mode),
                    fm1_graphics::embedded_graphics::prelude::Point::new(168, 227),
                    fm1_graphics::embedded_graphics::text::Alignment::Center,
                    ink,
                );
                display.write_rect(144, 224, 48, 16, band.pixels).await?;
            }
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
            (
                true,
                Part::Bass
                | Part::Supersaw
                | Part::Wavetable
                | Part::Fm
                | Part::Hoover
                | Part::Sampler,
            ) => b"SRC",
            (true, Part::Tr808 | Part::Tr909) => b"KIT",
        };
        self.text.text(16, 0, source, muted);
        self.text.text(0, 14, voice.label(part), ink);
        self.text.text(8, 14, voice.note_name(), bright);
        self.text.text(16, 14, b"V", muted);
        let velocity = voice.gate.note.map_or(0, |note| u32::from(note.velocity));
        self.text.number(17, 14, 3, velocity, ink);
        self.text.paint_panel(&mut display).await
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

impl Screen {
    async fn paint_scope(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        sweep: Sweep,
        theme: &InstrumentStyle,
        (rows, center): (Range<u16>, u16),
    ) -> Result<(), Error> {
        if self.sweep.as_ref() == Some(&sweep) {
            return Ok(());
        }
        let viewport = self.viewport();
        let list = &mut *self.selector;
        let palette = self.scope_palette.get_or_insert_with(|| {
            let style = list.style(theme.scope);
            Palette::new(
                style.background.to_be_bytes(),
                style.grid.to_be_bytes(),
                style.center.to_be_bytes(),
                core::array::from_fn(|age| {
                    let (ink, halo) = theme.phosphor(age);
                    Pen {
                        ink: list.shade(ink).to_be_bytes(),
                        halo: halo.map(|color| list.shade(color).to_be_bytes()),
                    }
                }),
            )
        });
        let damage = if let Some(old) = self.sweep.take() {
            sweep.damage(&old, palette, theme.stripes)
        } else {
            Damage::full(
                usize::from(rows.start)..usize::from(rows.end),
                list.scope_overhang(),
            )
        };
        let mut lcd = lcd.as_async(clock)?;
        for window in damage.windows().flat_map(|window| viewport.windows(window)) {
            for w in window.strips(self.strip.len()) {
                let pixels = &mut self.strip[..w.width * w.height];
                sweep.render(w, palette, usize::from(center), pixels);
                list.overlay(w, pixels);
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
