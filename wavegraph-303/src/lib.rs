#![no_std]
#![deny(unsafe_code)]
#![doc = include_str!("../README.md")]

mod compressor;
mod drive;
mod drum_fx;
mod dsp;
mod effects;
mod reverb;
mod voice;

pub use compressor::Compressor;
pub use drive::Drive;
pub use drum_fx::{DrumFx, DrumFxSettings};
pub use effects::{Effects, EffectsSettings, LengthMismatch, StereoFrame};
pub use reverb::Reverb;
pub use voice::{Voice, VoiceParams};

/// Mechanically translated CHOMPI voice, DSP helpers and effects.
///
/// The pointer-based methods retain the C++ calling conventions. Buffer pointers
/// must cover the requested frame count and satisfy the input/output aliasing
/// requirements. Delay and reverb storage supplied to `Init` must stay valid and
/// exclusively available to that instance for every call that uses it. At
/// 48 kHz, the original engine uses 96,000 stereo delay frames and 65,536
/// mono reverb samples.
#[cfg(feature = "raw")]
// Keep translation-specific lint allowances off handwritten crate code.
#[allow(
    unsafe_code,
    nonstandard_style,
    unused,
    unsafe_op_in_unsafe_fn,
    missing_debug_implementations,
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    clippy::all,
    clippy::pedantic,
    clippy::arithmetic_side_effects,
    clippy::panic
)]
pub mod raw;
