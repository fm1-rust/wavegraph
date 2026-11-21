# wavegraph-monosynth

A monophonic oscillator for WaveGraph. It has one message inlet for notes,
velocity and waveform changes, and one mono signal outlet. Sine, triangle,
sawtooth and square share a continuous 32-bit phase accumulator. Events take
effect at their frame offset. Note-off silences the currently selected note
and the phase keeps advancing.

The oscillator and its output level come from the [FM-1 waveform example][voice].
Full velocity peaks at about −24 dBFS. Sawtooth and square have sharp edges
and can alias at high pitches. Supply a `Tuning` for the output sample rate.
The same oscillator can render to `f32` or directly to signed 24-bit PCM.
The [engine example][engine] plays the synth from a compiled patch.

## License

Licensed under the [MIT License][mit].

[voice]: https://github.com/fm1-rust/fm1-rust/blob/a4a770c8743cee972cfbfb5478145e5198be4550/fm1-rticx/examples/rticx-waveform/voice.rs
[engine]: https://github.com/fm1-rust/wavegraph/tree/main/wavegraph#rendering-a-compiled-patch
[mit]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph-monosynth/LICENSE
