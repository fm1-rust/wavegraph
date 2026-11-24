//! Single-bar clip preview. Rebuild the note and controller lanes on a bar or slot
//! change; the running playhead repaints just its old and new columns.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Grid coordinates fit the 240-pixel panel and clips are bounded to 512 ticks."
)]

use super::{drums, mixer::Band};
use crate::{
    bass::CC_SLIDE,
    instrument::Part,
    sequencer::{
        BankPosition, Position, Sequencer,
        pattern::{BAR, Clip, SLOTS, Slot},
    },
};
use embedded_graphics::{
    pixelcolor::raw::ToBytes,
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_graphics::{Rgb565, TextGrid, WaveformStyle, palette};
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Display, Error},
    time::Clock,
};
use wavegraph::midi::MidiMessage;
use wavegraph_909::Voice;

const LEFT: usize = 48;
const TOP: usize = 48;
const WIDTH: usize = 12;
const HEIGHT: usize = 160;
const HALF: usize = HEIGHT / 2;
const INKS: [Rgb565; 3] = [
    palette::CYAN,
    Rgb565::new(30, 49, 7),
    Rgb565::new(28, 15, 20),
];
const ACID: Rgb565 = Rgb565::new(31, 24, 4);

pub(super) fn style(part: Part) -> WaveformStyle {
    if part.is_kit() {
        drums::style(part)
    } else {
        super::STYLE
    }
}

pub(super) fn ink(part: Part) -> Rgb565 {
    if part == Part::Tr909 {
        drums::GRAPHITE
    } else {
        palette::WHITE
    }
}

fn drum_label(v: Voice) -> &'static str {
    match v {
        Voice::Kick => "KICK",
        Voice::Snare => "SNARE",
        Voice::Clap => "CLAP",
        Voice::ClosedHat => "CL HH",
        Voice::OpenHat => "OP HH",
        Voice::Ride => "RIDE",
    }
}

#[derive(Clone, Copy, Default)]
struct Cell {
    gate: u8,
    accent: bool,
    slide: bool,
}

struct Roll {
    cells: [[Cell; 16]; 10],
    // 7-bit MIDI controls plus a sentinel for an unwritten lane.
    curves: [[u8; 3]; 128],
}

impl Roll {
    fn rows(part: Part, clip: &Clip) -> usize {
        if part.is_kit() {
            part.pads().len()
        } else {
            clip.pitches
                .len()
                .min(if part == Part::Bass { 7 } else { 10 })
        }
    }

    fn pitch_row(clip: &Clip, rows: usize, pitch: u8) -> usize {
        let rank = clip.pitches.partition_point(|&p| p > pitch);
        rank * rows / clip.pitches.len()
    }

    fn read(&mut self, clip: &Clip, part: Part, bar: u8) {
        self.cells = [[Cell::default(); 16]; 10];
        self.curves.fill([255; 3]);
        let rows = Self::rows(part, clip);
        let mut held = [None; 4];
        let mut events = clip.events.iter().peekable();
        let mut lanes = [255; 3];
        let mut slide = false;
        let start = u16::from(bar) * BAR;
        for tick in 0..start + BAR {
            let t = usize::from(tick % BAR);
            let step = t / 8;
            while let Some(e) = events.next_if(|e| e.tick() == tick) {
                match e.message() {
                    MidiMessage::On(n) if part.is_kit() => {
                        if tick >= start
                            && let Some(voice) = part.drum(n.number)
                            && let Some(row) = part.pads().iter().position(|&v| v == voice)
                        {
                            self.cells[row][step].gate = 255;
                            self.cells[row][step].accent |= n.velocity >= 100;
                        }
                    }
                    MidiMessage::On(n) => {
                        let row = Self::pitch_row(clip, rows, n.number);
                        if tick >= start {
                            let cell = &mut self.cells[row][step];
                            cell.accent |= n.velocity >= 100;
                            cell.slide |= slide || held.iter().any(Option::is_some);
                        }
                        if matches!(part, Part::Wavetable | Part::Organ) {
                            if !held.contains(&Some(n.number))
                                && let Some(v) = held.iter_mut().find(|v| v.is_none())
                            {
                                *v = Some(n.number);
                            }
                        } else {
                            held[0] = Some(n.number);
                        }
                    }
                    MidiMessage::Off(n) => {
                        if let Some(v) = held.iter_mut().find(|v| **v == Some(n)) {
                            *v = None;
                        }
                    }
                    MidiMessage::Control(CC_SLIDE, value) => slide = value >= 64,
                    MidiMessage::Control(cc, value) => {
                        if let Some(lane) = [74, 71, 17].iter().position(|&n| n == cc) {
                            lanes[lane] = value;
                        }
                    }
                }
            }
            if tick >= start {
                self.curves[t] = lanes;
                for &n in held.iter().flatten() {
                    self.cells[Self::pitch_row(clip, rows, n)][step].gate |= 1 << (t % 8);
                }
            }
        }
    }

