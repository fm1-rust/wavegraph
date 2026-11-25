//! Single-bar clip preview. Rebuild the note and controller lanes on a bar or slot
//! change; the running playhead repaints just its old and new columns.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Grid coordinates fit the 240-pixel panel and clips are bounded to 512 ticks."
)]

use super::TextGrid;
use super::{InstrumentStyle, drums, mixer::Band, style::ACID_LANES};
use crate::{
    bass::CC_SLIDE,
    instrument::Part,
    sequencer::{
        BankPosition, Position, SeqPage,
        edit::{Address, EDITS, Step, Suppressed},
        pattern::{BAR, Clip, SLOTS, Slot},
    },
};
use fm1_graphics::Rgb565;
use fm1_graphics::embedded_graphics::{
    pixelcolor::raw::ToBytes,
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_rticx::{
    bsp::display::{AsyncDisplay, Display, Error},
    time::Clock,
};
use wavegraph_909::Voice;
use wavegraph_dsp::midi::MidiMessage;

const LEFT: usize = 48;
const TOP: usize = 48;
const WIDTH: usize = 12;
const HEIGHT: usize = 160;
const HALF: usize = HEIGHT / 2;
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
    pitches: [u8; 10],
    pitch_count: usize,
    cells: [[Cell; 16]; 10],
    // 7-bit MIDI controls plus a sentinel for an unwritten lane.
    curves: [[u8; 3]; 128],
}

impl Roll {
    fn rows(&self, part: Part) -> usize {
        if part.is_kit() {
            part.pads().len()
        } else {
            self.pitch_count
        }
    }

    fn pitch_row(&self, pitch: u8) -> usize {
        self.pitches[..self.pitch_count]
            .partition_point(|&p| p >= pitch)
            .saturating_sub(1)
    }

