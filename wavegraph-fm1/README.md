# WaveGraph for the FM-1

The M-VAVE FM-1 plays a [909 drum machine](../instruments/wavegraph-909/) alongside three keyboard instruments, the green
[monosynth](../instruments/wavegraph-monosynth/), charcoal-and-lime four-voice
[M1-inspired organ](../instruments/wavegraph-organ/), and silver
[TB-303](../instruments/wavegraph-303/). PRESETS cycles the monosynth, organ,
303 and 909 with a four-row patch selector. Each part
keeps sounding when you select another. The default page is the green
monosynth, with sine selected and the cutoff at 5 kHz. The shared transport
starts stopped at 122 BPM. PLAY starts an original four-bar deep-house sketch:
quarter-note 909 kicks, sustained M1-style chords, and an accented, sliding 303 with automated
cutoff, resonance and overdrive. A sparse monosynth counterline is preloaded
but muted initially.

| Monosynth | 303 bank 1 | 303 bank 2 |
| --- | --- | --- |
| ![Boot logo and green monosynth scope](assets/monosynth.gif) | ![Silver 303 scope with decay on knob 4](assets/tb-303.png) | ![Silver 303 scope with overdrive on knob 4](assets/tb-303-voice.png) |

| Control | Action |
| --- | --- |
| PLAY/STOP | Start all enabled patterns on beat one from any page (green). Press again to release sequenced notes, silence the kit and stop (red). Live held keys keep sounding. |
| SEQ | View the selected instrument's preset pattern. SEQ or HOME returns to the instrument. |
| PRESETS in SEQ | Browse monosynth → M1 organ → TB-303 → TR-909 while keeping the sequence view open. |
| KNOB 2 in SEQ | Browse the four bars while stopped. While playing, the view follows the audio playhead. |
| SEL in SEQ | Mute/unmute the displayed pattern. The other patterns and master clock continue. |
| SELECT in SEQ | Turn clockwise for the tempo page, then use KNOB 3 for 30–300 BPM in 1 BPM steps. Turn SELECT back for the pattern. |
| PRESETS encoder | Cycle through monosynth → organ → TB-303 → TR-909. Held notes stay with their original part. |
| Piano keys | Play F3 to G5 at the default octave. Each key stays lit while you hold it. The organ has four voices. The monosynth and 303 each play one note at a time. |
| OCT− / OCT+ | Lower or raise the piano roll by one octave, up to three either way. The setting follows you across presets and pages. Held keys keep their pitch. The button for the active direction blinks slowly at one octave, blinks faster at two and stays lit at three. |
| ALGORITHM encoder | Change mono waveform, switch organ House/Analog harmonic recipes, or choose 303 saw/square. |
| SELECT encoder | Transpose monosynth/organ by ±12 semitones, or switch the 303's knob bank. |
| Knob 1 on the monosynth scope | Sweep the low-pass cutoff from 10 Hz to 20 kHz. Six detents span an octave. The header shows the cutoff in Hz. |
| Knobs 1–4 on the organ scope | Tone, key percussion, transient decay (5–500 ms), release (0–1000 ms). The charcoal panel shows the last-touched value. |
| ENV | Open or close the selected melodic instrument’s envelope page. ENV stays lit while the page is open. |
| Knobs 1–4 in monosynth or organ ENV | Adjust attack, decay, sustain and release for the selected part. |
| Knobs 1–4 on the 303 | Edit the four assignments shown above the scope. The active knob is highlighted and its value appears below the trace. |
| MASTER | Set the volume of the stereo mix (independent of the mixer faders). |
| GLO | Open or close the four-strip mixer; its lamp stays on while the mixer is visible. HOME closes the page. |
| SELECT encoder in mixer | Cycle through LEVEL, REVERB and DELAY for all four knobs. |
| Knobs 1–4 in mixer | Set tracks 1–4 independently, 0–128: monosynth, organ, 303, and TR-909. |
| FX | Turn SELECT to choose REVERB, DELAY or 303 / COMP. |
| Knobs 1–2 on FX / REVERB | Reverb decay (0–127) and wet return (0–128). |
| Knobs 1–3 on FX / DELAY | Delay time (5–1,000 ms), feedback (0–95%) and wet return (0–128). |
| Knobs 1–3 on 303 / COMP | Choose the sidechain source, set depth (0–100%) and set the low-shelf corner (40–1000 Hz). |
| ALGORITHM encoder in FX | Switch the existing full/half output-gain graph using an async edit. |
| SEL button in FX | Toggle the selected effect. Reverb and delay tails keep decaying. On 303 / COMP, SEL bypasses the insert. |
| SEL button (instrument view) | Switch the scope between the selected voice (VCA for mono, ORG for organ, SRC for 303, KIT for 909) and the post-MASTER DAC mix. |
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
house-music recording. The factory mix sends a little of the organ to the shared reverb.

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
| 1 | Attack | 1.5 ms–4 s (8 ms) | 0–4 s (4 ms) |
| 2 | Decay | 2 ms–10 s (160 ms) | 0–10 s (180 ms) |
| 3 | Sustain | 0–100% (70%) | 0–100% (68%) |
| 4 | Release | 2 ms–10 s (125 ms) | 0–1 s (180 ms) |

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