    fn drum_pixel(
        &self,
        part: Part,
        step: usize,
        x: usize,
        y: usize,
        head: bool,
        enabled: bool,
    ) -> Rgb565 {
        let skin = style(part);
        // ReDrum-style voice lanes and a separate row of transport lamps.
        let row_h = if part == Part::Tr808 { 32 } else { 22 };
        let row = y / row_h;
        let dy = y % row_h;
        if row >= part.pads().len() {
            if (140..146).contains(&y) && (2..10).contains(&x) {
                return if head { drums::HOT } else { skin.grid };
            }
            if (151..154).contains(&y) && step.is_multiple_of(4) {
                return skin.trace;
            }
            return skin.background;
        }
        if x < 2 && step.is_multiple_of(4) {
            return skin.center;
        }
        if !(2..11).contains(&x) || !(3..row_h - 3).contains(&dy) {
            return skin.background;
        }
        let cell = self.cells[row][step];
        if cell.gate == 0 || !enabled {
            return if head { skin.center } else { skin.grid };
        }
        let color = if part == Part::Tr808 {
            [
                Rgb565::new(30, 9, 3),
                drums::HOT,
                Rgb565::new(31, 40, 3),
                drums::CREAM,
            ][step / 4]
        } else if cell.accent {
            Rgb565::new(24, 18, 2)
        } else {
            Rgb565::new(15, 22, 16)
        };
        if head && dy < 7 { ink(part) } else { color }
    }