    fn read(&mut self, clip: &Clip, at: Address) {
        let part = at.part;
        self.cells = [[Cell::default(); 16]; 10];
        self.curves.fill([255; 3]);
        if !part.is_kit() {
            let mut used = [false; 128];
            for &pitch in clip.pitches {
                used[usize::from(pitch)] = true;
            }
            for step in 0..16 {
                for note in EDITS.step(Address { step, ..at }).notes() {
                    used[usize::from(note.number)] = true;
                }
            }
            let total = used.iter().filter(|&&present| present).count();
            self.pitch_count = total.min(if part == Part::Bass { 7 } else { 10 });
            let mut row = 0;
            for (rank, pitch) in (0..128u8)
                .rev()
                .filter(|&n| used[usize::from(n)])
                .enumerate()
            {
                if rank * self.pitch_count / total == row {
                    self.pitches[row] = pitch;
                    row += 1;
                }
            }
        }
        let mut edited = Step::EMPTY;
        let mut suppressed = Suppressed::EMPTY;
        let mut held = [None; 4];
        let mut events = clip.events.iter().peekable();
        let mut lanes = [255; 3];
        let mut slide = false;
        let start = u16::from(at.bar) * BAR;
        for tick in 0..start + BAR {
            let t = usize::from(tick % BAR);
            let step = t / 8;
            let phase = tick % 8;
            if phase == 0 {
                edited = EDITS.step(Address {
                    bar: (tick / BAR) as u8,
                    step: step as u8,
                    ..at
                });
                if edited.edited() && !part.is_kit() {
                    for &n in held.iter().flatten() {
                        suppressed.cut(n);
                    }
                    held.fill(None);
                }
            }
            let mut apply = |msg| match msg {
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
                    let row = self.pitch_row(n.number);
                    if tick >= start {
                        let cell = &mut self.cells[row][step];
                        cell.accent |= n.velocity >= 100;
                        cell.slide |= slide || held.iter().any(Option::is_some);
                    }
                    if part.is_polyphonic() {
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
            };
            while let Some(e) = events.next_if(|e| e.tick() == tick) {
                let msg = e.message();
                if suppressed.factory(edited, part, msg, phase) {
                    apply(msg);
                }
            }
            for msg in edited.events(phase) {
                apply(msg);
            }
            if tick >= start {
                self.curves[t] = lanes;
                for &n in held.iter().flatten() {
                    let row = self.pitch_row(n);
                    self.cells[row][step].gate |= 1 << (t % 8);
                }
            }
        }
    }

    fn drum_pixel(
        &self,
        (part, theme): (Part, &InstrumentStyle),
        step: usize,
        x: usize,
        y: usize,
        head: bool,
        enabled: bool,
    ) -> Rgb565 {
        // ReDrum-style voice lanes and a separate row of transport lamps.
        let row_h = if part == Part::Tr808 { 32 } else { 22 };
        let row = y / row_h;
        let dy = y % row_h;
        if row >= part.pads().len() {
            if (140..146).contains(&y) && (2..10).contains(&x) {
                return if head { theme.accent } else { theme.surface };
            }
            if (151..154).contains(&y) && step.is_multiple_of(4) {
                return theme.accent;
            }
            return theme.face;
        }
        if x < 2 && step.is_multiple_of(4) {
            return theme.muted;
        }
        if !(2..11).contains(&x) || !(3..row_h - 3).contains(&dy) {
            return theme.face;
        }
        let cell = self.cells[row][step];
        if cell.gate == 0 || !enabled {
            return if head { theme.muted } else { theme.surface };
        }
        let color = if part == Part::Tr808 {
            [
                Rgb565::new(30, 9, 3),
                drums::HOT,
                Rgb565::new(31, 40, 3),
                drums::CREAM,
            ][step / 4]
        } else if cell.accent {
            theme.accent
        } else {
            theme.muted
        };
        if head && dy < 7 { theme.ink } else { color }
    }

    fn pixel(
        &self,
        (part, theme): (Part, &InstrumentStyle),
        rows: usize,
        step: usize,
        x: usize,
        y: usize,
        head: bool,
        enabled: bool,
    ) -> Rgb565 {
        if part.is_kit() {
            return self.drum_pixel((part, theme), step, x, y, head, enabled);
        }
        let row = y / 16;
        let dy = y % 16;
        let background = if head { theme.surface } else { theme.face };
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
                    return if enabled { theme.ink } else { theme.muted };
                }
            }
            if (7..=12).contains(&dy) && (c.gate & (1 << (x * 8 / WIDTH))) != 0 {
                return if !enabled {
                    theme.muted
                } else if head {
                    theme.ink
                } else {
                    if part == Part::Bass && c.accent {
                        theme.ink
                    } else {
                        theme.accent
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
                    return if enabled {
                        ACID_LANES[lane]
                    } else {
                        theme.muted
                    };
                }
            }
        } else {
            return background;
        }
        if x == 0 || dy == 15 {
            if step.is_multiple_of(4) {
                theme.muted
            } else {
                theme.surface
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
    previous: Option<(Slot, u8, bool, u32, bool)>,
    cursor: Option<u8>,
    buttons: [Option<BankButton>; SLOTS],
    head: Option<u8>,
    roll: Roll,
}

impl Grid {
    pub(super) const fn new() -> Self {
        Self {
            previous: None,
            cursor: None,
            buttons: [None; SLOTS],
            head: None,
            roll: Roll {
                pitches: [0; 10],
                pitch_count: 0,
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
        self.cursor = None;
        self.buttons = [None; SLOTS];
        self.head = None;
    }

    async fn labels(
        &self,
        lcd: &mut AsyncDisplay<'_, Clock>,
        strip: &mut [[u8; 2]],
        (part, theme): (Part, &InstrumentStyle),
    ) -> Result<(), Error> {
        let rows = self.roll.rows(part);
        let paper = theme.face;
        for y in (0..HEIGHT).step_by(16) {
            let mut band = Band::new(strip, 0, 0, LEFT as u16, 16, paper)
                .with_panel_type(super::lettering::panel_type(part));
            if !part.is_kit() {
                let row = y / 16;
                if row < rows {
                    let n = self.roll.pitches[row];
                    let name = note_name(n);
                    band.label(
                        core::str::from_utf8(&name)
                            .expect("ASCII note")
                            .trim_ascii_end(),
                        Point::new(4, 4),
                        Alignment::Left,
                        theme.ink,
                    );
                } else if part == Part::Bass && row < rows + 3 {
                    band.label(
                        ["CUT", "RES", "DRV"][row - rows],
                        Point::new(4, 4),
                        Alignment::Left,
                        ACID_LANES[row - rows],
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
                let mut band = Band::new(strip, 0, 0, LEFT as u16, 12, paper)
                    .with_panel_type(super::lettering::panel_type(part));
                band.label(drum_label(*v), Point::new(4, 1), Alignment::Left, theme.ink);
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
        (part, theme): (Part, &InstrumentStyle),
        active: Slot,
        next: Slot,
        pressed_off: bool,
    ) -> Result<(), Error> {
        const LABELS: [&str; SLOTS] = ["1 F", "2 G", "3 A", "4 B", "5 C", "6 D", "7 E", "8 F"];
        const WIDTH: u16 = 240 / SLOTS as u16;
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
            let mut button = Band::new(strip, 0, 0, WIDTH, 16, theme.face)
                .with_panel_type(super::lettering::panel_type(part));
            if !state.dark {
                let (cap, text) = if state.selected {
                    (theme.ink, theme.face)
                } else {
                    (theme.surface, theme.ink)
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
                    theme.accent,
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
        (labels, theme): (&super::Labels, &InstrumentStyle),
        position: Option<Position>,
        bank: BankPosition,
    ) -> Result<(), Error> {
        let (seq, part) = (labels.seq, labels.focus);
        let next = seq.slot(seq.channel, part);
        let position = position.filter(|_| seq.transport.playing());
        let editing = seq.page == Some(SeqPage::Step);
        let cursor = editing.then_some(seq.cursor);
        let follow = position.filter(|_| !editing);
        let slot = follow.map_or(next, |_| bank.slot);
        let house_m7 = follow.map_or_else(
            || part.uses_house_patterns(labels.voice.waveform),
            |_| bank.house_m7,
        );
        let clip = slot.factory_clip(part, seq.channel, house_m7);
        let bar = follow.map_or(seq.bar % clip.bars, |_| bank.bar % clip.bars);
        let head = position
            .filter(|_| bank.slot == slot && bank.bar == bar)
            .map(|p| p.step % 16);
        let enabled = seq.enabled[seq.channel.index()];
        let key = (slot, bar, enabled, seq.edit_revision, house_m7);
        let changed = self.previous != Some(key);
        let mut lcd = lcd.as_async(clock)?;
        if seq.page == Some(SeqPage::Pattern) {
            self.buttons(
                &mut lcd,
                strip,
                (part, theme),
                slot,
                next,
                seq.pressed_key_dark,
            )
            .await?;
        } else {
            text.text(0, 13, b"P", theme.ink);
            text.number(1, 13, 1, u32::from(slot.number()), theme.ink);
            text.text(
                4,
                13,
                if editing {
                    b"K1 STEP   K2 BAR"
                } else {
                    b"PLAY TO OVERDUB "
                },
                theme.ink,
            );
        }
        text.number(16, 0, 1, u32::from(bar) + 1, theme.ink);
        text.text(17, 0, [b'/', b'0' + clip.bars], theme.ink);
        if let Some(head) = cursor.or(head) {
            text.number(17, 1, 2, u32::from(head) + 1, theme.ink);
        } else {
            text.text(17, 1, b"--", theme.ink);
        }
        if seq.full && matches!(seq.page, Some(SeqPage::Record | SeqPage::Step)) {
            text.text(0, 14, b"EDIT LIMIT REACHED  ", theme.ink);
        } else if editing {
            text.text(0, 14, b"KEY SET  SEL+KEY DEL", theme.ink);
        } else if seq.page == Some(SeqPage::Record) {
            text.text(0, 14, b"REC AGAIN TO FINISH ", theme.ink);
        } else if slot != next {
            text.text(0, 14, b"QUEUED   / NEXT BAR ", theme.ink);
            text.number(7, 14, 1, u32::from(next.number()), theme.ink);
        } else {
            text.text(0, 14, b"KEYS:BANK   REC EDIT", theme.ink);
        }
        if !changed && head == self.head && cursor == self.cursor {
            return lcd.flush().await;
        }
        if changed {
            let old_labels = (self.roll.pitch_count, self.roll.pitches);
            self.roll.read(
                clip,
                Address {
                    channel: seq.channel,
                    part,
                    slot,
                    bar,
                    step: 0,
                },
            );
            if self.previous.is_none() || old_labels != (self.roll.pitch_count, self.roll.pitches) {
                self.labels(&mut lcd, strip, (part, theme)).await?;
            }
        }
        let rows = self.roll.rows(part);
        for step in 0..16u8 {
            if !changed && ![self.head, head, self.cursor, cursor].contains(&Some(step)) {
                continue;
            }
            for top in [0, HALF] {
                let pixels = &mut strip[..WIDTH * HALF];
                for y in top..top + HALF {
                    for x in 0..WIDTH {
                        pixels[(y - top) * WIDTH + x] = if cursor == Some(step)
                            && (x == 0 || x == WIDTH - 1 || y == 0 || y == HEIGHT - 1)
                        {
                            theme.ink
                        } else {
                            self.roll.pixel(
                                (part, theme),
                                rows,
                                usize::from(step),
                                x,
                                y,
                                head == Some(step),
                                enabled,
                            )
                        }
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
        self.cursor = cursor;
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
