//! Four real-time sliders and a two-line sample readout, inspired by the Emulator II.

use super::{BAND, Selector, mixer::Band};
use crate::{
    assets,
    firmware::Labels,
    instrument::Voice,
    panel::{Channel, View},
    sampler::Program,
};
use core::fmt::Write;
use fm1_graphics::Rgb565;
use fm1_graphics::embedded_graphics::{
    prelude::{Point, Size},
    text::Alignment,
};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};
use heapless::String;
use wavegraph_sampler::{PlaybackMode, Timbre};

pub(super) const FACE: Rgb565 = super::style::SAMPLER.face;
pub(super) const PRINT: Rgb565 = super::style::SAMPLER.ink;
pub(super) const LCD: Rgb565 = Rgb565::new(22, 48, 12);
const INK: Rgb565 = Rgb565::new(3, 8, 3);
const RECESS: Rgb565 = super::style::SAMPLER.scope.background;

#[derive(Clone, Copy, Eq, PartialEq)]
struct Header {
    channel: Channel,
    slot: u8,
    source: bool,
    key: [u8; 4],
    velocity: u8,
    transpose: i8,
    root: Option<u8>,
    slices: bool,
    pad: Option<u8>,
}

#[derive(Default)]
pub(super) struct Panel {
    header: Option<Header>,
    controls: Option<(Program, bool, Option<u8>)>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]],
        selector: &mut Selector,
        labels: &Labels,
        (entering, take): (bool, Option<crate::clipboard::Take>),
    ) -> Result<(), Error> {
        let header = Header {
            channel: labels.channel,
            slot: labels.voice.waveform,
            source: labels.voice_tap,
            key: labels.voice.note_name(),
            velocity: labels.voice.gate.note.map_or(0, |n| n.velocity),
            transpose: labels.voice.transpose,
            slices: labels.sampler.playback.mode == PlaybackMode::Slice,
            pad: labels.slice.map(|(i, _)| i),
            root: if labels.voice.waveform == assets::CLIPBOARD_SLOT {
                take.map(|t| t.root)
            } else {
                Some(assets::RAVE_BANK[usize::from(labels.voice.waveform)].root_note())
            },
        };
        let mut program = labels.sampler;
        if header.slices {
            [program.playback.start, program.playback.end] =
                labels.slice.map_or([0, 100], |(_, trim)| trim);
        }
        let controls = (program, labels.view == View::SampleEdit, header.pad);
        let top = entering || self.header != Some(header);
        let bottom = entering || self.controls != Some(controls);
        if !top && !bottom {
            return Ok(());
        }
        let mut display = lcd.as_async(clock)?;
        if top {
            let index = usize::from(header.slot);
            let root = header.root.map_or(*b"--  ", Voice::pitch_name);
            let mut first = String::<32>::new();
            let mut second = String::<40>::new();
            write!(
                first,
                "{:02} {}    ROOT {}",
                header.slot + 1,
                ascii(&assets::SAMPLE_NAMES[index]),
                if header.root.is_some() {
                    ascii(&root)
                } else {
                    "EMPTY"
                }
            )
            .expect("LCD line");
            if header.slices {
                if let Some(i) = header.pad {
                    let key = Voice::pitch_name(assets::PAD_NOTES[usize::from(i)]);
                    write!(
                        second,
                        "C4 FULL    {} CHOP {}    REC",
                        ascii(&key).trim(),
                        i + 1
                    )
                } else {
                    write!(second, "C4 FULL    LIT KEYS CHOP")
                }
            } else {
                write!(
                    second,
                    "KEY {:3}  VEL {:3}  TRANS {:+}",
                    ascii(&header.key).trim(),
                    header.velocity,
                    header.transpose
                )
            }
            .expect("LCD line");
            for y in (0..56).step_by(BAND) {
                let mut band = Band::new(pixels, 0, y, SIZE, BAND as u16, FACE);
                band.title("SAMPLER II", Point::new(8, 3), PRINT);
                band.label(
                    header.channel.label(),
                    Point::new(177, 5),
                    Alignment::Left,
                    PRINT,
                );
                band.label(
                    if header.source { "SRC" } else { "DAC" },
                    Point::new(229, 5),
                    Alignment::Right,
                    PRINT,
                );
                band.rect(Point::new(6, 19), Size::new(228, 33), RECESS);
                band.rect(Point::new(9, 22), Size::new(222, 27), LCD);
                band.label(&first, Point::new(15, 24), Alignment::Left, INK);
                band.label(&second, Point::new(15, 36), Alignment::Left, INK);
                band.compose(selector);
                display
                    .write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
                    .await?;
            }
            self.header = Some(header);
        }
        if bottom {
            let (program, edit, pad) = controls;
            let mut values: [String<8>; 4] = core::array::from_fn(|_| String::new());
            let sliders = sliders(program, edit, &mut values);
            for y in (160..SIZE).step_by(BAND) {
                let mut band = Band::new(pixels, 0, y, SIZE, BAND as u16, FACE);
                band.rect(Point::new(5, 162), Size::new(230, 1), PRINT);
                band.label(
                    if edit {
                        "SAMPLE EDIT"
                    } else {
                        "REAL TIME CONTROL"
                    },
                    Point::new(9, 165),
                    Alignment::Left,
                    PRINT,
                );
                if edit && header.slices {
                    overview(&mut band, pad, program.playback);
                }
                for (i, (label, position)) in sliders.iter().enumerate() {
                    let x = 30 + i as i32 * 60;
                    band.label(label, Point::new(x, 178), Alignment::Center, PRINT);
                    band.rect(Point::new(x - 2, 191), Size::new(4, 24), RECESS);
                    let cap = 211 - i32::from(*position) * 19 / 100;
                    band.rect(Point::new(x - 9, cap), Size::new(18, 6), RECESS);
                    band.rect(Point::new(x - 7, cap + 2), Size::new(14, 1), PRINT);
                    band.label(
                        &values[i],
                        Point::new(x, 220),
                        Alignment::Center,
                        super::style::SAMPLER.accent,
                    );
                }
                band.compose(selector);
                display
                    .write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
                    .await?;
            }
            self.controls = Some(controls);
        }
        display.flush().await
    }
}

