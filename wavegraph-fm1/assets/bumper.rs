//! Render the boot cue on the build host so it costs no synth voices during startup.

use std::{env, fs, path::PathBuf};
use wavegraph_303::{Voice, VoiceParams};
use wavegraph_909::{Samples, SnareDrum};

// Nearest whole Hz to the FM-1's 24 MHz / 544 audio clock.
const RATE: u16 = 44_118;

#[allow(
    clippy::arithmetic_side_effects,
    reason = "A u16 millisecond count at 44118 Hz fits in 32 bits before division."
)]
fn frame(ms: u16) -> usize {
    (usize::from(ms) * usize::from(RATE) + 500) / 1000
}

enum Gate {
    Accent(u8),
    Pluck(u8),
    Slide(u8),
    Off,
}

fn acid(mix: &mut [f32]) {
    use Gate::{Accent, Off, Pluck, Slide};
    let mut voice = Voice::new(f32::from(RATE));
    let params = VoiceParams {
        cutoff_hz: 295.0,
        resonance: 0.86,
        env_oct: 2.8,
        decay_s: 0.18,
        overdrive_db: 15.0,
        accent: 0.82,
        slide_s: 0.065,
        ..VoiceParams::default()
    };
    // End times in ms. An F-minor pickup at 128 BPM, with an octave slide up
    // and a shorter slide back to the root before the percussion answers.
    let mut start = 0;
    for (end_ms, gate) in [
        (72, Accent(41)),
        (117, Off),
        (177, Pluck(41)),
        (234, Slide(53)),
        (286, Pluck(48)),
        (351, Off),
        (407, Accent(44)),
        (469, Slide(41)),
        (527, Off),
    ] {
        match gate {
            Accent(n) => voice.note_on(i32::from(n), true, false),
            Pluck(n) => voice.note_on(i32::from(n), false, false),
            Slide(n) => voice.note_on(i32::from(n), false, true),
            Off => voice.note_off(),
        }
        let end = frame(end_ms);
        voice.render_add(&params, &mut mix[start..end]);
        start = end;
    }
    // Trim the driven ladder before adding the drums.
    for sample in &mut mix[..start] {
        *sample *= 0.09;
    }
}

fn knocks(mix: &mut [f32]) {
    // Speed up the 909 snare's shell and wires together. The higher ghost
    // stroke leads the lower, louder one by a 32nd note.
    for (ms, speed, mut gain) in [(527, 3, 0.38), (586, 2, 0.90)] {
        let mut snare = SnareDrum::new(u32::from(RATE), speed).expect("fixed snare clock");
        snare.trigger();
        for sample in &mut mix[frame(ms)..] {
            *sample += gain * f32::from(snare.next_frame()) / 32768.0;
            gain *= 0.9993;
        }
    }
    // Just the first 12 ms of the closed hat puts a metallic edge on the last hit.
    let hat = Samples::X0XPROC1.closed_hat.as_chunks::<2>().0;
    let mut gain = 0.16;
    for (sample, &bytes) in mix[frame(586)..].iter_mut().zip(hat).take(frame(12)) {
        *sample += gain * f32::from(i16::from_le_bytes(bytes)) / 32768.0;
        gain *= 0.99;
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "The PCM peak is scaled to a quarter of full scale."
)]
fn main() {
    println!("cargo:rerun-if-changed=assets/bumper.rs");
    let mut mix = vec![0.0; frame(750)];
    acid(&mut mix);
    knocks(&mut mix);
    let peak = mix.iter().copied().map(f32::abs).fold(0.0, f32::max);
    let scale = f32::from(i16::MAX) * 0.25 / peak;
    let pcm: Vec<u8> = mix
        .iter()
        .flat_map(|x| ((x * scale).round() as i16).to_le_bytes())
        .collect();
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::write(out.join("bumper.s16le"), pcm).expect("write boot cue");
}
