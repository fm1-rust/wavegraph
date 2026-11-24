//! Render the boot cue on the build host so it costs no synth voices during startup.

mod boot_timing;
mod fonts;
mod patterns;

use boot_timing::{ACCENT_A, FIRST_KNOCK, HIGH_D, SLIDE_C};

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
    Slide(u8, bool),
    Off,
}

fn acid(mix: &mut [f32]) {
    use Gate::{Accent, Off, Pluck, Slide};
    let mut voice = Voice::new(f32::from(RATE));
    let params = VoiceParams {
        cutoff_hz: 315.0,
        resonance: 0.88,
        env_oct: 3.0,
        decay_s: 0.25,
        overdrive_db: 18.0,
        accent: 0.82,
        slide_s: 0.080,
        ..VoiceParams::default()
    };
    // End times in ms. Quote the factory bass's two low Ds and A–C slide,
    // then climb G–A–D. Hold the accented fifth into the high tonic so the
    // pitch slews under one continuous filter envelope.
    let mut start = 0;
    for (end_ms, gate) in [
        (75, Accent(38)),
        (109, Off),
        (168, Pluck(38)),
        (ACCENT_A, Off),
        (SLIDE_C, Accent(45)),
        (329, Slide(48, false)),
        (FIRST_KNOCK, Off),
        (426, Pluck(43)),
        (451, Off),
        (HIGH_D, Accent(45)),
        (680, Slide(50, true)),
        (750, Off),
    ] {
        match gate {
            Accent(n) => voice.note_on(i32::from(n), true, false),
            Pluck(n) => voice.note_on(i32::from(n), false, false),
            Slide(n, accent) => voice.note_on(i32::from(n), accent, true),
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
    // Two quiet shell knocks lead into the resolution. The hat marks the landing.
    for (ms, speed, mut gain) in [(FIRST_KNOCK, 3, 0.25), (451, 2, 0.42)] {
        let mut snare = SnareDrum::new(u32::from(RATE), speed).expect("fixed snare clock");
        snare.trigger();
        for sample in &mut mix[frame(ms)..] {
            *sample += gain * f32::from(snare.next_frame()) / 32768.0;
            gain *= 0.9993;
        }
    }
    // Just the first 12 ms of the closed hat puts a metallic edge on the arriving tonic.
    let hat = Samples::X0XPROC1.closed_hat.as_chunks::<2>().0;
    let mut gain = 0.16;
    for (sample, &bytes) in mix[frame(HIGH_D)..].iter_mut().zip(hat).take(frame(12)) {
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
    println!("cargo:rerun-if-changed=assets/fonts.rs");
    println!("cargo:rerun-if-changed=assets/boot_timing.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build directory"));
        fonts::write_atlases(&out);
        println!("cargo:rustc-link-arg=--defsym=__wavegraph_job_reserve=4864");
        println!(
            "cargo:rustc-link-arg=-Map={}",
            out.join("wavegraph-fm1.map").display()
        );
    }
    let mut mix = vec![0.0; frame(750)];
    acid(&mut mix);
    knocks(&mut mix);
    // End the printed release at digital silence on the logo boundary.
    let fade = &mut mix[frame(740)..];
    let length = f32::from(u16::try_from(fade.len()).expect("ten millisecond fade"));
    for (remaining, sample) in (0_u16..).zip(fade.iter_mut().rev()) {
        *sample *= f32::from(remaining) / length;
    }
    let peak = mix.iter().copied().map(f32::abs).fold(0.0, f32::max);
    let scale = f32::from(i16::MAX) * 0.25 / peak;
    let mut pcm: Vec<_> = mix
        .iter()
        .map(|x| ((x * scale).round() as i16).to_le_bytes())
        .collect();
    // An exhausted cue mixes as silence. Omit trailing zero frames from flash.
    while pcm.last() == Some(&[0, 0]) {
        pcm.pop();
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    patterns::compile_banks(&out);
    #[cfg(feature = "packed-cymbals")]
    pack_cymbals(&out);
    // The cue shares flash with the banks. Independent 128-frame blocks allow
    // each DMA half to decode directly into the mix without a PCM buffer.
    let mut encoded = Vec::new();
    let mut offsets = Vec::new();
    for block in pcm.chunks(128) {
        offsets.push(u16::try_from(encoded.len()).expect("cue fits 64 KiB"));
        encode_block(block, &mut encoded);
    }
    fs::write(out.join("bumper.delta"), encoded).expect("write packed cue");
    fs::write(
        out.join("bumper_index.rs"),
        format!(
            "const FRAMES: usize = {}; const OFFSETS: &[u16] = flash_asset!(u16, &{offsets:?});",
            pcm.len()
        ),
    )
    .expect("write cue index");
    // The host regression test compares every decoded sample to the original mix.
    fs::write(out.join("bumper.s16le"), pcm.as_flattened()).expect("write boot cue");
}

#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Widths are 0..=16; the bit reservoir has at most 23 occupied bits and emits its low byte."
)]
fn encode_block(block: &[[u8; 2]], encoded: &mut Vec<u8>) {
    // Pack signed second differences at a fixed bit width per 16 frames.
    // Wrap at i16 precision so even a full-scale transition takes at most 16 bits.
    let mut previous = 0_i16;
    let mut slope = 0_i16;
    for group in block.chunks(16) {
        let residuals: Vec<u16> = group
            .iter()
            .map(|&bytes| {
                let sample = i16::from_le_bytes(bytes);
                let delta = sample.wrapping_sub(previous);
                let residual = delta.wrapping_sub(slope);
                previous = sample;
                slope = delta;
                ((residual << 1) ^ (residual >> 15)).cast_unsigned()
            })
            .collect();
        let width = (16 - residuals.iter().copied().max().unwrap_or(0).leading_zeros()) as u8;
        encoded.push(width);
        let mut bits = 0_u32;
        let mut count = 0;
        for residual in residuals {
            bits |= u32::from(residual) << count;
            count += width;
            while count >= 8 {
                encoded.push(bits as u8);
                bits >>= 8;
                count -= 8;
            }
        }
        if count != 0 {
            encoded.push(bits as u8);
        }
    }
}

#[cfg(feature = "packed-cymbals")]
fn pack_cymbals(out: &std::path::Path) {
    let kit = Samples::X0XPROC1;
    for (name, bytes) in [
        ("closed-hat", kit.closed_hat),
        ("open-hat", kit.open_hat),
        ("ride", kit.ride),
    ] {
        let (frames, rest) = bytes.as_chunks::<2>();
        assert!(rest.is_empty(), "whole PCM16 frames");
        let samples: Vec<_> = frames.iter().copied().map(i16::from_le_bytes).collect();
        let packed = fm1_assets::pcm::encode(&samples).expect("factory sample count");
        fs::write(out.join(format!("{name}.pcm1")), packed).expect("write packed cymbal");
    }
}
