# wavegraph-arp

A sample-clocked MIDI arpeggiator with one WaveGraph message inlet and one message
outlet. Route a MIDI channel to its own `Arpeggiator`, then connect the outlet
to a synth or MIDI output on that channel. The patch supplies the channel
routing. The object uses ordinary `wavegraph_core::Event<MidiMessage>` values,
requires no allocator, and can track all 128 pitches and their strike velocities.

Use `Arpeggiator::new(24_000_000, 544)` for the FM-1 clock. Call `configure`
with a `Config` and tempo in BPM. Modes are up, down, up/down, as played and
random. Steps range from quarter to thirty-second notes, with eighth- and
sixteenth-note triplets. Range is one to four octaves, and gate is 1–100%.
An octave pass skips pitches above 127. At 100% gate, note-off precedes the
next note-on at the same frame.

The first chord starts immediately. Latch holds a released chord until the
next chord begins. Controllers pass through at their input offsets. All Sound
Off and All Notes Off also clear the chord and release the generated note.
The separate `Chord` and `Clock` types support panel or MIDI input collected on
a different task from the audio clock.


For example, a host can route channel 2 into a patch and serialize its output
back onto the same channel. Frame offsets stay available to the host.

```rust
use wavegraph_arp::{Arpeggiator, Config};
use wavegraph_core::{Event, Process, ProcessContext, midi::MidiMessage};

let mut arp = Arpeggiator::new(48_000, 1).expect("audio clock");
arp.configure(Config { octaves: 2, ..Config::INITIAL }, 128);
let channel = 1; // Zero-based MIDI channel 2.
let packet = [0x91, 60, 96];
if packet[0] & 0x0f == channel {
    let note = MidiMessage::decode(packet).expect("note message");
    let input = [Event { frame: 17, port: 0, message: note }];
    arp.process(&mut ProcessContext::<_, f32> {
        frames: 128, inputs: &[], outputs: &mut [], events: &input,
        emit: &mut |e| {
            assert_eq!((e.frame, e.message.encode(channel)), (17, Some(packet)));
            Ok(())
        },
    });
}
```
