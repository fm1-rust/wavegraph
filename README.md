# WaveGraph

WaveGraph is a Rust interface for building Pure Data-like audio patches on
embedded systems. Patch objects process blocks of samples and exchange control
messages at frame offsets within each block. The host connects the objects,
supplies their buffers and determines their processing order.

## Crates

| Crate | Description | crates.io | Documentation | Build status |
| --- | --- | --- | --- | --- |
| [`wavegraph`](wavegraph/) | Patch objects, signal ports and control messages | Unpublished | [Patch guide](wavegraph/README.md) | [![Build status][wavegraph-build]][wavegraph-ci] |
| [`wavegraph-sample`](wavegraph-sample/) | PCM sample types, conversions and arithmetic | Unpublished | [Sample guide](wavegraph-sample/README.md) | [![Build status][sample-build]][sample-ci] |

Both libraries support Rust 1.85 and build without `std` or a heap allocator.
To browse their API documentation locally, run `cargo doc --workspace --no-deps --open`.

## Patch example

The [patch integration test](wavegraph/tests/patch.rs) connects two gain objects.
One cord carries audio samples and another carries gain changes. Both objects
apply each change at the same frame. The test renders the patch with `f32`
and signed 24-bit PCM.

```sh
cargo +stable test -p wavegraph --test patch --locked
```

## License

The `wavegraph` crate is licensed under the [MIT License](wavegraph/LICENSE).
`wavegraph-sample` vendors RustAudio's [`dasp_sample` implementation][upstream]
and is licensed under either the [MIT License](wavegraph-sample/LICENSE-MIT) or
the [Apache License, Version 2.0](wavegraph-sample/LICENSE-APACHE) at your option.

[wavegraph-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph.yml/badge.svg?branch=main&event=push
[wavegraph-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph.yml
[sample-build]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-sample.yml/badge.svg?branch=main&event=push
[sample-ci]: https://github.com/fm1-rust/wavegraph/actions/workflows/wavegraph-sample.yml
[upstream]: wavegraph-sample/README.md#upstream
