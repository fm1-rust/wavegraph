# wavegraph-fm

A four-voice, two-operator FM instrument for WaveGraph. A sine modulator shifts
the carrier's phase. The **ratio** sets the spacing of the sidebands and the
**index** sets their strength. Harder strikes increase both volume and index.
The modulator's **level envelope** sets how that brightness evolves. **Feedback**
feeds the previous modulator sample back into its phase.

Each key starts three ADSRs. The carrier envelope controls the note's volume.
The modulator envelope controls its sidebands. The pitch envelope bends both
operators together and preserves their frequency ratio. Set its depth from
−1200 to +1200 cents. All keys use the patch's shape and depth, and each key
runs its own attack and release. The four ADSR parameters use WaveGraph's
stepped times in microseconds and a sustain percentage.

The three original patches are Tine EP (2:1 electric piano), Glass Bell
(approximately √2:1) and Wire Bass (1:1 with feedback and a short two-semitone
pitch drop). Zero index produces a sine. The engine reduces the index in the
upper register and silences a carrier bent beyond Nyquist.

```rust
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_fm::{Fm, Message, Patch, Tuning};

let tuning = Tuning::new(44_100, 1).unwrap();
let mut fm = Fm::new(&tuning);
let mut audio = [0.0_f32; 128];
let events = [
    Event { frame: 0, port: 0, message: Message::Patch(Patch::BELL) },
    Event { frame: 0, port: 0, message: Message::NoteOn { note: 69, velocity: 100 } },
];
fm.process(&mut ProcessContext {
    frames: audio.len(), inputs: &[], outputs: &mut [&mut audio],
    events: &events, emit: &mut Err,
});
assert!(audio.iter().any(|&sample| sample != 0.0));
```

The processor borrows one tuning table, allocates no heap memory and accepts
frame-timed messages on inlet zero. Its mono output supports both `f32` and
`I24`. Poly, Chord and Unison use WaveGraph's four-voice keyboard modes.
