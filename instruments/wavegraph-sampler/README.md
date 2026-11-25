# wavegraph-sampler

A one-shot PCM player with one message inlet and one mono signal outlet.
Each `Sampler` borrows a clip and has its own playback position. A drum kit
can give each voice a separate output and use `Stop` to choke an open hat.

`Sampler::from_pcm16le` accepts headerless, signed 16-bit little-endian PCM,
including bytes in flash from `include_bytes!`. Playback advances one source
sample per output frame. Prepare the clip at the intended output rate. The
FM-1's 24 MHz / 544 clock is about 44,118 frames/s. The audio callback uses
no heap and converts directly to the graph's signal format, including `I24`
and `f32`.

The optional `packed-pcm` feature adds `Sampler::from_packed` for lossless
`fm1_assets::pcm::PackedPcm16` clips. Each voice streams from the borrowed
clip with its own decoder cursor.

```rust
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_sampler::{Message, Sampler};

let mut voice = Sampler::from_pcm16le(&[0x00, 0xc0, 0x00, 0x40]).expect("two PCM frames");
let mut output = [0.0_f32; 4];
voice.process(&mut ProcessContext {
    frames: output.len(), inputs: &[], outputs: &mut [&mut output],
    events: &[Event { frame: 1, port: 0, message: Message::Trigger }],
    emit: &mut Err,
});
assert_eq!(output, [0.0, -0.5, 0.5, 0.0]);
```

`Trigger` restarts the clip at frame zero. `Stop` and the end of the clip
both produce silence. Messages at the same frame take effect in slice order.

## Chromatic playback

The optional `ima-adpcm` feature adds `Clip::from_ima_adpcm` for the data
chunk of a mono IMA WAV file encoded with 64-byte blocks. Supply the
original frame count to exclude padding. Trims and pitch jumps skip whole
blocks directly. Each voice decodes at most one block to seek to a frame.

`PitchedSampler` plays a bank of PCM16 clips from MIDI note and velocity
messages. Each `Clip::from_pcm16le(bytes, source_hz, root_note)` declares the
recording rate and the MIDI key for native-speed playback. Construct the
player with the bank and output rate. It uses a fractional cursor and linear
interpolation. An octave up doubles speed and halves duration. Source buffers
stay borrowed and may live in flash.

With the optional `packed-pcm` feature, `Clip::from_pcm1` validates a
losslessly packed clip in the FM-1 PCM1 format. The pitch interpolator reads
its two neighboring frames directly and skips whole 32-frame groups when
necessary. Raw and packed clips can share a bank.

`PitchedMessage::Select` and `PitchedMessage::Playback` set up the next
strike. `Playback` selects a start and end in percent and one of three modes:
one-shot, gated or forward loop. Gated and looped notes fade on a matching
note-off. With the default flat amplitude contour a zero playback release uses
a 64-frame de-click, and positive values set the fade in milliseconds.
`Stop` cuts any tail immediately.

`PitchedMessage::Timbre` sets a four-pole digital resonant low-pass filter
and independent amplitude and filter envelopes. Both use the compact
`wavegraph_adsr::Shape`. Cutoff can track the keyboard and strike velocity.
Signed envelope depth selects rising or inverted sweeps. A timbre edit
retimes a sounding contour without resetting sample position. A shaped
amplitude contour uses its own ADSR release. `Timbre::DRY` passes the
decoded PCM through at full velocity with the filter bypassed.

Tag a rhythmic clip with `Clip::with_native_tempo` and send
`PitchedMessage::Tempo` to follow a transport. Tempo changes preserve the
current position. The root key tracks the beat and other keys transpose by
changing playback speed. Untagged clips retain their tuning. For fractional
DAC clocks use `PitchedSampler::at_sample_rate`.

## Play a sample

From the workspace root, play the [909 example kit's closed hi-hat][hat] through the
graph's signed 24-bit signal path.

```sh
cargo run -q -p wavegraph-sampler --example one_shot -- instruments/wavegraph-909/samples/closed-hat.s16le | ffplay -autoexit -nodisp -f s16le -sample_rate 44100 -i -
```

To substitute your own WAV, convert it once and pass the raw file to the
same example.

```sh
ffmpeg -i hit.wav -ac 1 -ar 44100 -f s16le hit.s16le
cargo run -q -p wavegraph-sampler --example one_shot -- hit.s16le | ffplay -autoexit -nodisp -f s16le -sample_rate 44100 -i -
```

The [909 kit][909] uses three instances of this player and publishes each
voice on its own signal outlet.

## License

The Rust code is available under either the [Apache License, Version 2.0][apache]
or the [MIT License][mit].

[909]: https://github.com/fm1-rust/wavegraph/tree/main/instruments/wavegraph-909
[hat]: https://github.com/fm1-rust/wavegraph/blob/main/instruments/wavegraph-909/samples/closed-hat.s16le
[apache]: https://github.com/fm1-rust/wavegraph/blob/main/instruments/wavegraph-sampler/LICENSE-APACHE
[mit]: https://github.com/fm1-rust/wavegraph/blob/main/instruments/wavegraph-sampler/LICENSE-MIT