## TR-909 instrument view

Turn PRESETS one detent left from the monosynth to open the 909. The scope
shows the kit before its channel fader and effect sends. SEL switches the
trace to the final DAC mix while the lamps keep following the kit. The orange pulse
follows the transport. Each key's lamp flashes at the attack and glows dimly
until that voice finishes. A closed hat cuts off an open hat immediately.

Press PLAY to watch the pattern, or tap the first six white piano keys to play
the kit yourself, even with the transport stopped. From left to right, F3, G3,
A3, B3, C4 and D4 play kick, snare, clap, closed hat, open hat and ride. These
physical pad assignments stay in place across octave changes. SEQ opens the
drum pattern and HOME returns to the same live view.

## Preset sequencer and master tempo

SEQ opens on the instrument selected with PRESETS. The compact grid shows
one bar of sixteen steps, with the current step highlighted during playback.
Pitched tracks show their occupied note lanes and gate lengths. The 303 adds
`A` accent and `S` outgoing-slide marks plus `CUT`, `RES` and `DRV` curves.
The drum view shows all six voices: kick, snare, clap, closed/open hat and ride.
Only changed playhead columns are redrawn between bar changes.

The original factory sketch moves from D minor seventh to C major seventh.
The organ plays A3–C4–F4 and G3–B3–E4. The 303 supplies the roots below
those voicings. The long offbeat chords have a 92% sustain level and 200 ms
release. Shorter stabs answer them at the end of bars one and three.
The monosynth has a separate, quiet counterline; enable it with SEL in its SEQ
view. This spike provides fixed presets and track mutes, with no recording,
step editing or pattern saving yet. Edit `src/sequencer/pattern.rs` to change
the notes, gates, accents, slides and automation control points.

The 303's cutoff sweeps from 180 to 1800 Hz, resonance from 74 to 94%, and
overdrive from 12 to 28 dB over the four bars. Half-bar control points interpolate
at an audio-clocked rate of one update per 128 samples, including across loop
boundaries. Its filter envelope adds another three octaves, with 330 ms decay
and 85% accent. The preset uses the existing 303 drive into the filter.

While its pattern plays, the sequencer owns those three automated controls.
The other voice controls remain live. Moving an automated knob retains its
panel setting for when the track stops, is muted, or is taken over by live keys.
Pressing keys temporarily takes over that melodic track; the other tracks
continue. After key-up, the track rejoins at its next scheduled note. Browsing
the drum pattern leaves the live keyboard on the last selected melodic part.

The 909 repeats one bar. The kick hits all four beats, a short closed hat
plays each offbeat, and snare and clap strike together on two and four.
The kick blends its clean body with a saturated copy for extra low-mid
harmonics. The kit runs its modeled amplitude decays at 55% of stock.

The tempo control follows [the stock FM-1 sequencer][stock-seq]. Page 2 shows
the current BPM and PLAY/STOP state. Tempo edits retain the position within
the current sixteenth. One rational clock advances with the FM-1 audio frames
and retains the fractional sample remainder across DMA blocks. Notes, short
gates, slides, drum hits and automation therefore share the same transport;
panel scans and LCD redraws do not quantize the beat. The bundled 44.1 kHz
cymbals play at the hardware rate of about 44,118 Hz, a pitch difference of
roughly 0.7 cent.

## Audio overloads and USB log

A seven-pixel dot in the upper-right corner turns amber on a missed audio
deadline and red on a second miss within one second. It disappears after one
second without a miss. The `V` readout is the last key velocity.

Connect USB and run `fm1 log` from the [FM-1 CLI][cli]. For example,
`XR12 T4870 W3070 G2902 DMA` reports the 12th miss, 4,870 ms after boot,
with 3,070 µs of interrupt work and 2,902 µs since the previous audio IRQ.
`DMA` means the refill crossed a DMA boundary. The nominal 128-frame budget
is 2,901 µs. Timestamps have 10 ms resolution. Work and gap values saturate
at 99,999 µs. The snapshot holds the latest one or two events that fit the
USB log packet, including misses while disconnected. Repeated reads preserve
the snapshot until another miss occurs.

The graph skips the oscillator and filter work of silent keyboard parts.
Held notes, VCA release tails and both shared effects keep running when you
change presets. The 303 still discharges its accent capacitor while silent.

[cli]: https://github.com/fm1-rust/fm1-rust/tree/main/fm1-cli

