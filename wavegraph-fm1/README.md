# WaveGraph for the FM-1

Play three instruments together on the M-VAVE FM-1: the green
[monosynth](../instruments/wavegraph-monosynth/), charcoal-and-lime four-voice
[M1-inspired organ](../instruments/wavegraph-organ/), and silver
[TB-303](../instruments/wavegraph-303/). PRESETS cycles them in that order, one detent
at a time, with a transient three-row patch-selection overlay. Each part
keeps sounding when you select another. The default page is the green
monosynth, with sine selected and the cutoff at 5 kHz.

| Monosynth | 303 bank 1 | 303 bank 2 |
| --- | --- | --- |
| ![Boot logo and green monosynth scope](assets/monosynth.gif) | ![Silver 303 scope with decay on knob 4](assets/tb-303.png) | ![Silver 303 scope with overdrive on knob 4](assets/tb-303-voice.png) |

| Control | Action |
| --- | --- |
| PRESETS encoder | Cycle through monosynth → organ → TB-303. Each detent updates the panel and routes new keys to the highlighted instrument. Held notes stay with their original part. |
| Piano keys | Play F3 to G5 at the default octave. Each key stays lit while you hold it. The organ has four voices. The monosynth and 303 each play one note at a time. |
| OCT− / OCT+ | Lower or raise the piano roll by one octave, up to three either way. The setting follows you across presets and pages. Held keys keep their pitch. The button for the active direction blinks slowly at one octave, blinks faster at two and stays lit at three. |
| ALGORITHM encoder | Change mono waveform, switch organ House/Analog harmonic recipes, or choose 303 saw/square. |
| SELECT encoder | Transpose monosynth/organ by ±12 semitones, or switch the 303's knob bank. |
| Knob 1 on the monosynth scope | Sweep the low-pass cutoff from 10 Hz to 20 kHz. Six detents span an octave. The header shows the cutoff in Hz. |
| Knobs 1–4 on the organ scope | Tone, key percussion, transient decay (5–500 ms), release (0–1000 ms). The charcoal panel shows the last-touched value. |
| ENV | Open or close the selected instrument’s envelope page. ENV stays lit while the page is open. |
| Knobs 1–4 in monosynth or organ ENV | Adjust attack, decay, sustain and release for the selected part. |
| Knobs 1–4 on the 303 | Edit the four assignments shown above the scope. The active knob is highlighted and its value appears below the trace. |
| MASTER | Set the volume of the stereo mix. |
| PLAY/STOP | Reserved for a future sequencer. |
| FX | Open or close the global reverb view. The FX button stays illuminated while that page is visible. |
| Knobs 1–4 in FX | Monosynth send, 303 send, decay and wet return level (0–128, except decay 0–127). |
| ALGORITHM encoder in FX | Switch the existing full/half output-gain graph using an async edit. |
| SEL button in FX | Mute or unmute the wet return without clearing the reverb tail. |
| SEL button (instrument view) | Switch the scope between the selected voice (VCA for mono, ORG for organ, SRC for 303) and the post-MASTER DAC mix. |
| Both OCTAVE buttons | Press together to return the piano roll to its default octave. Hold both for two seconds to enter USB download mode. |

| 303 bank | Knob 1 | Knob 2 | Knob 3 | Knob 4 |
| --- | --- | --- | --- | --- |
| B1/2 | Cutoff | Resonance | Filter env mod | Filter decay |
| B2/2 | Tuning (±12 semitones, 10 cents per click) | VCA attack (0–60 ms) | Accent amount (0–100%) | Overdrive (0–36 dB into the filter) |

SELECT updates the bank indicator and all four labels, including when the keys
are idle. Returning to a bank recalls its settings and last-touched knob.

Hold a monosynth note, turn PRESETS once to the organ, and play a four-note
chord. Turn PRESETS again to the 303 and play a bassline: all three parts
continue together. Keys retain the part they were pressed on. On the organ,
the latest four physical keys take priority, and releasing a newer key
restores an older held key. The monosynth and 303 preserve their existing
last-note priority. On the 303, velocity 96+ triggers accent; overlapping
notes slide and a new strike after release restarts the envelopes.

## M1-inspired organ

