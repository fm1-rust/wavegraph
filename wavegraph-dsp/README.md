# wavegraph-dsp

[![Build status][ci-badge]][ci]

`wavegraph-dsp` runs compiled audio patches on caller-owned buffers without a heap.
It re-exports `Event`, `Op`, `Port`, `Process`, and `ProcessContext` from
[`wavegraph-core`][core]. Instrument crates depend on that interface directly.
An application supplies instances and selects the sample format and DSP
implementation.

## Rendering a compiled patch

The following program plays the waveform monosynth through a half-gain stage.
The note starts at frame 2. Its state carries over to the next block.

```rust
use wavegraph_dsp::{Event, bytecode::Program, dsp::Software};
use wavegraph_monosynth::{Message, Monosynth, Tuning, Waveform};
use wavegraph_sample::I24;

const TUNING: Tuning = Tuning::new(24_000_000, 544).unwrap();
let mut voice = Monosynth::new(&TUNING);
let program = [
    1,          // version
    1, 0, 0, 1, // call object 0 with no signal inputs and one output (bus 0)
    2, 0, 64,   // scale bus 0 by 64/128, producing bus 1
];
let mut objects: [&mut dyn wavegraph_dsp::Process<Message, I24>; 1] = [&mut voice];
let schedule = Program::<2>::decode(&program).unwrap();
let mut patch = schedule.bind(&mut objects).unwrap();
let notes = [
    Event { frame: 2, port: 0, message: Message::Waveform(Waveform::Square) },
    Event { frame: 2, port: 0, message: Message::NoteOn { note: 69, velocity: 100 } },
];
let mut buses = [[I24::default(); 256]; 2];
patch.render(&mut Software, &mut buses, &[&notes], |_, event| Err(event)).unwrap();
assert_eq!(buses[1][1].inner(), 0);
assert!(buses[1][2].inner() > 0);
```

The [bytecode module][bytecode] specifies the wire format. Load a plan outside
the audio callback. Each call to `render` executes it once per block. Control
events retain their frame offsets and ties retain their order. A second program can be
decoded while the first plays. At a block boundary, `Patch::replace` validates
its object layouts and buffer requirements and switches the schedule. The
instances retain their state.

## Awaiting a program change

Enable the `async` feature and split a `wavegraph_dsp::control::EditQueue` between
the editor and the render host. The editor calls `control.commit(&next).await`
with a predecoded program. At a block boundary, the host calls
`host.begin_block(|plan| patch.replace(plan, buses.len()))` and keeps the
returned revision with the block. When the output accepts it, the host calls
`host.submitted(revision, first_frame)`. The editor receives that revision and
frame, or the error from a rejected replacement. A missed block leaves the
edit pending. Frame numbers come from the host's render timeline.

The queue holds one edit in caller-owned storage. The render callback stays
synchronous. Only the slot and waiter are accessed inside a critical section.
Choose a backend and task waker that fit the audio deadline. On the FM-1 both
endpoints run on CPU0, where the critical section masks interrupts. For desktop
tests, `critical-section/std` uses a process-wide mutex.
The [held-note test](tests/control.rs) exercises a route change during playback.

A host can delegate rendering to another core through two
`wavegraph_dsp::mailbox::Mailbox` instances in coherent memory. Send a tagged,
exclusively borrowed buffer through one mailbox and receive it back through
the other. The worker keeps its patch and scratch buffers between blocks.
If a deadline passes, ownership stays with the worker until it returns the
buffer. Only a current block accepted by the output earns an edit receipt.
The [mailbox test](tests/mailbox.rs) exercises this exchange on two threads.

## Platform DSP

`dsp::Software` provides gain and saturating mix for `f32` and signed 24-bit
PCM (`I24`). A board adapter can depend on `wavegraph-core` alone and implement
`wavegraph_core::Dsp`. Pass it to
`render` to select optimized block operations. Instruments can supply native
processors through the same object table. The application owns the audio
peripheral and its refill timing.

For the FM-1, use the BSP's rational `FRAME_RATE` for tuning and render
`BLOCK_FRAMES` at a time. Write `I24::inner()` into the left and right words
of the stereo output block and submit it with `Stream::try_write`. Retain the
rendered block on `WouldBlock` so a retry does not advance the synth twice.
See the [FM-1 audio driver][fm1].

## License

Licensed under either of

- [Apache License, Version 2.0][apache]
- [MIT License][mit]

at your option.

### Contribution

Unless you explicitly state otherwise, you license any contribution you
intentionally submit for inclusion in this work (as defined in the Apache-2.0
license) under both licenses above, without additional terms or conditions.

[ci-badge]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-dsp.yml/badge.svg?branch=main&event=push
[ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-dsp.yml
[core]: https://github.com/fm1-rust/wavegraph/tree/main/wavegraph-core
[bytecode]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph-core/src/bytecode.rs
[fm1]: https://github.com/fm1-rust/fm1-rust/blob/main/fm1/src/audio.rs
[apache]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph-dsp/LICENSE-APACHE
[mit]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph-dsp/LICENSE-MIT
