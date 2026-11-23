# wavegraph-core

[![Build status][ci-badge]][ci]

The `wavegraph-core` crate defines the interface for building Pure Data-like audio patches without an allocator.
A patch object implements `Process` and declares its inlets and outlets in
left-to-right order. A signal cord lends an upstream sample buffer to a
downstream inlet. A message cord delivers a payload at a frame offset.
Objects can process signals, messages, or both.

For example, a gain object can take a signal on inlet 0 and gain changes on
inlet 1. A change at frame 97 affects sample 97. The same interface lets a
sequencer send a note with accent and slide to a 303, or a mixer read several
mono signals. The host owns the patch cords, schedule and buffers.

Patches select their message and sample types. For example, a message enum
can represent bangs, numbers, symbols and lists. Signal samples default to
`f32`, with 1.0 representing nominal full scale. A processor can use
[`wavegraph_sample::Sample`][sample] for format-generic arithmetic. The
[patch integration test][patch] runs the same pair of connected objects with
`f32` and signed 24-bit PCM and stores messages in a fixed-capacity `heapless::Vec`.

A board adapter can implement `Dsp<S>` to provide block gain and mix kernels.
Oscillators can share a `Tuning` with MIDI phase increments and the output sample period.
The core crate has no dependencies. The scalar implementations and interpreter
live in `wavegraph`.

## Patch bytecode

The same `Op` enum is available to host compilers and embedded interpreters.
Instructions encode into caller-provided storage and borrow input bus lists
when decoded. A file or USB payload carries one version byte followed by the
encoded instructions.

```rust
use wavegraph_core::{Op, bytecode::VERSION};

let voice = Op::Call { object: 0, inputs: &[], outputs: 1 };
let gain = Op::Gain { input: 0, level: 64 };
let mut program = [0; 8];
program[0] = VERSION;
assert_eq!(voice.encode(&mut program[1..]), Ok(4));
assert_eq!(gain.encode(&mut program[5..]), Ok(3));

let (first, rest) = Op::decode(&program[1..]).unwrap();
assert_eq!(first, voice);
assert_eq!(Op::decode(rest), Ok((gain, &[][..])));
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

[ci-badge]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph.yml/badge.svg?branch=main&event=push
[ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph.yml
[sample]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph-sample/src/lib.rs
[patch]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph/tests/patch.rs
[apache]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph-core/LICENSE-APACHE
[mit]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph-core/LICENSE-MIT
