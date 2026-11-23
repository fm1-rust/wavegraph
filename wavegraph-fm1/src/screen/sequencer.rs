//! A cached, single-bar pattern grid. Only moving playhead columns are repainted.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "All coordinates are bounded by a 240-pixel display and a 64-step pattern."
)]

use crate::{
    instrument::Part,
    sequencer::{Position, Sequencer, pattern},
};
use embedded_graphics::pixelcolor::raw::ToBytes;
use fm1_graphics::{Rgb565, TextGrid, palette};
use fm1_rticx::{
    bsp::display::{Display, Error},
    time::Clock,
};
use wavegraph_909::Voice;

const LEFT: usize = 48;
const TOP: usize = 48;
const WIDTH: usize = 12;
const HEIGHT: usize = 160;
const HALF: usize = HEIGHT / 2;
const ROW: usize = 16;
const BACKGROUND: Rgb565 = super::STYLE.background;
const GRID: Rgb565 = Rgb565::new(2, 9, 4);
const BEAT: Rgb565 = Rgb565::new(4, 19, 9);
const CURSOR: Rgb565 = Rgb565::new(3, 14, 9);
const ACID: Rgb565 = Rgb565::new(31, 24, 4);
const RESONANCE: Rgb565 = Rgb565::new(30, 49, 7);
const DRIVE: Rgb565 = Rgb565::new(28, 15, 20);
const DRUMS: [(Voice, &[u8]); 6] = [
    (Voice::Kick, b"BD"),
    (Voice::Snare, b"SD"),
    (Voice::Clap, b"CP"),
    (Voice::ClosedHat, b"CH"),
    (Voice::OpenHat, b"OH"),
    (Voice::Ride, b"RD"),
];

#[derive(Clone, Copy, Default)]
struct Cell {
    pixels: u16,
    onset: bool,
    accent: bool,
    slide: bool,
}

pub(super) struct Grid {
    previous: Option<(Part, u8, bool)>,
    head: Option<u8>,
    pitches: [u8; 8],
    rows: usize,
}