#[inline(never)]
fn sliders(program: Program, edit: bool, values: &mut [String<8>; 4]) -> [(&'static str, u8); 4] {
    let [first, second, third, fourth] = values;
    let p = program.playback;
    let t = program.timbre;
    if edit {
        let mode = match p.mode {
            PlaybackMode::OneShot => "SHOT",
            PlaybackMode::Gate => "GATE",
            PlaybackMode::Loop => "LOOP",
            PlaybackMode::Slice => "SLICE",
        };
        first.push_str(mode).expect("mode");
        write!(second, "{}%", p.start).expect("start");
        write!(third, "{}%", p.end).expect("end");
        write!(fourth, "{}%", t.key_tracking).expect("key track");
        [
            (
                "1 MODE",
                if p.mode == PlaybackMode::Slice {
                    100
                } else {
                    p.mode as u8 * 33
                },
            ),
            ("2 START", p.start),
            ("3 END", p.end),
            ("4 KEY TRK", t.key_tracking),
        ]
    } else {
        if t.cutoff_hz == Timbre::OPEN {
            first.push_str("OPEN").expect("cutoff");
        } else {
            write!(first, "{}Hz", t.cutoff_hz).expect("cutoff");
        }
        write!(second, "{}", t.resonance).expect("resonance");
        write!(third, "{:+}st", t.env_semitones).expect("envelope depth");
        write!(fourth, "{}st", t.velocity_semitones).expect("velocity depth");
        [
            (
                "1 CUTOFF",
                (u16::from(program.cutoff_step()) * 100
                    / u16::from(crate::instrument::MAX_CUTOFF_STEP)) as u8,
            ),
            ("2 RESO", (u16::from(t.resonance) * 100 / 127) as u8),
            (
                "3 ENV AMT",
                ((i16::from(t.env_semitones) + 48) * 100 / 96) as u8,
            ),
            (
                "4 VEL>VCF",
                (u16::from(t.velocity_semitones) * 100 / 48) as u8,
            ),
        ]
    }
}

// 96 RMS columns measured from the four-bar factory recording, computed off-device.
const AMEN_LEVEL: [u8; 96] = [
    11, 7, 5, 10, 7, 5, 8, 2, 2, 2, 3, 2, 2, 3, 2, 11, 9, 8, 8, 4, 2, 2, 3, 2, 11, 7, 5, 10, 7, 5,
    8, 3, 1, 2, 3, 2, 2, 3, 2, 11, 9, 8, 8, 4, 2, 2, 3, 2, 11, 7, 5, 10, 7, 5, 8, 2, 2, 2, 3, 2, 2,
    3, 2, 11, 7, 5, 4, 2, 1, 8, 2, 1, 2, 3, 2, 11, 7, 5, 8, 3, 1, 2, 3, 2, 2, 3, 2, 10, 7, 5, 4, 2,
    2, 7, 3, 1,
];

fn overview(band: &mut Band<'_>, pad: Option<u8>, playback: wavegraph_sampler::Playback) {
    let frames = assets::AMEN_PADS[7].end;
    let (mut lo, mut hi) = (0, frames);
    if let Some(i) = pad {
        let cut = assets::AMEN_PADS[usize::from(i)];
        let len = cut.end - cut.start;
        lo = cut.start + len * u32::from(playback.start) / 100;
        hi = cut.start + (len * u32::from(playback.end)).div_ceil(100);
    }
    let first = lo * 96 / frames;
    let end = (hi * 96).div_ceil(frames);
    for (index, &height) in AMEN_LEVEL.iter().enumerate() {
        let x = u32::try_from(index).expect("96 columns");
        let ink = if (first..end).contains(&x) {
            PRINT
        } else {
            LCD
        };
        band.rect(
            Point::new(
                134 + i32::try_from(x).expect("LCD x"),
                176 - i32::from(height),
            ),
            Size::new(1, u32::from(height)),
            ink,
        );
    }
}

fn ascii(bytes: &[u8]) -> &str {
    core::str::from_utf8(bytes).expect("ASCII panel text")
}

pub(super) async fn clipboard(
    lcd: &mut Display,
    clock: &mut Clock,
    pixels: &mut [[u8; 2]],
    labels: &Labels,
    status: crate::clipboard::Status,
) -> Result<(), Error> {
    use crate::clipboard::State;
    let colors = super::mixer::RackColors::GLOBAL;
    let root = Voice::pitch_name(labels.clip_options.root);
    let length = match labels.clip_options.beats() {
        1 => "1 BEAT",
        4 => "1 BAR",
        8 => "2 BARS",
        _ => "4 BARS",
    };
    let state = match status.state {
        State::Idle if status.take.is_some() => "READY",
        State::Idle => "EMPTY",
        State::Armed if labels.seq.transport.playing() => "WAITING FOR DOWNBEAT",
        State::Armed => "PLAY A NOTE",
        State::Recording => "RECORDING",
        State::Interrupted => "TAKE INTERRUPTED",
        State::SelfInput => "CHOOSE ANOTHER SOURCE",
    };
    let source = heapless::format!(32; "{} / {}", labels.channel.label(), labels.focus.name())
        .expect("short source");
    let mut info = String::<40>::new();
    if let Some(take) = status.take {
        write!(
            info,
            "{} HZ   {} MS",
            take.hz,
            u32::from(take.frames) * 1000 / u32::from(take.hz)
        )
        .expect("take");
    }
    let source_hint = if matches!(status.state, State::Armed | State::Recording) {
        status.source.label()
    } else {
        ""
    };
    let action = match status.state {
        State::Armed => "REC  CANCEL",
        State::Recording => "REC  FINISH",
        _ => "REC  ARM CAPTURE",
    };
    let (length_label, root_label) = if status.state == State::Recording {
        ("1  NEXT LENGTH", "2  NEXT ROOT")
    } else {
        ("1  LENGTH", "2  ROOT")
    };
    let mut display = lcd.as_async(clock)?;
    for y in (0..SIZE).step_by(BAND) {
        let mut band = Band::new(pixels, 0, y, SIZE, BAND as u16, super::mixer::BACKGROUND);
        band.title("PCM CLIPBOARD", Point::new(8, 8), colors.ink);
        band.label(&source, Point::new(8, 35), Alignment::Left, colors.ink);
        band.rect(Point::new(7, 55), Size::new(226, 57), colors.surface);
        band.label(state, Point::new(14, 65), Alignment::Left, colors.accent);
        band.label(
            source_hint,
            Point::new(228, 65),
            Alignment::Right,
            colors.accent,
        );
        band.label(&info, Point::new(14, 89), Alignment::Left, colors.accent);
        for (row, label, value) in [(132, length_label, length), (151, root_label, ascii(&root))] {
            band.label(label, Point::new(8, row), Alignment::Left, colors.ink);
            band.label(value, Point::new(228, row), Alignment::Right, colors.ink);
        }
        band.label(action, Point::new(8, 187), Alignment::Left, colors.ink);
        if labels.focus == crate::instrument::Part::Sampler {
            band.label(
                "SEL  USE CLIPBOARD",
                Point::new(8, 209),
                Alignment::Left,
                colors.ink,
            );
        } else {
            band.label(
                "PRESETS  CHOOSE CHANNEL",
                Point::new(8, 209),
                Alignment::Left,
                colors.ink,
            );
        }
        display
            .write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
            .await?;
    }
    display.flush().await
}
