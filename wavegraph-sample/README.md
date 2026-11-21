# wavegraph-sample

PCM sample types such as `I24`, format conversions, and the `Sample` trait
from RustAudio's `dasp_sample`. The default build uses `core` and requires
no allocator. Enable the `std` feature to use the standard library's
floating-point square roots.

With `std` disabled, upstream's `FloatSample::sample_sqrt` uses a
Newton approximation. It returns NaN for positive infinity and can lose
precision on subnormal inputs.

## Upstream

This is a temporary copy of the [`dasp_sample` source and tests][source] at
`d0892971a3b72dee9b3ff0d16f94b0962605e858`. WaveGraph is waiting for a
crates.io release containing the [stable `no_std` fix][stable]. The published
0.11.0 release uses `core_intrinsics` with `std` disabled. Upstream tracks the
release request in [issue #187][release].

The local crate uses the name `wavegraph_sample` in Rust and makes `std`
opt-in. It removes the unused `extern crate alloc` that makes a freestanding
binary require a global allocator. To retire this copy, the published crate
must also work without a global allocator.

## License

Copyright (c) 2016 RustAudio Developers. Licensed under either the
[MIT license](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE)
at your option.

[source]: https://github.com/RustAudio/dasp/tree/d0892971a3b72dee9b3ff0d16f94b0962605e858/dasp_sample
[stable]: https://github.com/RustAudio/dasp/pull/184
[release]: https://github.com/RustAudio/dasp/issues/187