impl Grid {
    pub(super) const fn new() -> Self {
        Self {
            previous: None,
            head: None,
            pitches: [0; 8],
            rows: 0,
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.previous = None;
        self.head = None;
    }

    fn labels(&mut self, text: &mut TextGrid, track: Part) {
        for row in 3..13 {
            text.text(0, row, b"    ", palette::MUTED);
        }
        text.text(4, 2, b"1   2   3   4   ", palette::MUTED);
        self.rows = 0;
        if track == Part::Drums {
            for (row, (_, label)) in DRUMS.into_iter().enumerate() {
                text.text(0, row + 3, label, palette::MUTED);
            }
            self.rows = DRUMS.len();
            return;
        }
        // One pitch per occupied lane, rather than spending the small screen
        // on semitones absent from this preset. High pitches are at the top.
        for step in 0..pattern::STEPS {
            for &note in pattern::step(track, step).notes {
                if !self.pitches[..self.rows].contains(&note) && self.rows < self.pitches.len() {
                    self.pitches[self.rows] = note;
                    self.rows += 1;
                }
            }
        }
        self.pitches[..self.rows].sort_unstable_by(|a, b| b.cmp(a));
        for (row, &note) in self.pitches[..self.rows].iter().enumerate() {
            text.text(0, row + 3, note_name(note), palette::MUTED);
        }
        if track == Part::Bass {
            for (row, (label, ink)) in [
                (b"CUT", palette::CYAN),
                (b"RES", RESONANCE),
                (b"DRV", DRIVE),
            ]
            .into_iter()
            .enumerate()
            {
                text.text(0, self.rows + row + 3, label, ink);
            }
        }
    }

    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        text: &mut TextGrid,
        strip: &mut [[u8; 2]],
        (seq, part): (Sequencer, Part),
        position: Option<Position>,
    ) -> Result<(), Error> {
        let position = position.filter(|_| seq.transport.playing());
        let bar = position.map_or(seq.bar, |p| p.step / 16);
        let head = position.map(|p| p.step % 16);
        let enabled = seq.enabled[seq.channel.index()];
        let key = (part, bar, enabled);
        let changed = self.previous != Some(key);
        if changed && self.previous.is_none_or(|(track, _, _)| track != part) {
            self.labels(text, part);
        }
        text.number(15, 0, 1, u32::from(bar) + 1, palette::WHITE);
        text.text(16, 0, b"/4", palette::MUTED);
        if let Some(step) = head {
            text.number(17, 1, 2, u32::from(step) + 1, palette::WHITE);
        } else {
            text.text(17, 1, b"--", palette::MUTED);
        }
        if !changed && self.head == head {
            return Ok(());
        }
        let mut lcd = lcd.as_async(clock)?;
        for step in 0..16_u8 {
            if !changed && self.head != Some(step) && head != Some(step) {
                continue;
            }
            let absolute = usize::from(bar) * 16 + usize::from(step);
            let cells: [Cell; 8] = core::array::from_fn(|row| {
                if part == Part::Drums {
                    Cell {
                        pixels: if row < DRUMS.len()
                            && pattern::drums(absolute).contains(&DRUMS[row].0)
                        {
                            0x7fe
                        } else {
                            0
                        },
                        onset: true,
                        ..Cell::default()
                    }
                } else if row < self.rows {
                    note_cell(part, absolute, self.pitches[row])
                } else {
                    Cell::default()
                }
            });
            let automation: [[u8; WIDTH + 1]; 3] = core::array::from_fn(|lane| {
                core::array::from_fn(|x| automation_height(absolute, x, lane))
            });
            for top in [0, HALF] {
                let pixels = &mut strip[..WIDTH * HALF];
                for y in top..top + HALF {
                    for x in 0..WIDTH {
                        pixels[(y - top) * WIDTH + x] = pixel(
                            part,
                            &cells,
                            &automation,
                            self.rows,
                            step,
                            head == Some(step),
                            enabled,
                            x,
                            y,
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
    const NAMES: [[u8; 2]; 12] = [
        *b"C ", *b"C#", *b"D ", *b"D#", *b"E ", *b"F ", *b"F#", *b"G ", *b"G#", *b"A ", *b"A#",
        *b"B ",
    ];
    let [letter, accidental] = NAMES[usize::from(note % 12)];
    let octave = b'0' + (note / 12).saturating_sub(1);
    if accidental == b'#' {
        [letter, accidental, octave, b' ']
    } else {
        [letter, octave, b' ', b' ']
    }
}

fn note_cell(track: Part, absolute: usize, note: u8) -> Cell {
    let strike = pattern::step(track, absolute);
    let mut cell = Cell {
        onset: strike.notes.contains(&note),
        accent: strike.velocity >= 96,
        slide: strike.slide,
        ..Cell::default()
    };
    // A u8 gate spans at most 32 steps. Include notes tied over a bar boundary.
    // The preset monophonic tracks terminate their old pitch at a new strike.
    for age in 0..32 {
        let step = pattern::step(track, (absolute + pattern::STEPS - age) % pattern::STEPS);
        if step.notes.contains(&note) {
            let gate = if step.slide {
                step.gate.max(8)
            } else {
                step.gate
            };
            for x in 0..WIDTH {
                let elapsed = age * 8 + x * 8 / WIDTH;
                if elapsed < usize::from(gate) {
                    cell.pixels |= 1 << x;
                }
            }
        }
        if track != Part::Organ && !step.notes.is_empty() {
            break;
        }
    }
    cell
}

fn automation_height(step: usize, x: usize, lane: usize) -> u8 {
    // Include the previous column's final point so adjacent strips meet.
    let (step, fraction) = if x == 0 {
        ((step + pattern::STEPS - 1) % pattern::STEPS, 234)
    } else {
        (step, ((x - 1) * 256 / WIDTH) as u8)
    };
    let value = pattern::automation(step, fraction);
    let scaled = match lane {
        0 => u32::from(value.cutoff_hz.saturating_sub(150)).min(1650) * 12 / 1650,
        1 => u32::from(value.resonance.saturating_sub(64)).min(36) * 12 / 36,
        _ => u32::from(value.overdrive_db).min(36) * 12 / 36,
    };
    14 - scaled as u8
}

#[allow(
    clippy::too_many_arguments,
    reason = "One small strip is composed without a framebuffer."
)]
fn pixel(
    track: Part,
    cells: &[Cell; 8],
    automation: &[[u8; WIDTH + 1]; 3],
    rows: usize,
    step: u8,
    head: bool,
    enabled: bool,
    x: usize,
    y: usize,
) -> Rgb565 {
    let row = y / ROW;
    let dy = y % ROW;
    let background = if head { CURSOR } else { BACKGROUND };
    if row < rows {
        let cell = cells[row];
        if cell.onset && track == Part::Bass {
            // Small A/S flags above each struck note: accent and outgoing slide.
            let flag = if cell.accent && (1..=3).contains(&x) {
                Some(([2, 5, 7, 5, 5], x - 1))
            } else if cell.slide && (7..=9).contains(&x) {
                Some(([3, 4, 2, 1, 6], x - 7))
            } else {
                None
            };
            if let Some((glyph, dx)) = flag
                && (1..=5).contains(&dy)
                && glyph[dy - 1] & (4 >> dx) != 0
            {
                return if enabled { palette::WHITE } else { BEAT };
            }
        }
        let note_top = if track == Part::Bass { 7 } else { 3 };
        if (note_top..=12).contains(&dy) && cell.pixels & (1 << x) != 0 {
            return if !enabled {
                BEAT
            } else if head {
                palette::WHITE
            } else {
                match track {
                    Part::Mono => palette::GREEN,
                    Part::Organ => super::ORGAN_LIME,
                    Part::Bass if cell.accent => RESONANCE,
                    Part::Bass => ACID,
                    Part::Drums => palette::CYAN,
                }
            };
        }
    } else if track == Part::Bass && row < rows + 3 {
        let lane = row - rows;
        let a = usize::from(automation[lane][x]);
        let b = usize::from(automation[lane][x + 1]);
        if (a.min(b)..=a.max(b)).contains(&dy) {
            return if !enabled {
                BEAT
            } else if head {
                palette::WHITE
            } else {
                [palette::CYAN, RESONANCE, DRIVE][lane]
            };
        }
    } else {
        return background;
    }
    if x == 0 || dy == 15 {
        if step.is_multiple_of(4) { BEAT } else { GRID }
    } else {
        background
    }
}
