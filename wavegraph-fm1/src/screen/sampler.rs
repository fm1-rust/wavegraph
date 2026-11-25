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
}

#[derive(Default)]
pub(super) struct Panel {
    header: Option<Header>,
    controls: Option<(Program, bool)>,
}

impl Panel {
    pub(super) async fn paint(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        pixels: &mut [[u8; 2]],
        selector: &mut Selector,
        labels: &Labels,
        entering: bool,
    ) -> Result<(), Error> {
        let header = Header {
            channel: labels.channel,
            slot: labels.voice.waveform,
            source: labels.voice_tap,
            key: labels.voice.note_name(),
            velocity: labels.voice.gate.note.map_or(0, |n| n.velocity),
            transpose: labels.voice.transpose,
        };
        let controls = (labels.sampler, labels.view == View::SampleEdit);
        let top = entering || self.header != Some(header);
        let bottom = entering || self.controls != Some(controls);
        if !top && !bottom {
            return Ok(());
        }
        let mut display = lcd.as_async(clock)?;
        if top {
            let index = usize::from(header.slot);
            let root = Voice::pitch_name(assets::RAVE_BANK[index].root_note());
            let mut first = String::<32>::new();
            let mut second = String::<40>::new();
            write!(
                first,
                "{:02} {}    ROOT {}",
                header.slot + 1,
                ascii(&assets::SAMPLE_NAMES[index]),
                ascii(&root)
            )
            .expect("LCD line");
            write!(
                second,
                "KEY {:3}  VEL {:3}  TRANS {:+}",
                ascii(&header.key).trim(),
                header.velocity,
                header.transpose
            )
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
            let (program, edit) = controls;
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
        };
        first.push_str(mode).expect("mode");
        write!(second, "{}%", p.start).expect("start");
        write!(third, "{}%", p.end).expect("end");
        write!(fourth, "{}%", t.key_tracking).expect("key track");
        [
            ("1 MODE", p.mode as u8 * 50),
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

fn ascii(bytes: &[u8]) -> &str {
    core::str::from_utf8(bytes).expect("ASCII panel text")
}
