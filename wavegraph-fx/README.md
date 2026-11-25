# wavegraph-fx

Fixed-storage, `no_std` effects for signed Q23 audio. The FM-1 uses a
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
- `Flanger::new(rate_hz)` mixes the dry signal with a fractionally delayed
  copy. A 0.10–3.94 Hz triangle LFO sweeps a 64-sample PCM16 line. At full
  depth the delay runs from one sample to 2 ms, up to 63 samples. Send
  `Message::Flanger(FlangerControls)` to edit rate, depth, feedback (up to
  75%) and mix. `Message::Enabled(false)` fades the insert to dry.
- `Saturator` applies drive, shapes the clipped signal, then adjusts its wet
  level and mix. Shape 0 uses the original cubic curve. Shape 128 hard-clips
  with the same low-level gain and peak. Sweeps between the two change the
  harmonics around the clipping knee.
- `PatternFilter` sweeps a resonant low-pass or band-pass filter over any mono
  input. Set cutoff, resonance, envelope amount and decay with
  `Message::PatternFilter`. At each step, send the accent from one of the
  eight sixteen-step `FILTER_PATTERNS` as a sample-offset `Message::FilterTrigger`.
  A zero accent lets the last sweep decay. With amount at zero the cutoff
  stays fixed. The player supplies the clock, so a held note can pulse
  independently of its note pattern.
- `SidechainDucker<N>` takes the program on signal inlet 0. Route a kick
  or another signal to a further inlet, then use `Message::SidechainKey` to
  choose the detector. `SidechainDepth` controls the gain reduction and
  `SidechainHz` sets the low-shelf corner. The low band ducks more deeply
  than the highs. Disabling the insert crossfades to unity dry audio.
- `MasterBus` runs three-band EQ, a feed-forward compressor and a separate
  sample-peak limiter. Its mono graph port accepts Q23. For a wide mixer,
  pass a signed sum in Q23 units to `process_sample` before it saturates.
  `ChannelMixer::with_dry_headroom(3)` accommodates four boosted tracks.
  Restore those three bits when summing the dry bus with the wet returns.

Mixer and effect controls accept sample-offset events. `Message::Wet` scales a
return, and `Message::Enabled(false)` mutes it while its tail continues to decay.

## License

Dual-licensed under [Apache-2.0](LICENSE-APACHE) OR [MIT](LICENSE-MIT).
