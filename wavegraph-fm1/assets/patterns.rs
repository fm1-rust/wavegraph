//! Compile Standard MIDI Files on the host. The audio interrupt only reads ROM events.

use core::fmt::Write as _;
use midly::{Format, MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use std::{fs, path::Path};

const BANKS: [&str; 8] = [
    "mono",
    "organ",
    "tb303-low",
    "tr909",
    "tr808",
    "supersaw",
    "wavetable",
    "tb303-high",
];
const TICKS_PER_BEAT: u64 = 32;
const BAR: u64 = TICKS_PER_BEAT * 4;

pub(super) fn compile_banks(out: &Path) {
    println!("cargo:rerun-if-changed=assets/patterns.rs");
    let mut code = String::from("static BANKS: [[Clip; SLOTS]; 8] = [\n");
    for bank in BANKS {
        code.push_str("[\n");
        for slot in 1..=4 {
            let filename = format!("patterns/{bank}/{slot:02}.mid");
            println!("cargo:rerun-if-changed={filename}");
            let bytes = fs::read(&filename).expect(&filename);
            let clip = compile(&bytes, bank).unwrap_or_else(|why| {
                eprintln!("{filename}: {why}");
                std::process::exit(1)
            });
            code.push_str(&clip);
        }
        code.push_str("],\n");
    }
    code.push_str("];\n");
    fs::write(out.join("patterns.rs"), code).expect("write pattern banks");
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Four bars constrain event times; the checked scale operates on u64."
)]
fn compile(bytes: &[u8], bank: &str) -> Result<String, String> {
    let smf = Smf::parse(bytes).map_err(|e| e.to_string())?;
    let Timing::Metrical(ppqn) = smf.header.timing else {
        return Err("use musical PPQN timing".into());
    };
    let ppqn = u64::from(ppqn.as_int());
    if ppqn == 0 || matches!(smf.header.format, Format::Sequential) {
        return Err("use SMF format 0 or 1 and a positive PPQN".into());
    }
    let mut events = Vec::new();
    let mut length = 0;
    let mut input_channel = None;
    for track in smf.tracks {
        let mut time = 0_u64;
        for event in track {
            time += u64::from(event.delta.as_int());
            if time > 16 * ppqn {
                return Err("pattern exceeds four bars of 4/4".into());
            }
            let tick = (time * TICKS_PER_BEAT + ppqn / 2) / ppqn;
            length = length.max(tick);
            let TrackEventKind::Midi { channel, message } = event.kind else {
                if matches!(event.kind, TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, ..)) if (n,d) != (4,2))
                {
                    return Err("use a 4/4 clip".into());
                }
                continue;
            };
            if input_channel
                .replace(channel)
                .is_some_and(|old| old != channel)
            {
                return Err("export one instrument channel per clip".into());
            }
            let (kind, key, value): (u8, u8, u8) = match message {
                MidiMessage::NoteOn { key, vel } if vel.as_int() != 0 => {
                    (9, key.as_int(), vel.as_int())
                }
                MidiMessage::NoteOff { key, vel } | MidiMessage::NoteOn { key, vel } => {
                    (8, key.as_int(), vel.as_int())
                }
                MidiMessage::Controller { controller, value } => {
                    (11, controller.as_int(), value.as_int())
                }
                _ => return Err("use note and control-change messages".into()),
            };
            if matches!((kind, key), (11, 120..=127)) {
                return Err("channel-mode messages are reserved for the transport".into());
            }
            events.push((tick, kind, key, value));
        }
    }
    events.sort_by_key(|e| e.0); // stable, preserving file order for simultaneous events
    let bars = length.div_ceil(BAR).max(1);
    let mut pitches: Vec<_> = events.iter().filter(|e| e.1 == 9).map(|e| e.2).collect();
    pitches.sort_unstable_by(|a, b| b.cmp(a));
    pitches.dedup();
    let mut code = format!("Clip {{ bars: {bars}, pitches: &{pitches:?}, events: &[");
    let kit = matches!(bank, "tr808" | "tr909");
    let mut held_since = [None; 128];
    let mut last_tick = None;
    let mut messages = 0;
    let mut hits = 0;
    for (tick, kind, key, value) in events {
        if tick == bars * BAR && kind != 8 {
            return Err("last onset or controller needs time before the loop ends".into());
        }
        if last_tick != Some(tick) {
            last_tick = Some(tick);
            messages = 0;
            hits = 0;
        }
        messages += 1;
        hits += usize::from(kind == 9);
        // At 300 BPM on the FM-1, at most one tick falls in a DMA half.
        // Kits can also receive all 40 buffered live strikes in that same half.
        if messages > 16 || (kit && hits > 6) {
            return Err(format!("too many simultaneous messages at tick {tick}"));
        }
        let held = &mut held_since[usize::from(key)];
        if kind == 9 {
            if held.replace(tick).is_some() {
                return Err(format!("overlapping strikes of pitch {key} at tick {tick}"));
            }
            if !kit && held_since.iter().flatten().count() > 4 {
                return Err(format!("more than four held notes at tick {tick}"));
            }
        } else if kind == 8 && held.take() == Some(tick) {
            return Err(format!(
                "note {key} is shorter than the 32 PPQN grid at tick {tick}"
            ));
        }
        // Bits 4..12 carry the musical tick, bits 0..3 the MIDI status nibble.
        write!(
            code,
            "Timed({}, [{key},{value}]),",
            (tick << 4) | u64::from(kind)
        )
        .map_err(|e| e.to_string())?;
    }
    if held_since.iter().any(Option::is_some) {
        return Err("end each held note before or at the loop boundary".into());
    }
    code.push_str("] },\n");
    Ok(code)
}
