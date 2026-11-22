# wavegraph-organ

A four-voice, heapless, `no_std` house organ for WaveGraph, **inspired by** the
Korg M1's Organ 2 factory preset (#17). This is an **original approximation**:
the M1 used PCM multisounds and proprietary program/effects data. No Korg
samples, ROM data, or firmware are bundled or required.

The DSP offers two switchable oscillator recipes: House uses approximate 1:3:6
harmonic ratios (from additive/FM community attempts). Analog follows a
published subtractive recipe: triangle/saw bass, triangle two octaves above,
and square three octaves above, in a 1:4:8 frequency ratio.
Both include a decaying key transient. The output is dry. The
four-voice ceiling avoids dynamic allocation, bounds the rendering cost, and
leaves headroom when used beside the TB-303 and monosynth.

## Controls

| Message | Effect | Initial setting |
| --- | --- | --- |
| `Voicing(House/Analog)` | Select harmonic model (ALGORITHM on FM-1) | House |
| `Tone(0..=127)` | Upper harmonic / brightness | 80 |
| `Percussion(0..=127)` | Attack click / punch | 44 |
| `DecayMs(5..=500)` | How long the click lasts | 70 ms |
| `ReleaseMs(0..=1000)` | How long released notes fade | 55 ms |

For house bass riffs, use short release, moderate punch, and a dry output. For
bright chord stabs, raise tone and release; chorus/reverb can be added *after*
the instrument so the bass doesn't get cloudy. The M1 organ was famously used
as a bassline/riff sound in StoneBridge's remix of Robin S's *Show Me Love*.

## Process API

One message inlet and one mono signal outlet, supporting signed 24-bit PCM
(`wavegraph_sample::I24`) and `f32`. Messages are applied immediately before
the given frame. Note-on for a sounding pitch retriggers that pitch; zero
velocity means note-off. When all four slots are in use, the oldest released
voice is reused first, otherwise the oldest held note is stolen. Late note-offs
for stolen notes are ignored. `AllNotesOff` keeps release tails, `Reset` kills
all voices. The instrument is allocation-free even during note stealing.

```rust
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_organ::{Organ, Message};
use wavegraph_sample::I24;

let mut organ = Organ::new(24_000_000, 544).expect("FM-1 audio clock");
let chord = [60, 64, 67].map(|note| Event {
    frame: 0, port: 0, message: Message::NoteOn { note, velocity: 110 }
});
let mut samples = [I24::default(); 128];
organ.process(&mut ProcessContext {
    frames: samples.len(), inputs: &[], outputs: &mut [&mut samples],
    events: &chord, emit: &mut Err,
});
assert_eq!(organ.active_voices(), 3);
```

## Performance and limitations

The render path uses Q16 envelope state, a 65-entry sine lookup table and integer
mixing; it performs no transcendental math and allocates nothing per block.
An approximate sine plus triangle harmonic model cannot null against the M1
PCM sound, particularly across the original keyboard's multisample zones.
The transient and Analog's upper square wave are not bandlimited; higher
registers can alias even though upper harmonics above Nyquist are suppressed.
Benchmark on FM-1 before adding chorus or effects.

## Research

- [StoneBridge interview on how he found preset 17](https://funktasy.com/exclusive-interviews/stonebridge/)
- [Korg's description of the M1 PCM engine](https://www.korg.com/us/products/software/kc_m1/)
- [Future Music's M1-style organ synthesis walkthrough](https://www.musicradar.com/tuition/tech/how-to-create-a-korg-m1-style-organ-bass-sound-642650)
- [Measured M1 plugin comparison and Surge XT recreation](https://www.reddit.com/r/synthrecipes/comments/18erzix/)
- [Independent 1:3:6 harmonic patch recipe](https://www.reddit.com/r/synthrecipes/comments/1rew8uf/korg_m1_organ_2_preset_creating_on_sylenth_1/)
- [MusicRadar on house-specific M1 production](https://www.musicradar.com/news/how-to-create-classic-house-sounds-with-the-korg-m1-and-when-to-use-them)

## License

Licensed under either the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT), at your option. The sine lookup was adapted from
`wavegraph-monosynth` and retains the FM-1 Rust Developers' MIT notice.

### Contribution

Unless you explicitly state otherwise, you license contributions intentionally
submitted for inclusion in this work under both licenses, without additional
terms or conditions.