The organ is **modeled, not sampled**. ALGORITHM selects either a 1:3:6
harmonic recipe (House) or a brighter 1:4:8 blend (Analog). Both include
short key percussion with its own decay and a separate per-voice amplitude envelope.
The modeling follows independent community recipes, not Korg's actual PCM
multisample. No Korg ROM data is bundled. For a proper similarity check,
compare a dry Organ 2 patch in the Korg M1 plug-in against both modes; a
spectrum and envelope comparison would be more informative than a single
house-music recording. The organ is initially **dry**; the global reverb
continues to service the other two parts unchanged.

The organ's dark charcoal panel with lime accents and a cyan readout takes
visual inspiration from the [KORG Collection M1 V2 UI][korg-m1]. The PRESETS
list is inspired by the FM-1's stock patch-selector overlay. A bright outline
and dark fill mark the selected row over the dimmed instrument page. The scope
and controls stay live while browsing. The list closes one second after the
last turn.

## VCA envelope

Hold a key and turn Knob 3 to hear the sustain level change. Release the key
and turn Knob 4 to shorten or extend the tail. The graph and readouts also
respond while the keyboard is idle. Press ENV again to return to the scope.
Switching instruments with PRESETS closes the editor. Each part recalls its own settings.

| Knob | Parameter | Monosynth range (initial) | Organ range (initial) |
| --- | --- | --- | --- |
| 1 | Attack | 1.5 ms–4 s (8 ms) | 0–4 s (2 ms) |
| 2 | Decay | 2 ms–10 s (160 ms) | 0–10 s (100 ms) |
| 3 | Sustain | 0–100% (70%) | 0–100% (100%) |
| 4 | Release | 2 ms–10 s (125 ms) | 0–1 s (55 ms) |

On the organ, knob 4 edits the same release in either view, in 5 ms steps.
Knob 3 on the scope sets the key click's decay. On the ENV page, lower sustain
to hear knob 2 shape the decay of the whole note.

The monosynth ranges follow the [Roland SH-101][sh101]. The editor scales each
stage's width by knob position so you can see a short attack next to a long
release. The flat section shows the level held until key-up.

![FM-1 VCA envelope editor showing the curve and all four settings](assets/vca-envelope.png)

## 303 envelope preview

Press ENV on the silver 303 page. The display plays a silent reference phrase
through a separate instance of the voice’s envelopes. PLAIN marks one ordinary
strike. The three A marks are accented strikes 96 ms apart. Each vertical grid
division is 96 ms. Turn SELECT to switch knob banks while the preview is open.

| Trace | What to try |
| --- | --- |
| VCF · filter cutoff (62.5 Hz–16 kHz, logarithmic) | Raise Env Mod to deepen the sweep. Decay stretches the plain strike. Accented strikes keep their short filter decay, about 200 ms to 1% in this model. |
| ACC · accent contribution to cutoff (0–3 octaves) | Raise Accent on bank 2, then Resonance on bank 1. The later accents start with charge left by the previous strike. Higher resonance slows the drain. |
| VCA · amplitude gain (0–1.9) | Turn bank 2 knob 2 to lengthen the attack. Accent also boosts the gain. |

To isolate the rounded sweep, turn Env Mod down to zero and raise Accent and
Resonance. On the original 303, some direct filter modulation remains at
minimum Env Mod.

The hardware uses a second section of the resonance pot and a 1 µF capacitor
in its accent sweep. WaveGraph approximates that branch with a 12 ms charge
time constant and an 80–430 ms drain. The preview uses the same equations as
the audio voice. [Robin Whittle’s circuit description][accent-sweep]

| Main envelope and accents | Accent sweep with Env Mod at zero |
| --- | --- |
| ![303 preview with filter decay and rising accent peaks](assets/tb-303-envelope.png) | ![303 preview with the accent sweep isolated and a 30 ms VCA attack](assets/tb-303-accent-sweep.png) |

## Global reverb

FX and ENV are mutually exclusive views; opening either closes the other.
The FX view stays global while PRESETS changes the selected instrument.
The instrument's own knob assignments remain unchanged outside FX.
The global page always scopes the final DAC mix, regardless of the instrument
scope-tap selection.

