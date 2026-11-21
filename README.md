# WaveGraph

WaveGraph runs Pure Data-like audio patches on embedded systems. The engine
interprets a compiled signal graph and processes frame-timed events using
caller-owned buffers. Each instrument is a separate crate built on the same
patch object interface.

## Crates

| Crate | Description | crates.io | Documentation | Build status |
| --- | --- | --- | --- | --- |
| [`wavegraph`](wavegraph/) | DSP and bytecode interpreter | [crates.io](https://crates.io/crates/wavegraph) | [Patch guide](wavegraph/README.md) | [![Build status][wavegraph-build]][wavegraph-ci] |
| [`wavegraph-core`](wavegraph-core/) | Patch objects, events and bytecode instructions | Unpublished | [Object interface](wavegraph-core/README.md) | [![Build status][wavegraph-build]][wavegraph-ci] |
| [`wavegraph-monosynth`](wavegraph-monosynth/) | Sine, triangle, sawtooth and square monosynth | Unpublished | [Instrument guide](wavegraph-monosynth/README.md) | [![Build status][wavegraph-build]][wavegraph-ci] |
| [`wavegraph-sample`](wavegraph-sample/) | PCM sample types, conversions and arithmetic | [crates.io](https://crates.io/crates/wavegraph-sample) | [Sample guide](wavegraph-sample/README.md) | [![Build status][sample-build]][sample-ci] |

The libraries support Rust 1.85 and build without `std` or a heap allocator.
To browse their API documentation locally, run `cargo doc --workspace --no-deps --open`.

## Patch example

The [engine example](wavegraph/README.md#rendering-a-compiled-patch) runs the
monosynth from bytecode at the FM-1 sample rate.

The [patch integration test](wavegraph/tests/patch.rs) connects two gain objects.
One cord carries audio samples and another carries gain changes. Both objects
apply each change at the same frame. The test renders the patch with `f32`
and signed 24-bit PCM.

```sh
cargo +stable test -p wavegraph --test patch --locked
```

## License

The engine, core interface and monosynth crates are licensed under the [MIT License](wavegraph/LICENSE).
`wavegraph-sample` vendors RustAudio's [`dasp_sample` implementation][upstream]
and is licensed under either the [MIT License](wavegraph-sample/LICENSE-MIT) or
the [Apache License, Version 2.0](wavegraph-sample/LICENSE-APACHE) at your option.

[wavegraph-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph.yml/badge.svg?branch=main&event=push
[wavegraph-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph.yml
[sample-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-sample.yml/badge.svg?branch=main&event=push
[sample-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-sample.yml
[upstream]: wavegraph-sample/README.md#upstream
