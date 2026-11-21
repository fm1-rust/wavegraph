# WaveGraph for the FM-1

Play a four-waveform monosynth through the WaveGraph bytecode engine on the
M-VAVE FM-1. The WaveGraph wordmark drops across a generated waveform, swings
past its centerline, and settles there for half a second before the live scope
appears. The synth starts silent with a sine wave selected. You can play
during the splash.

![Emulator capture of the boot logo, followed by the live monosynth and scope](assets/monosynth.gif)

| Control | Action |
| --- | --- |
| Piano keys | Play F3 to G5 at strike velocity. The newest held key sounds. |
| PRESETS encoder | Cycle through sine, triangle, sawtooth and square. |
| SELECT encoder | Transpose by semitones, up to one octave either way. |
| Knob 1 | Sweep the low-pass cutoff from 10 Hz to 20 kHz. Six detents span an octave. |
| MASTER | Set output volume. |
| PLAY/STOP | Unassigned (reserved for a future sequencer). |
| FX | Switch between the dry patch and a half-gain stage. |
| SEL button | Switch the scope between the filtered voice (VCF) and the post-MASTER output (DAC). |
| Both OCTAVE buttons | Hold for two seconds to return to USB download mode. |

Overlap two keys and release the newer one to hear the earlier note at its
original velocity. Releasing the last key silences the voice. The footer shows
the sounding pitch and its velocity (V).

Select sawtooth, hold a key and turn Knob 1 counterclockwise to hear the
upper harmonics fade. The header shows the cutoff in Hz, starting at 5000.

While a note is sounding, press FX. The DAC trace shrinks to half its height.
Press SEL to see the filtered voice at the key's original level while the output
continues at half gain.

## Audio and display

A single [Monosynth](../wavegraph-monosynth/) supplies both stereo channels.
The ALNK0 interrupt renders 128 frames at a time. The input task awaits FX
edits through WaveGraph's `EditQueue`. The audio task changes the program
between blocks, keeps the oscillator's phase, and acknowledges the edit when
the first block using the new program is submitted to DMA.

The scope captures 240 consecutive samples starting at a rising zero crossing.
A two-block queue feeds an async observer. Missing blocks and routing changes
start a fresh trace. The trace labels travel with the samples. The cutoff
readout follows the knob even while the scope waits for a trigger. A red LATE
indicator latches if audio service misses its timing budget.

Both screens use [fm1-graphics] to draw a waveform on a 24-pixel grid. The
boot illustration combines two harmonics, with phase modulation. The logo
slides over it in buffered RGB565 strips. During play, dirty tiles update the
trace and a narrow phosphor sweep highlights it. Display DMA runs below audio
and panel scanning. The 208 × 36 logo occupies 1,872 bytes in firmware.
To change it, replace `assets/wavegraph.png` and run
`python3 wavegraph-fm1/assets/pack_logo.py` with Pillow installed.

## Build and flash

The [firmware workflow] builds a `wavegraph-fm1` artifact containing
`wavegraph-fm1.fwsc` for M-Flasher. For a local build, follow the
[FM-1 toolchain setup] for `RUSTC`, `PI32V2_BIN` and `PI32V2_ISA`, and set
`PI32V2_TARGET` to the SDK's `fm1/targets/pi32v2-none.json`. From the
WaveGraph workspace root run

```sh
./wavegraph-fm1/build.sh
```

The image is written to `target/pi32v2-none/firmware/wavegraph-fm1.fwsc`.
Cargo resolves the FM-1 SDK from crates.io, including `fm1` 0.1.3 and
`fm1-rticx` 0.1.1. CI uses the SDK’s shared pi32v2 build workflow to package
the flashable image.

## License

The firmware source is available under the [Apache License, Version 2.0](LICENSE-APACHE)
or the [MIT License](LICENSE-MIT).

[fm1-graphics]: https://docs.rs/fm1-graphics/0.1.0/fm1_graphics/
[firmware workflow]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-fm1.yml
[FM-1 toolchain setup]: https://github.com/fm1-rust/fm1-rust/blob/953165a87c12fac5bc1e2897d13e38a3045bbf69/fm1/README.md#build-and-flash