## Four-track mixer, reverb and delay

Press GLO to enter the mixer. Its four horizontal strips show the selected
bank as values and bars. Turn SELECT to cycle through LEVEL, REVERB and
DELAY. Knobs 1–4 edit the monosynth, M1 organ, 303 and 909 respectively.
Press GLO again or HOME to leave. GLO, FX, ENV, and SEQ are mutually exclusive.
PRESETS continues to change the keyboard's selected instrument without
changing the mixer page or its bank. All four strips are live, including
the sequenced TR-909.

| Factory mix (128 = unity) | Mono | M1 organ | TB-303 | TR-909 |
| --- | ---: | ---: | ---: | ---: |
| Fader | 128 | 128 | 112 | 80 |
| Reverb send | 32 | 36 | 24 | 0 |
| Delay send | 0 | 0 | 92 | 0 |

**Both sends are post-fader.**
Lowering a track fader reduces the new signal it feeds to either effect.
Existing tails continue to play. All gains are 0–128, with 128 representing unity.

To change the 303 echo level, open GLO, turn SELECT to DELAY and turn knob 3.
For the 909, raise knob 4. Open FX and turn SELECT to DELAY to adjust the
shared echo. Knob 1 changes the time in 5 ms steps (initially 369 ms, a dotted
eighth at 122 BPM). The delay time stays fixed when tempo changes. Knob 2
sets feedback (initially 35%), and knob 3 sets the wet return (initially
64/128). Zero feedback gives one repeat.

On FX / REVERB, knobs 1 and 2 set decay and wet level. SEL mutes the displayed
return, and its LED shows whether that return is enabled. Both effects keep
their state across full/half gain edits. FX scopes the final DAC mix.

### Hear the kick duck the bass

Press PLAY, open FX, and turn SELECT to **303 / COMP**. Knob 1 starts on
**909 KICK**, the isolated kick before its fader. Keep that source selected and
turn knob 2 from 0% to 100%. At 0% the insert keeps its makeup gain, so you
can compare depths at the same level between kicks. Listen to the bottom of
the 303 dip on each quarter and come back between hits. Knob 3 moves the shelf
up in frequency to duck more of the squelch as well. SEL compares the entire
insert, including makeup gain, with the dry 303.

Try **M1 ORGAN** as the key. The bass now makes room for chord stabs, even
with the organ fader down. Muting its SEQ track lets its release finish and
the ducking subside. **909 KIT** includes hats and claps in the detector.
**MONO** follows the monosynth and **303 SELF** follows the acid line's own
accents. Only the 303 is processed. The factory setting is 75% depth at
180 Hz, with a 2 ms attack and 90 ms recovery for full-depth moves.

## Audio and display

Every 128-frame ALNK0 block first merges frame-stamped preset events with live
panel messages in bounded queues, ordering releases before new strikes. The
merged events feed three persistent synths, the 909, and a four-input Q23 mixer
with dry, reverb-send and delay-send outputs.
One reverb and one mono digital delay run in parallel. The master mix adds
both wet returns to the dry sum before output gain and stereo duplication.
The delay borrows one second of PCM16 storage (88,236 bytes) and skips empty
blocks once its line is silent.

The hot 303 preset reserves 16-fold output headroom before converting to Q23.
Its sidechain insert sits between the synth and the channel fader, so both
post-fader sends hear the shaped acid line. The GLO faders and sends apply
to live and sequenced notes.
FX's ALGORITHM encoder retains the async `EditQueue` to switch the output
gain program between blocks, with an acknowledgement once DMA accepts the
first block under that route.

The scope captures 240 consecutive samples starting at a rising zero crossing.
A two-block queue feeds an async observer. Missing blocks and routing changes
start a fresh trace. The trace labels travel with the samples. The cutoff
readout follows the knob even while the scope waits for a trigger.

The splash and instrument pages use [fm1-graphics] to draw a waveform on a
24-pixel grid. The boot illustration combines two harmonics with phase
modulation. When the green WaveGraph title finishes bouncing, the pink FM-1
lettering rises from the bottom. A 750 ms cue starts on that landing. An
accented 303 slides up an octave and back down, followed by two pitched 909
snare strokes and a touch of closed hat. MASTER controls the cue throughout.
Turning it fully down skips the rest of the cue for that boot. The firmware
version beneath the logo comes from the `wavegraph-fm1` Cargo package.

The 208 × 36 title and 104 × 38 model name occupy 2,860 bytes of packed
artwork. Both draw in buffered RGB565 strips. The scope stays live while the transport is stopped. During playback, a
narrow phosphor pulse crosses the trace once per quarter note, starting at
the left edge with the kick. It follows the audio master clock across tempo
changes and clears on STOP. Display DMA runs below
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

[stock-seq]: https://fuleo.github.io/fm1-guide/sequencer.html
