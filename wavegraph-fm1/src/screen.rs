//! Scope and envelope pages for the FM-1 display.

use crate::{
    bass::{Bank, Controls, Knob, envelope as bass_env},
    capture::{FLAT, HEIGHT, Trace, WIDTH},
    envelope::EnvelopeControls,
    firmware::Frame,
    instrument::Part,
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

mod splash;

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
const MENU_BG: Rgb565 = Rgb565::new(4, 7, 8);
const MENU_BORDER: Rgb565 = Rgb565::new(22, 48, 5);
const MENU_TICKS: u32 = 140; // The 100 Hz timer holds the patch list for 1.4 s.
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
    Menu(Part),
    Fx,
    Envelope(EnvelopeControls),
    BassEnvelope(Controls),
}

pub(super) struct Screen {
    plot: &'static mut Waveform<WIDTH, WIDTH>,
    lanes: [&'static mut Lane; 3],
    text: &'static mut TextGrid,
    strip: &'static mut [[u8; 2]; BAND * HEIGHT],
    beam: Beam,
    page: Option<Page>,
}

impl Screen {
    pub(super) fn new() -> Self {
        static PLOT: StaticCell<Waveform<WIDTH, WIDTH>> = StaticCell::new();
        static LANES: [StaticCell<Lane>; 3] = [const { StaticCell::new() }; 3];
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
            page: None,
            beam: Beam {
                old_head: None,
                old_trace: FLAT,
            },
        }
    }

    fn change_view(&mut self, style: WaveformStyle) {
        self.plot.set_style(style);
        *self.text = TextGrid::with_background(style.background);
        self.beam.old_head = None;
    }

    // A modal overlay on top of the previous scope. During the 1.4-second
    // selection window only the menu is painted; audio and scope capture keep
    // running. The next page repaint invalidates the covered waveform tiles.
    async fn paint_menu(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        focus: Part,
    ) -> Result<(), Error> {
        let next = Page::Menu(focus);
        if self.page == Some(next) {
            return Ok(());
        }
        self.page = Some(next);
        *self.text = TextGrid::with_background(MENU_BG);
        // Narrow lime frame and charcoal interior evoke the M1 patch browser.
        lcd.fill_rect(12, 48, 216, 144, MENU_BORDER.into_storage())?;
        lcd.fill_rect(14, 50, 212, 140, MENU_BG.into_storage())?;
        self.text.text(3, 4, b"PATCH SELECT", palette::CYAN);
        let names: [&[u8]; 3] = [b"1 MONOSYNTH", b"2 M1 ORGAN", b"3 TB-303"];
        for (i, (part, name)) in [Part::Mono, Part::Organ, Part::Bass]
            .into_iter()
            .zip(names)
            .enumerate()
        {
            let selected = part == focus;
            self.text
                .text(3, 6 + i * 2, if selected { b"+" } else { b" " }, ORGAN_LIME);
            self.text.text(
                5,
                6 + i * 2,
                name,
                if selected { ORGAN_LIME } else { palette::WHITE },
            );
        }
        self.text.paint_async(lcd, clock).await
    }

    async fn paint_envelope(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        controls: EnvelopeControls,
        late: bool,
    ) -> Result<(), Error> {
        let page = Page::Envelope(controls);
        let previous = self.page.replace(page);
        if previous != Some(page) {
            let entering = !matches!(previous, Some(Page::Envelope(_)));
            if entering {
                self.change_view(ENV_STYLE);
            }
            self.plot.paint_async(lcd, &controls.trace(), clock).await?;
            if entering {
                // The curve's dirty tiles stay between the two label bands.
                let background = ENV_STYLE.background.into_storage();
                lcd.fill_rect(0, 0, SIZE, 16, background)?;
                lcd.fill_rect(0, 192, SIZE, SIZE - 192, background)?;
            }
            self.text.text(0, 0, b"VCA ENVELOPE", palette::GREEN);
            let adsr = controls.adsr();
            env_time(self.text, 0, 12, b"A", adsr.attack_us);
            env_time(self.text, 10, 12, b"D", adsr.decay_us);
            self.text.text(0, 14, b"S", palette::WHITE);
            self.text
                .number(1, 14, 4, u32::from(adsr.sustain_percent), palette::GREEN);
            self.text.text(5, 14, b"/100", palette::MUTED);
            env_time(self.text, 10, 14, b"R", adsr.release_us);
        }
        if late {
            self.text.text(16, 0, b"LATE", palette::RED);
        }
        self.text.paint_async(lcd, clock).await
    }

