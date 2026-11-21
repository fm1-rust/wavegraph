# WaveGraph

WaveGraph is a `no_std` crate for building Pure Data-like audio patches.
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
`wavegraph_sample::Sample` for format-generic arithmetic. The integration test
runs the same pair of connected objects with `f32` and signed 24-bit PCM and
stores messages in a fixed-capacity `heapless::Vec`. [Patch example][patch]

## License

Licensed under the [MIT license](http://opensource.org/licenses/MIT).

[patch]: https://github.com/fm1-rust/wavegraph/blob/main/wavegraph/tests/patch.rs
