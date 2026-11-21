# WaveGraph for the FM-1

Play the [monosynth](../wavegraph-monosynth/) and [303 bass synth](../wavegraph-303/)
together on the M-VAVE FM-1. The firmware starts on the green monosynth page
with a sine selected and the cutoff at 5 kHz. Turn PRESETS one click in
either direction for the silver TB-303 page. Both instruments are playable
during the boot animation.

| Monosynth | TB-303 |
| --- | --- |
| ![Boot logo and green monosynth scope](assets/monosynth.gif) | ![Emulator capture of the silver TB-303 page during play](assets/tb-303.png) |

| Control | Action |
| --- | --- |
| PRESETS encoder | Select the instrument for new key presses, panel edits and the display. |
| Piano keys | Play F3 to G5. Each instrument sounds its most recently pressed key. |
| ALGORITHM encoder | Monosynth cycles through sine, triangle, sawtooth and square. The 303 switches between saw and square. |
| SELECT encoder | Transpose the selected instrument up or down by up to 12 semitones. |
| Knob 1 on the monosynth | Sweep the low-pass cutoff from 10 Hz to 20 kHz. Six detents span an octave. The header shows the cutoff in Hz. |
| Knobs 1–4 on the 303 | Set cutoff, resonance, envelope modulation and decay. The silver panel shows the last knob's value. |
| MASTER | Set the volume of the stereo mix. |
| PLAY/STOP | Reserved for a future sequencer. |
| FX | Switch the mix between dry and half gain. |
| SEL button | Switch the scope between the selected voice before FX and MASTER (VCF on the monosynth, SRC on the 303) and the stereo mix after MASTER (DAC). |
| Both OCTAVE buttons | Hold for two seconds to return to USB download mode. |

Hold a monosynth note, turn PRESETS, then play a different key for the 303.
Both notes sound until you release their respective keys. Within each part,
releasing the newest key recalls the previous key and its strike velocity.
On the 303, strikes at velocity 96 and above trigger accent. Overlapping keys
slide, and a fresh strike after release restarts the envelopes. Turn knob 1 on
either page to adjust that instrument’s cutoff while both play.

## Audio and display

Every 128-frame ALNK0 block calls both persistent WaveGraph nodes. The graph
trims the 303 to quarter gain, adds both voices, and sends the mix to both
stereo channels. FX uses the async `EditQueue` to switch the gain stage
between blocks. The edit is acknowledged when DMA accepts the first block
using the new route.

The scope captures 240 consecutive samples starting at a rising zero crossing.
A two-block queue feeds an async observer. Missing blocks and routing changes
start a fresh trace. The trace labels travel with the samples. The cutoff
readout follows the knob even while the scope waits for a trigger. A red LATE
indicator latches if audio service misses its timing budget.

The splash and instrument pages use [fm1-graphics] to draw a waveform on a
24-pixel grid. The boot illustration combines two harmonics, with phase
modulation. The logo
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
