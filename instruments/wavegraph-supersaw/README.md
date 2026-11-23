# wavegraph-supersaw

A monophonic instrument inspired by the Roland JP-8000/8080 Super Saw. One note drives a
center saw and six detuned saws through a shared high-pass filter. Each voice
has one message inlet and one mono signal outlet. It supports `f32` and
signed 24-bit PCM, runs without a heap, and accepts frame-timed events.

| Message | Action |
| --- | --- |
| `NoteOn { note, velocity }` | Play a MIDI note with a short velocity-sensitive attack. |
| `NoteOff(note)` | Release the matching note. Zero-velocity note-on does the same. |
| `Detune(0..=127)` | Spread three saws above and three below the center. The last few steps open up rapidly. |
| `Mix(0..=127)` | Raise the six side saws together. A little of them is audible even at zero. |

Create a `Supersaw::new(&tuning)` with a `Tuning` at the physical output rate.
One tuning table can serve the monosynth and supersaw. The seven 32-bit phase
accumulators run at twice that rate. Their phases continue across notes, knob
changes and rests. A key-tracking one-pole high-pass shapes the sum, and a
31-tap half-band filter removes high-frequency energy before decimation.
Sharp saws can still alias, especially in the highest octaves.
During silent blocks the voice advances all seven phases without rendering the filters.

[The Usual Suspects' 39C3 talk] describes the seven ramps, high-pass stage
and 88.2 kHz internal rate. The high-pass follows the pitch tracking reported
in [Adam Szabo’s JP recordings]. The asymmetric pitch intervals, nonlinear detune
response and fixed center level here follow [Joakim's JE-8086 traces] and
[control measurements]. The phase seeds, filter implementations, output
headroom and 2 ms gate slew are WaveGraph choices. The output is mono and dry.

## Play a note

```rust
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_supersaw::{Message, Supersaw, Tuning};

let tuning = Tuning::new(44_100, 1).unwrap();
let mut voice = Supersaw::new(&tuning);
let mut samples = [0.0_f32; 128];
voice.process(&mut ProcessContext {
    frames: samples.len(),
    inputs: &[],
    outputs: &mut [&mut samples],
    events: &[Event {
        frame: 0,
        port: 0,
        message: Message::NoteOn { note: 69, velocity: 100 },
    }],
    emit: &mut Err,
});
```

## License

Licensed under either of

- [Apache License, Version 2.0][apache]
- [MIT License][mit]

at your option.

### Contribution

Unless you explicitly state otherwise, you license any contribution you
intentionally submit for inclusion in this work (as defined in the Apache-2.0
license) under both licenses above, without additional terms or conditions.

[The Usual Suspects' 39C3 talk]: https://media.ccc.de/v/39c3-from-silicon-to-darude-sand-storm-breaking-famous-synthesizer-dsps
[Adam Szabo’s JP recordings]: https://www.adamszabo.com/internet/adam_szabo_how_to_emulate_the_super_saw.pdf
[Joakim's JE-8086 traces]: https://atosynth.blogspot.com/2026/02/confirming-that-code-is-correct.html
[control measurements]: https://atosynth.blogspot.com/2026/02/jp80x0-midi-to-coefficient-mapping.html
[apache]: https://github.com/fm1-rust/wavegraph/blob/main/instruments/wavegraph-supersaw/LICENSE-APACHE
[mit]: https://github.com/fm1-rust/wavegraph/blob/main/instruments/wavegraph-supersaw/LICENSE-MIT
