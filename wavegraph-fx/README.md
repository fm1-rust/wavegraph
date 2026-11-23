# wavegraph-fx

Fixed-storage, `no_std` send effects for signed Q23 audio. The FM-1 uses a
`ChannelMixer<4, 3>` to apply each track fader once and produce the dry sum,
reverb send and delay send on successive outlets. Each effect has a
wet-only return that can be added to the dry bus. With the default two outputs,
`ChannelMixer<4>` supplies a single send. `SendMixer` remains available as a
two-input send-only node.

- `Reverb` uses four comb filters and two allpasses with 17,048 bytes of
  caller-owned storage tuned for approximately 44.1 kHz.
- `Delay::new(line, rate_hz)` borrows one `i16` per frame of maximum delay.
  `Message::TimeMs` sets echo spacing and `Message::Feedback` sets the fraction
  fed back (0–95%). Zero feedback gives a single echo. Changes to the time
  crossfade the live taps over 64 frames.

Mixer and effect controls accept sample-offset events. `Message::Wet` scales a
return, and `Message::Enabled(false)` mutes it while its tail continues to decay.

## License

Dual-licensed under [Apache-2.0](LICENSE-APACHE) OR [MIT](LICENSE-MIT).