    fn pixel(
        &self,
        part: Part,
        rows: usize,
        step: usize,
        x: usize,
        y: usize,
        head: bool,
        enabled: bool,
    ) -> Rgb565 {
        if part.is_kit() {
            return self.drum_pixel(part, step, x, y, head, enabled);
        }
        let row = y / 16;
        let dy = y % 16;
        let background = if head {
            Rgb565::new(3, 14, 9)
        } else {
            super::STYLE.background
        };
        if row < rows {
            let c = self.cells[row][step];
            if part == Part::Bass && (1..=5).contains(&dy) {
                let flag = if c.accent && (1..=3).contains(&x) {
                    Some(([2, 5, 7, 5, 5], x - 1))
                } else if c.slide && (7..=9).contains(&x) {
                    Some(([3, 4, 2, 1, 6], x - 7))
                } else {
                    None
                };
                if flag.is_some_and(|(glyph, dx)| glyph[dy - 1] & (4 >> dx) != 0) {
                    return if enabled {
                        palette::WHITE
                    } else {
                        palette::MUTED
                    };
                }
            }
            if (7..=12).contains(&dy) && (c.gate & (1 << (x * 8 / WIDTH))) != 0 {
                return if !enabled {
                    palette::MUTED
                } else if head {
                    palette::WHITE
                } else {
                    match part {
                        Part::Bass if c.accent => INKS[1],
                        Part::Bass => ACID,
                        Part::Wavetable => super::microwave::STYLE.trace,
                        Part::Supersaw => super::jp8080::STYLE.trace,
                        Part::Organ => super::ORGAN_LIME,
                        _ => palette::GREEN,
                    }
                };
            }
        } else if part == Part::Bass && row < rows + 3 {
            let lane = row - rows;
            let t = step * 8 + x * 8 / WIDTH;
            let v = self.curves[t][lane];
            if v < 128 {
                let height = |v: u8| 14 - usize::from(v) * 12 / 127;
                let a = height(v);
                let b = self.curves[t.saturating_sub(1)][lane];
                let b = if b < 128 { height(b) } else { a };
                if (a.min(b)..=a.max(b)).contains(&dy) {
                    return if enabled { INKS[lane] } else { palette::MUTED };
                }
            }
        } else {
            return background;
        }
        if x == 0 || dy == 15 {
            if step.is_multiple_of(4) {
                super::STYLE.center
            } else {
                super::STYLE.grid
            }
        } else {
            background
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct BankButton {
    selected: bool,
    queued: bool,
    dark: bool,
}

pub(super) struct Grid {
    previous: Option<(Slot, u8, bool)>,
    buttons: [Option<BankButton>; SLOTS],
    head: Option<u8>,
    roll: Roll,
}

impl Grid {
    pub(super) const fn new() -> Self {
        Self {
            previous: None,
            buttons: [None; SLOTS],
            head: None,
            roll: Roll {
                cells: [[Cell {
                    gate: 0,
                    accent: false,
                    slide: false,
                }; 16]; 10],
                curves: [[255; 3]; 128],
            },
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.previous = None;
        self.buttons = [None; SLOTS];
        self.head = None;
    }

    async fn labels(
        lcd: &mut AsyncDisplay<'_, Clock>,
        strip: &mut [[u8; 2]],
        part: Part,
        clip: &Clip,
    ) -> Result<(), Error> {
        let rows = Roll::rows(part, clip);
        let paper = style(part).background;
        for y in (0..HEIGHT).step_by(16) {
            let mut band = Band::new(strip, 0, 0, LEFT as u16, 16, paper);
            if !part.is_kit() {
                let row = y / 16;
                if row < rows {
                    let n = clip.pitches[(row * clip.pitches.len()).div_ceil(rows)];
                    let name = note_name(n);
                    band.label(
                        core::str::from_utf8(&name)
                            .expect("ASCII note")
                            .trim_ascii_end(),
                        Point::new(4, 4),
                        Alignment::Left,
                        ink(part),
                    );
                } else if part == Part::Bass && row < rows + 3 {
                    band.label(
                        ["CUT", "RES", "DRV"][row - rows],
                        Point::new(4, 4),
                        Alignment::Left,
                        INKS[row - rows],
                    );
                }
            }
            lcd.write_rect_buffered(
                0,
                (TOP + y) as u16,
                LEFT as u16,
                16,
                band.pixels.as_flattened(),
            )
            .await?;
        }
        if part.is_kit() {
            let h = if part == Part::Tr808 { 32 } else { 22 };
            for (i, v) in part.pads().iter().enumerate() {
                let mut band = Band::new(strip, 0, 0, LEFT as u16, 12, paper);
                band.label(drum_label(*v), Point::new(4, 1), Alignment::Left, ink(part));
                lcd.write_rect_buffered(
                    0,
                    (TOP + i * h + (h - 12) / 2) as u16,
                    LEFT as u16,
                    12,
                    band.pixels.as_flattened(),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn buttons(
        &mut self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        strip: &mut [[u8; 2]],
        part: Part,
        active: Slot,
        next: Slot,
        pressed_off: bool,
    ) -> Result<(), Error> {
        const LABELS: [&str; SLOTS] = ["1 F", "2 G", "3 A", "4 B", "5 C", "6 D", "7 E", "8 F"];
        const WIDTH: u16 = 240 / SLOTS as u16;
        let skin = style(part);
        for (i, label) in LABELS.into_iter().enumerate() {
            let number = i as u8 + 1;
            let requested = number == next.number();
            let state = BankButton {
                selected: number == active.number(),
                queued: requested && next != active,
                dark: requested && pressed_off,
            };
            if self.buttons[i] == Some(state) {
                continue;
            }
            let mut button = Band::new(strip, 0, 0, WIDTH, 16, skin.background);
            if !state.dark {
                let (cap, text) = if state.selected {
                    (ink(part), skin.background)
                } else {
                    (skin.grid, ink(part))
                };
                button.rect(Point::new(1, 2), Size::new(u32::from(WIDTH - 4), 14), cap);
                button.label(
                    label,
                    Point::new(i32::from(WIDTH / 2), 4),
                    Alignment::Center,
                    text,
                );
            }
            if state.queued {
                button.rect(
                    Point::new(2, 14),
                    Size::new(u32::from(WIDTH - 6), 2),
                    drums::HOT,
                );
            }
            lcd.write_rect_buffered(
                i as u16 * WIDTH,
                208,
                WIDTH,
                16,
                button.pixels.as_flattened(),
            )
            .await?;
            self.buttons[i] = Some(state);
        }
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The grid borrows the shared panel drawing buffers."
    )]
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        text: &mut TextGrid,
        strip: &mut [[u8; 2]],
        (seq, part): (Sequencer, Part),
        position: Option<Position>,
        bank: BankPosition,
    ) -> Result<(), Error> {
        let next = seq.slot(seq.channel, part);
        let position = position.filter(|_| seq.transport.playing());
        let slot = position.map_or(next, |_| bank.slot);
        let clip = slot.clip(part, seq.channel);
        let bar = position.map_or(seq.bar % clip.bars, |_| bank.bar % clip.bars);
        let head = position.map(|p| p.step % 16);
        let enabled = seq.enabled[seq.channel.index()];
        let key = (slot, bar, enabled);
        let changed = self.previous != Some(key);
        let mut lcd = lcd.as_async(clock)?;
        self.buttons(&mut lcd, strip, part, slot, next, seq.pressed_key_dark)
            .await?;
        text.number(16, 0, 1, u32::from(bar) + 1, ink(part));
        text.text(17, 0, [b'/', b'0' + clip.bars], ink(part));
        if let Some(head) = head {
            text.number(17, 1, 2, u32::from(head) + 1, ink(part));
        } else {
            text.text(17, 1, b"--", ink(part));
        }
        if slot != next {
            text.text(0, 14, b"QUEUED   / NEXT BAR ", ink(part));
            text.number(7, 14, 1, u32::from(next.number()), ink(part));
        } else {
            text.text(0, 14, b"WHITE KEYS  / SELECT", ink(part));
        }
        if !changed && head == self.head {
            return lcd.flush().await;
        }
        if changed {
            if self.previous.is_none_or(|p| p.0 != slot) {
                Self::labels(&mut lcd, strip, part, clip).await?;
            }
            self.roll.read(clip, part, bar);
        }
        let rows = Roll::rows(part, clip);
        for step in 0..16u8 {
            if !changed && self.head != Some(step) && head != Some(step) {
                continue;
            }
            for top in [0, HALF] {
                let pixels = &mut strip[..WIDTH * HALF];
                for y in top..top + HALF {
                    for x in 0..WIDTH {
                        pixels[(y - top) * WIDTH + x] = self
                            .roll
                            .pixel(
                                part,
                                rows,
                                usize::from(step),
                                x,
                                y,
                                head == Some(step),
                                enabled,
                            )
                            .to_be_bytes();
                    }
                }
                lcd.write_rect_buffered(
                    (LEFT + usize::from(step) * WIDTH) as u16,
                    (TOP + top) as u16,
                    WIDTH as u16,
                    HALF as u16,
                    pixels.as_flattened(),
                )
                .await?;
            }
        }
        lcd.flush().await?;
        self.previous = Some(key);
        self.head = head;
        Ok(())
    }
}

fn note_name(note: u8) -> [u8; 4] {
    const NAMES: [&[u8; 2]; 12] = [
        b"C ", b"C#", b"D ", b"D#", b"E ", b"F ", b"F#", b"G ", b"G#", b"A ", b"A#", b"B ",
    ];
    let [letter, accidental] = *NAMES[usize::from(note % 12)];
    let octave = if note < 12 {
        *b"-1"
    } else {
        [b'0' + note / 12 - 1, b' ']
    };
    if accidental == b' ' {
        [letter, octave[0], octave[1], b' ']
    } else {
        [letter, accidental, octave[0], octave[1]]
    }
}