The independent send levels are initially 32/128 (monosynth) and 24/128
(303). The organ currently bypasses the reverb sends and joins the dry mix.
Disable the return with SEL to hear dry audio while the reverb continues
processing. The same reverb tank stays alive across full/half graph edits.
This first mono, fixed-point effect uses approximately 17 KiB of static delay
storage; it is not an emulation of the Roland DEP-5.

## Audio and display

Every 128-frame ALNK0 block calls three persistent instrument nodes, a shared
send mixer and a reverb processor. WaveGraph bytecode trims the 303 to quarter
gain, creates independent monosynth and 303 sends, and adds the dry organ and
the wet return before duplicating the mix to both stereo channels.
FX's ALGORITHM encoder retains the async `EditQueue` to switch the output
gain program between blocks, with an acknowledgement once DMA accepts the
first block under that route.

The scope captures 240 consecutive samples starting at a rising zero crossing.
A two-block queue feeds an async observer. Missing blocks and routing changes
start a fresh trace. The trace labels travel with the samples. The cutoff
readout follows the knob even while the scope waits for a trigger. A red LATE
indicator latches if audio service misses its timing budget.

The splash and instrument pages use [fm1-graphics] to draw a waveform on a
24-pixel grid. The boot illustration combines two harmonics with phase
modulation. When the green WaveGraph title finishes bouncing, the pink FM-1
lettering rises from the bottom. The firmware version appears beneath it,
taken from the `wavegraph-fm1` Cargo package at build time.

The 208 × 36 title and 104 × 38 model name occupy 2,860 bytes of packed
artwork. Both draw in buffered RGB565 strips. During play, dirty tiles update
the trace and a narrow phosphor sweep highlights it. Display DMA runs below
audio and panel scanning. To change the lettering, edit `assets/wavegraph.png`
or `assets/fm1.png` and run
`python3 wavegraph-fm1/assets/pack_logo.py` with Pillow installed.

## Build and flash

The [firmware workflow] builds a `wavegraph-fm1` artifact containing
`wavegraph-fm1.fwsc` for M-Flasher and the [browser emulator], plus a raw
`wavegraph-fm1.bin` for emulators that accept an application image (such as
[fm1-emulator]). For a local build, follow the
[FM-1 toolchain setup] for `RUSTC`, `PI32V2_BIN` and `PI32V2_ISA`, and set
`PI32V2_TARGET` to the SDK's `fm1/targets/pi32v2-none.json`. From the
WaveGraph workspace root run

```sh
./wavegraph-fm1/build.sh
```

Both files are written to `target/pi32v2-none/firmware/`.
Cargo resolves the FM-1 SDK from crates.io, including `fm1` 0.1.3 and
`fm1-rticx` 0.1.1. CI uses the SDK’s shared pi32v2 build workflow to package
the flashable image.

To cut a release, update the version in `wavegraph-fm1/Cargo.toml` and
`Cargo.lock`, then push a `wavegraph-fm1/vX.Y.Z` tag for that commit. The
[release workflow] checks the tag against the Cargo version, tests and builds
the firmware, then attaches versioned `.fwsc` and `.bin` files to a GitHub
Release. Open the `.fwsc` in the [browser emulator] or run it with
`fm1-live --fwsc`; no additional conversion is needed for that emulator.

## License

The firmware source is available under the [Apache License, Version 2.0](LICENSE-APACHE)
or the [MIT License](LICENSE-MIT).

[sh101]: https://support.roland.com/hc/en-us/articles/201921519-SH-101-Technical-Specifications
[korg-m1]: https://www.korg.com/us/products/software/kc_m1/
[fm1-graphics]: https://docs.rs/fm1-graphics/0.1.0/fm1_graphics/
[firmware workflow]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-fm1.yml
[release workflow]: https://github.com/fm1-rust/wavegraph/actions/workflows/release-fm1.yml
[browser emulator]: https://joakimeriksson.github.io/fm1-emu/
[fm1-emulator]: https://github.com/simonjohansson/fm1-emulator
[FM-1 toolchain setup]: https://github.com/fm1-rust/fm1-rust/blob/953165a87c12fac5bc1e2897d13e38a3045bbf69/fm1/README.md#build-and-flash

[accent-sweep]: https://www.firstpr.com.au/rwi/dfish/303-unique.html
