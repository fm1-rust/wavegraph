# wavegraph-wavetable

A four-voice wavetable instrument for WaveGraph. Each key has two clocks. The
oscillator repeats a single cycle at the note frequency. A separate cursor
selects the frame, so notes played an octave apart traverse a bank in the same
number of milliseconds. Stagger the notes in a chord to hear different vowels
on each key.

| Scan mode | Behavior while a note sounds |
| --- | --- |
| Stepped | START selects the nearest frame. |
| Smooth | START crossfades adjacent frames at the same oscillator phase. |
| OneShot | Each strike travels from START to END over TIME and holds there. |
| PingPong | Each strike travels from START to END over TIME and reverses at each endpoint. |
| Envelope | Each strike sweeps from START to END on attack, decays to its sustain position, and returns toward START on release. |

Positions use 0–127 across the bank. Swap START and END for a reverse sweep.
Editing TIME during a sweep preserves its progress. Re-striking a pitch
restarts its oscillator and scan. In Envelope mode, send
`Message::PositionEnvelope(Adsr)` to shape the sweep. A 50% sustain holds
halfway between the endpoints. `Message::Envelope(Adsr)` sets amplitude
independently, so a longer amp release lets the return sweep ring out.
Editing either envelope preserves the progress of sounding notes.

```rust
use wavegraph_core::{Event, Process, ProcessContext, Tuning};
use wavegraph_wavetable::{BANKS, Message, Scan, Wavetable};

let tuning = Tuning::new(24_000_000, 544).unwrap();
let mut synth = Wavetable::new(&tuning, &BANKS).unwrap();
let events = [
    Message::Bank(1),
    Message::Scan(Scan::VOCAL),
    Message::NoteOn { note: 57, velocity: 104 },
].map(|message| Event { frame: 0, port: 0, message });
let mut out = [0.0_f32; 128];
synth.process(&mut ProcessContext {
    frames: 128, inputs: &[], outputs: &mut [&mut out], events: &events, emit: &mut Err,
});
assert!(out.iter().any(|&sample| sample != 0.0));
```

The oscillator interpolates across each cycle boundary. At note-on it picks
the brightest of six prefiltered tables whose stored harmonics fit below
Nyquist. Banks and tuning can be shared by multiple instruments. Rendering
uses fixed voice storage, works with both float and signed 24-bit PCM, and
requires neither a heap nor an operating system.

The Waldorf WAVE's stepped/smooth control inspired the position modes. The
FIZMO manual describes a Transwave as ordered frames whose selection can be
modulated. The timed modes are our own take on that idea. The two supplied
banks come from WaveEdit Online. [WAVE parameter reference][wave] ·
[FIZMO User’s Guide, printed pp. 26–27][fizmo] · [Bank sources and importer](banks/README.md).

[wave]: https://unofficial.waldorf-wave.de/os/OS_1-900_WAVE_Sysex_manual.pdf
[fizmo]: https://synthmanuals.com/manuals/ensoniq/fizmo/owners_manual/fizmoownersmanual.pdf

Code is available under [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT).
The bundled wave data is [CC0-1.0](banks/LICENSE-CC0).
