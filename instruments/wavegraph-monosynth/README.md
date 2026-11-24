# wavegraph-monosynth

A monophonic synthesizer for WaveGraph. It has one message inlet for notes,
velocity, waveform, filter cutoff and VCA envelope, and one mono signal outlet. Sine, triangle,
sawtooth and square share a continuous 32-bit phase accumulator. Events take
effect at their frame offset. The oscillator keeps advancing through note-off.

Send `Message::CutoffHz(1_000)` to set the cutoff to 1 kHz. A sine wave at
that frequency drops by about 3 dB. The [SH-101]-inspired filter uses four
cascaded poles with resonance fixed at zero. It starts fully open and smooths
cutoff changes. The VCA scales the filtered signal by strike velocity and
envelope gain.

Use a VCA envelope message for a quick attack and a short tail.

```rust
use wavegraph_monosynth::{Adsr, Message};

let envelope = Message::VcaEnvelope(Adsr {
    attack_us: 8_000,
    decay_us: 160_000,
    sustain_percent: 70,
    release_us: 125_000,
});
```

The monosynth uses [wavegraph-adsr](../../wavegraph-adsr/) for its VCA.
Attack rises linearly to peak gain. Decay and release follow quadratic
curves to sustain and silence. Overlapping notes share a gate. Releasing the
current note starts the tail at its present level, and a fresh attack can
start from that tail. Adjusting a time mid-stage rescales the remaining
duration. Adjusting a held sustain level slews it over 4 ms. The default
`Adsr::GATE` gives immediate attack and release at full sustain.

The oscillator and its output level come from the [FM-1 waveform example][voice].
Full velocity peaks at about −24 dBFS. Sawtooth and square have sharp edges
and can alias at high pitches. Supply a `Tuning` for the output sample rate.
The same oscillator can render to `f32` or directly to signed 24-bit PCM.
The [engine example][engine] plays the synth from a compiled patch.

## License

Licensed under either of

- [Apache License, Version 2.0][apache]
- [MIT License][mit]

at your option.

The oscillator adapted from the [FM-1 waveform example][voice] carries the
FM-1 Rust Developers' [MIT notice][mit]. Retain that notice with copies or
substantial portions of the oscillator under either option.

### Contribution

Unless you explicitly state otherwise, you license any contribution you
intentionally submit for inclusion in this work (as defined in the Apache-2.0
license) under both licenses above, without additional terms or conditions.

[voice]: https://github.com/fm1-rust/fm1-rust/blob/a4a770c8743cee972cfbfb5478145e5198be4550/fm1-rticx/examples/rticx-waveform/voice.rs
[engine]: https://github.com/fm1-rust/wavegraph/tree/main/wavegraph-dsp#rendering-a-compiled-patch
[SH-101]: https://support.roland.com/hc/en-us/articles/201921519-SH-101-Technical-Specifications
[apache]: https://github.com/fm1-rust/wavegraph/blob/main/instruments/wavegraph-monosynth/LICENSE-APACHE
[mit]: https://github.com/fm1-rust/wavegraph/blob/main/instruments/wavegraph-monosynth/LICENSE-MIT
