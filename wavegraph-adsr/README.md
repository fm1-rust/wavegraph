# wavegraph-adsr

A sample-clocked, allocation-free ADSR for amplitude control. Attack is linear.
Decay and release use quadratic curves that reach their targets on the final
frame. Prepare one timing per instrument and keep one envelope per voice.

```rust
use wavegraph_adsr::{Adsr, Envelope};

let patch = Adsr { attack_us: 2_000, decay_us: 0, sustain_percent: 100, release_us: 2_000 };
let timing = patch.at_sample_period(1.0 / 1000.0);
let mut voice = Envelope::IDLE;
voice.gate_on(&timing);
assert_eq!(voice.advance(&timing), 0.5);
assert_eq!(voice.advance(&timing), 1.0);
voice.gate_off(&timing);
assert_eq!(voice.advance(&timing), 0.25);
assert_eq!(voice.advance(&timing), 0.0);
assert!(voice.is_idle());
```

For a live edit, prepare the new timing and call `voice.retime(&old, &new)`
for each voice before its next sample. Each voice continues from its current
gain. Reopening the gate during release attacks from the tail. Assign
`Envelope::IDLE` before gating on to start a fresh strike from silence.

## License

Licensed under either the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT), at your option.