    fn bass_panel(&mut self, controls: Controls) {
        let (ink, muted) = (OXIDE_RED, BASS_MUTED);
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
        late: bool,
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
        if late {
            self.text.text(16, 0, b"LATE", OXIDE_RED);
        }
        self.text.paint_async(lcd, clock).await
    }

    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        frame: &Frame,
        late: bool,
    ) -> Result<(), Error> {
        let labels = frame.labels;
        if labels
            .menu_since
            .is_some_and(|since| ticks().wrapping_sub(since) < MENU_TICKS)
        {
            return self.paint_menu(lcd, clock, labels.focus).await;
        }
        // The menu covered scope tiles; force a clean redraw on its exit.
        if matches!(self.page, Some(Page::Menu(_))) {
            self.plot.invalidate();
            self.page = None;
        }
        if labels.env_mode && !labels.fx_view {
            match labels.focus {
                Part::Bass => {
                    return self
                        .paint_bass_envelope(lcd, clock, labels.bass, late)
                        .await;
                }
                Part::Mono => {
                    return self
                        .paint_envelope(lcd, clock, labels.mono_envelope, late)
                        .await;
                }
                Part::Organ => {} // Organ ENV is reserved for a future editor.
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
        let page = if labels.fx_view {
            Page::Fx
        } else {
            Page::Scope(labels.focus)
        };
        let changed_page = self.page.replace(page) != Some(page);
        if changed_page {
            self.change_view(style);
        }
        // Keep the silver trace and its halo between the knob legend and the readout.
        let trace = if bass {
            frame.trace.map(|y| (25 + u16::from(y) * 3 / 4) as u8)
        } else {
            frame.trace
        };
        self.plot
            .paint_async(lcd, &trace.map(|y| y + TOP as u8), clock)
            .await?;
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
        self.beam
            .paint(lcd, clock, &trace, self.strip, style, labels.focus)
            .await?;
        if labels.fx_view {
            self.text.text(0, 0, b"FX / REVERB", ink);
            self.text.text(
                12,
                0,
                if labels.fx.enabled { b"ON " } else { b"OFF" },
                bright,
            );
            self.text.text(
                16,
                0,
                if late {
                    b"LATE"
                } else if labels.half_gain {
                    b"HALF"
                } else {
                    b"FULL"
                },
                muted,
            );
            self.text.text(0, 13, b"1:M", muted);
            self.text.number(4, 13, 3, u32::from(labels.fx.mono), ink);
            self.text.text(8, 13, b"2:303", muted);
            self.text.number(15, 13, 3, u32::from(labels.fx.bass), ink);
            self.text.text(0, 14, b"3:DEC", muted);
            self.text.number(6, 14, 3, u32::from(labels.fx.decay), ink);
            self.text.text(11, 14, b"4:WET", muted);
            self.text.number(17, 14, 3, u32::from(labels.fx.wet), ink);
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
        self.text.text(17, 0, source, muted);
        self.text.text(0, 14, labels.voice.label(labels.focus), ink);
        self.text.text(8, 14, labels.voice.note_name(), bright);
        if late {
            self.text
                .text(11, 14, b"LATE", if bass { OXIDE_RED } else { palette::RED });
        }
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

fn env_time(text: &mut TextGrid, x: usize, y: usize, label: &[u8], us: u32) {
    let (value, unit) = if us.is_multiple_of(1_000_000) {
        (us / 1_000_000, b"S ")
    } else if us.is_multiple_of(1000) {
        (us / 1000, b"MS")
    } else {
        (us, b"US")
    };
    text.text(x, y, label, palette::WHITE);
    text.number(x + 1, y, 5, value, palette::GREEN);
    text.text(x + 6, y, unit, palette::MUTED);
}

struct Beam {
    old_head: Option<usize>,
    old_trace: Trace,
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

impl Beam {
    async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        trace: &Trace,
        scratch: &mut [[u8; 2]],
        style: WaveformStyle,
        part: Part,
    ) -> Result<(), Error> {
        let head = self.old_head.map_or(0, |x| (x + BAND) % WIDTH);
        let mut lcd = lcd.as_async(clock)?;
        for x0 in (0..WIDTH).step_by(BAND) {
            // Include the old halo when a note or its gain changes under the beam.
            let mut top = HEIGHT;
            let mut bottom = 0;
            for x in x0..x0 + BAND {
                for (which_head, which_trace) in
                    [(Some(head), trace), (self.old_head, &self.old_trace)]
                {
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
            let pixels = &mut scratch[..BAND * height];
            for dx in 0..BAND {
                let x = x0 + dx;
                let (a, b) = segment(trace, x);
                let (ink, halo) = phosphor(part, age(head, x));
                for y in top..=bottom {
                    let paper = paper(style, x, usize::from(TOP) + y);
                    let color = if (a..=b).contains(&y) {
                        ink
                    } else if y + 1 == a || y == b + 1 {
                        halo.unwrap_or(paper)
                    } else {
                        paper
                    };
                    pixels[(y - top) * BAND + dx] = color.to_be_bytes();
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
        self.old_head = Some(head);
        self.old_trace = *trace;
        Ok(())
    }
}
