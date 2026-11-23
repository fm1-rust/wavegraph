# wavegraph-909

A six-voice TR-909-style percussion group. `Tr909` combines a modeled kick,
snare and hand clap with three [one-shot PCM cymbals](../wavegraph-sampler/). Pass
`Samples::X0XPROC1` for the bundled 44.1 kHz kit, or supply your own mono
s16le cymbals. `Tr909::with_sample_rate` accepts a rational graph clock, such
as 24,000,000 / 544 on the FM-1. Supply clips at that rate for native-pitch
playback. An empty slice mutes that cymbal.

Send a `Voice` to message inlet 0. A strike retriggers that voice on the
addressed frame. The hats choke each other. Simultaneous open and closed
strikes select the open hat, following the [ReBirth 909][rebirth]. The ride
keeps ringing across hat strikes. Roland's [service notes][service] describe the
original cymbal recordings and the analog circuits that shaped their envelopes.
An individual-output capture includes that shaping at its recorded settings.

| Signal outlet | Audio |
| --- | --- |
| 0 | Mono mix at unity gain, saturated after summing all voices |
| 1 | Kick |
| 2 | Snare |
| 3 | Hand clap |
| 4 | Closed hi-hat |
| 5 | Open hi-hat |
| 6 | Ride |

`Voice::outlet()` returns the corresponding dry outlet. Use outlet 0 for the
stock mix, or patch the dry voices through individual effects and sum them in
the graph. All seven outlets come from one render of each voice.

```rust
use wavegraph_909::{Samples, Tr909, Voice};
use wavegraph_core::{Event, Process, ProcessContext};

let mut kit = Tr909::new(Samples {
    closed_hat: &[0x00, 0x40], ..Samples::X0XPROC1
}).expect("whole PCM frames");
let mut buses = [[0.0_f32; 4]; 7];
kit.process(&mut ProcessContext {
    frames: 4, inputs: &[], outputs: &mut buses.each_mut().map(|b| b.as_mut_slice()),
    events: &[Event { frame: 2, port: 0, message: Voice::ClosedHat }], emit: &mut Err,
});
assert_eq!(buses[Voice::ClosedHat.outlet()], [0.0, 0.0, 0.5, 0.0]);
assert_eq!(buses[0], buses[Voice::ClosedHat.outlet()]);
```

## Modeled drums

The [service notes][service] describe the original analog signal paths. The
kick sweeps a shaped triangle oscillator down to about 50 Hz, with a separate
short noise attack. The snare mixes two pitched shells with filtered noise.
Both models have fixed tunings and decay settings.

`BassDrum`, `SnareDrum` and `HandClap` can each run independently as a
one-output `Process` with a unit trigger message. To embed the whole kit in
a mono instrument, call `Tr909::trigger` and `next_mix_frame`. The latter
renders directly into the mix without allocating stem buffers.

### Hand clap

`Voice::Clap` triggers a noise synthesizer. It also works on its own as
`HandClap`, a one-output process that accepts `()` to strike. The 909's
[circuit][clap] feeds shift-register noise through a band-pass filter into
two paths. A pulsed VCA produces the initial claps. A darker, more slowly
decaying path gives them a sense of room.

The model uses the component values around IC26b for a nominal 963 Hz filter.
Four bursts spaced about 10.7 ms apart approximate the transients in
[AudioRealism's individual-output captures][pack]. Retriggering restarts
the envelopes and carries the noise and filter state into the next hit.

## Play the kit

The FM-1 and host example share a two-bar pattern with a four-on-the-floor kick,
snare and clap on two and four, offbeat open hats, closed hats and ride accents.
The host plays it at 126 BPM and lets the voices ring out. The three PCM hits
come from Fugue State Audio's synthesized `x0xproc1` kit. They occupy
148,124 bytes uncompressed.
The [sample credits](samples/README.md)
give the source WAVs, license and conversion command.

```sh
cargo run -q -p wavegraph-909 --example cymbals | ffplay -autoexit -nodisp -f s16le -sample_rate 44100 -i -
```

To audition your own recordings, pass three WAV paths in closed hat, open hat,
ride order. FFmpeg converts the files to mono at 44.1 kHz on the host.

```sh
cargo run -q -p wavegraph-909 --example cymbals -- ch.wav oh.wav ride.wav | ffplay -autoexit -nodisp -f s16le -sample_rate 44100 -i -
```

For comparison, [Janne G:son Berg's hardware captures][jgb] include 909 hats
and rides. Altemark published the recordings under CC BY 4.0. Freesound's
[FAQ][faq] flags the underlying Roland hi-hat and cymbal waveforms separately.
The example also accepts the extracted directory of [AudioRealism's TR-909
pack][pack]. Its `Readme.txt` requires permission to redistribute individual hits.

```sh
cargo run -q -p wavegraph-909 --example cymbals -- "$HOME/Downloads/TR-909-44.1kHz-16bit" | ffplay -autoexit -nodisp -f s16le -sample_rate 44100 -i -
```

## License

The Rust code is licensed under either the [Apache License, Version 2.0](LICENSE-APACHE)
or the [MIT License](LICENSE-MIT), at your option. The bundled samples are
available under [CC0 1.0](samples/LICENSE-CC0).

[service]: https://www.synfo.nl/servicemanuals/Roland/ROLAND_TR-909_SERVICE_NOTES.pdf#page=5
[rebirth]: https://www.deepsonic.ch/deep/docs_manuals/propellerhead_rebirth_rb338_v2.01_manual.pdf#page=35
[pack]: https://audiorealism.se/TR-909-SamplePack.html
[jgb]: https://freesound.org/people/altemark/packs/1643/
[faq]: https://freesound.org/help/faq/#what-sounds-are-legal-to-put-on-freesound
[clap]: https://www.firstpr.com.au/rwi/tr-909/TR-909-Sound-Mods.pdf#page=9
