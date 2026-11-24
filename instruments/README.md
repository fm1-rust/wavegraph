# WaveGraph instruments

A WaveGraph instrument is a stateful patch object that generates audio. It
implements [`wavegraph_core::Process<M, S>`](../wavegraph-core/src/lib.rs),
where the patch chooses the message type `M` and sample type `S`. The instruments
below all take control messages on inlet 0 and write mono audio to
outlet 0. Each instance keeps its voice state between blocks.

| Crate | What to look at |
| --- | --- |
| [`wavegraph-monosynth`](wavegraph-monosynth/) | A single oscillator with a filter and a gate-controlled ADSR envelope. |
| [`wavegraph-supersaw`](wavegraph-supersaw/) | Seven detuned phase accumulators, a pitch-tracking high-pass and 2× oversampling. |
| [`wavegraph-wavetable`](wavegraph-wavetable/) | Four independent scan clocks, stepped or interpolated frames, and two CC0 banks. |
| [`wavegraph-organ`](wavegraph-organ/) | Polyphony with four preallocated voices and a note-stealing policy. |
| [`wavegraph-303`](wavegraph-303/) | A TB-303 voice that handles accent, slide and live parameter changes. |
| [`wavegraph-sampler`](wavegraph-sampler/) | One-shot PCM playback from borrowed sample data. |
| [`wavegraph-808`](wavegraph-808/) | Four synthesized drums with standalone child processors, shared metallic excitation, and individual and mixed outputs. |
| [`wavegraph-909`](wavegraph-909/) | Six-voice 909 kit with modeled drums, sampled cymbals, individual outputs and hi-hat choke. |

## Implement a new instrument

From the repository root, run `cargo new --lib instruments/wavegraph-click`.
The workspace includes crates matching `instruments/wavegraph-*`. Give the new
crate these dependencies in `Cargo.toml`. The sample conversions let it render
to both `f32` and the FM-1's signed 24-bit PCM type, `I24`.

```toml
[dependencies]
wavegraph-core.workspace = true
wavegraph-sample.workspace = true
```

This click emits a one-sample pulse. Give it the following `src/lib.rs`.

```rust
#![no_std]
//! A one-sample percussion instrument.

use core::mem;
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::FromSample;

/// Strike the instrument with a signed 8-bit pulse amplitude.
#[derive(Clone, Copy, Debug)]
pub struct Strike(pub i8);

/// Click generator, initially silent.
#[derive(Clone, Copy, Debug, Default)]
pub struct Click {
    pending: i8,
}

impl Click {
    fn render<S: FromSample<i8>>(&mut self, output: &mut [S]) {
        for sample in output {
            *sample = S::from_sample_(mem::take(&mut self.pending));
        }
    }
}

impl<S: FromSample<i8>> Process<Strike, S> for Click {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Strike, S>) {
        let output = &mut cx.outputs[0];
        let mut start = 0;
        for event in cx.events {
            self.render(&mut output[start..event.frame]);
            self.pending = event.message.0;
            start = event.frame;
        }
        self.render(&mut output[start..]);
    }
}
```

Render up to each event, apply its message, then render from that frame onward.
Events at the same frame arrive in slice order. A zero-frame call still delivers
messages at frame 0, so a pending strike sounds on the next rendered frame.
Overwrite every output sample, including silence. Keep the audio callback
bounded, allocation-free and nonblocking.

For a pitched voice, replace `Strike` with an enum for note-on, note-off and
parameter changes. Set up tuning for the host's sample rate at construction
and keep oscillator phase and envelope state on the instrument.

## Test and connect it

Put this host test in `tests/strike.rs` and run `cargo test -p wavegraph-click`.
Starting with a nonzero buffer also checks that the instrument writes silence.

```rust
//! Check the timing and amplitude of a strike.

use wavegraph_click::{Click, Strike};
use wavegraph_core::{Event, Process, ProcessContext};

#[test]
fn strike_sounds_on_its_frame() {
    let mut samples = [1.0_f32; 4];
    let strike = Event {
        frame: 2,
        port: 0,
        message: Strike(64),
    };
    Click::default().process(&mut ProcessContext {
        frames: samples.len(),
        inputs: &[],
        outputs: &mut [&mut samples],
        events: &[strike],
        emit: &mut Err,
    });
    assert_eq!(samples, [0.0, 0.0, 0.5, 0.0]);
}
```

To play it in a patch, create the instrument once, bind it in the program's
object table, and use `Op::Call` with zero signal inputs and one signal output.
Pass its events to `patch.render` for each block. The [engine example](../wavegraph-dsp/README.md#rendering-a-compiled-patch)
shows the whole path for the monosynth. When several instruments use different
message types, the host can wrap them in one patch-wide enum and dispatch
each variant to its instrument, as the [FM-1 does](../wavegraph-fm1/src/instrument.rs).
