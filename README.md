# WaveGraph

[![WaveGraph boot screen with pink FM-1 lettering over the green waveform](wavegraph-fm1/assets/boot-screen.png)](wavegraph-fm1/)

WaveGraph runs Pure Data-like audio patches on embedded systems. The engine
interprets a compiled signal graph and processes frame-timed events using
caller-owned buffers. Each instrument is a separate crate. The
[instrument guide](instruments/README.md) shows how to implement one as a patch object.

## Crates

| Crate | Description | crates.io | Documentation | Build status |
| --- | --- | --- | --- | --- |
| [`wavegraph-303`](instruments/wavegraph-303/) | Monophonic 303 bass synth with accent and slide | Unpublished | [Instrument guide](instruments/wavegraph-303/README.md) | [![Build status][303-build]][303-ci] |
| [`wavegraph-808`](instruments/wavegraph-808/) | Synthesized 808 kick, snare and shared-source hats with individual outputs | Unpublished | [Instrument guide](instruments/wavegraph-808/README.md) | [![Build status][808-build]][808-ci] |
| [`wavegraph-909`](instruments/wavegraph-909/) | Six-voice hybrid 909 with a mono mix and individual outputs | Unpublished | [Instrument guide](instruments/wavegraph-909/README.md) | [![Build status][909-build]][909-ci] |
| [`wavegraph-adsr`](wavegraph-adsr/) | Sample-clocked ADSR shared by the monosynth and organ | Unpublished | [Envelope guide](wavegraph-adsr/README.md) | [![Build status][dsp-build]][dsp-ci] |
| [`wavegraph-arp`](wavegraph-arp/) | Frame-clocked MIDI arpeggiator | Unpublished | [Arpeggiator guide](wavegraph-arp/README.md) | [![Build status][arp-build]][arp-ci] |
| [`wavegraph-audio`](wavegraph-audio/) | Routed audio I/O for macOS and the FM-1 | Unpublished | [Run the Mac synth](wavegraph-audio/README.md) | [![Build status][audio-build]][audio-ci] |
| [`wavegraph-core`](wavegraph-core/) | Patch objects, events and bytecode instructions | Unpublished | [Object interface](wavegraph-core/README.md) | [![Build status][dsp-build]][dsp-ci] |
| [`wavegraph-dsp`](wavegraph-dsp/) | DSP and bytecode interpreter | Unpublished | [Patch guide](wavegraph-dsp/README.md) | [![Build status][dsp-build]][dsp-ci] |
| [`wavegraph-fm1`](wavegraph-fm1/) | Four assignable channels, five synths, 808 and 909 drums and a live oscilloscope for the M-VAVE FM-1 | [Firmware releases](https://github.com/fm1-rust/wavegraph/releases) | [Controls and build](wavegraph-fm1/README.md) | [![Build status][fm1-build]][fm1-ci] |
| [`wavegraph-lfo`](wavegraph-lfo/) | Frame-clocked modulation source with a float message outlet | Unpublished | [LFO guide](wavegraph-lfo/README.md) | [![Build status][lfo-build]][lfo-ci] |
| [`wavegraph-monosynth`](instruments/wavegraph-monosynth/) | Sine, triangle, sawtooth and square monosynth | Unpublished | [Instrument guide](instruments/wavegraph-monosynth/README.md) | [![Build status][dsp-build]][dsp-ci] |
| [`wavegraph-organ`](instruments/wavegraph-organ/) | Four-voice, modeled M1-inspired house organ | Unpublished | [Instrument guide](instruments/wavegraph-organ/README.md) | [![Build status][organ-build]][organ-ci] |
| [`wavegraph-sample`](wavegraph-sample/) | PCM sample types, conversions and arithmetic | [crates.io](https://crates.io/crates/wavegraph-sample) | [Sample guide](wavegraph-sample/README.md) | [![Build status][sample-build]][sample-ci] |
| [`wavegraph-sampler`](instruments/wavegraph-sampler/) | One-shot mono PCM player | Unpublished | [Playback guide](instruments/wavegraph-sampler/README.md) | [![Build status][sampler-build]][sampler-ci] |
| [`wavegraph-supersaw`](instruments/wavegraph-supersaw/) | JP-inspired seven-saw unison with detune and mix | Unpublished | [Instrument guide](instruments/wavegraph-supersaw/README.md) | [![Build status][supersaw-build]][supersaw-ci] |
| [`wavegraph-wavetable`](instruments/wavegraph-wavetable/) | Four-voice wavetable scanning with resonant and vocal banks | Unpublished | [Instrument guide](instruments/wavegraph-wavetable/README.md) | [![Build status][wavetable-build]][wavetable-ci] |

The DSP libraries and the FM-1 audio backend support Rust 1.88 and build without `std` or a heap allocator.
To browse their API documentation locally, run `cargo doc --workspace --no-deps --open`.

## Patch example

The [engine example](wavegraph-dsp/README.md#rendering-a-compiled-patch) runs the
monosynth from bytecode at the FM-1 sample rate.

The [patch integration test](wavegraph-dsp/tests/patch.rs) connects two gain objects.
One cord carries audio samples and another carries gain changes. Both objects
apply each change at the same frame. The test renders the patch with `f32`
and signed 24-bit PCM.

```sh
cargo +stable test -p wavegraph-dsp --test patch --locked
```

[dsp-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-dsp.yml/badge.svg?branch=main&event=push
[dsp-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-dsp.yml
[303-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-303.yml/badge.svg?branch=main&event=push
[303-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-303.yml
[808-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-808.yml/badge.svg?branch=main&event=push
[808-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-808.yml
[909-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-909.yml/badge.svg?branch=main&event=push
[909-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-909.yml
[lfo-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-lfo.yml/badge.svg?branch=main&event=push
[lfo-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-lfo.yml
[arp-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-arp.yml/badge.svg?branch=main&event=push
[arp-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-arp.yml
[fm1-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-fm1.yml/badge.svg?branch=main&event=push
[fm1-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-fm1.yml
[organ-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-organ.yml/badge.svg?branch=main&event=push
[organ-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-organ.yml
[sample-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-sample.yml/badge.svg?branch=main&event=push
[sample-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-sample.yml
[sampler-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-sampler.yml/badge.svg?branch=main&event=push
[sampler-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-sampler.yml
[supersaw-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-supersaw.yml/badge.svg?branch=main&event=push
[supersaw-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-supersaw.yml
[wavetable-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-wavetable.yml/badge.svg?branch=main&event=push
[wavetable-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-wavetable.yml

[audio-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-audio.yml/badge.svg?branch=main&event=push
[audio-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-audio.yml
