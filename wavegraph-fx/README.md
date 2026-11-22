# wavegraph-fx

Small, fixed-storage, `no_std` global send effects for WaveGraph. The first
processor is a mono, fixed-point Schroeder-style reverb (4 combs, 2 allpasses).
It requires 17,048 bytes of caller-owned delay storage, runs without allocation,
and produces an entirely wet return to be mixed with the dry instrument output.

The `SendMixer` has two independent signal inputs for the current FM-1
monosynth and TB-303. Each send gain, decay, bypass and return level accepts
sample-offset `Message` events through `Process<Message, I24>`; no parameter
changes require reallocating the reverb or stopping audio. The WaveGraph
bytecode schedules the send mixer, reverb and final dry/wet mix explicitly.

The lengths target the FM-1's approximately 44.1 kHz output. The return is
currently mono and should be evaluated for CPU cost, SRAM headroom and audible
artifacts on the real FM-1 before expanding to stereo or more sophisticated
algorithms. It does not reproduce the Roland DEP-5 or the CHOMPI 303 reverb.

## License

Dual-licensed under [Apache-2.0](LICENSE-APACHE) OR [MIT](LICENSE-MIT).
