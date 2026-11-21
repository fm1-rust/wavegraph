# wavegraph-303

A heapless, `no_std` Rust implementation of the CHOMPI x0x bass voice and
effects. `Voice::render_add` adds to a mono slice. `Drive::process` applies
CHOMPI's post-VCA, pedal-style overdrive in place, using only its own filter
history. Each bassline can own a separate voice and drive.

```rust
use wavegraph_303::{Drive, Voice, VoiceParams};

let rate = 48_000.0;
let mut voice = Voice::new(rate);
let mut drive = Drive::new(rate);
let params = VoiceParams::default();
let mut block = [0.0; 128];

voice.note_on(36, true, false);
voice.render_add(&params, &mut block[..53]);
voice.note_on(43, false, true); // Glide with the gate still open.
voice.render_add(&params, &mut block[53..]);
drive.process(0.65, &mut block);
```

`Effects` includes the same drive, bit crusher, chorus/flanger, and stereo tape
delay. `Reverb` adds its wet return to an existing stereo block. Both borrow
their delay storage from the caller. `DrumFx` and `Compressor` work in place.

## Timing and math

Split blocks at note or control changes. Panel controls take effect at the
start of each call. The voice advances the ladder twice per output frame and
updates its four cutoff coefficients every fourth frame, carrying that phase
across blocks. A legato slide carries the envelopes across notes. Drive at
or below 0.005 bypasses the signal and freezes its filter history.

The default build depends only on `micromath`. It uses its floor for pitch
and quantization and its exponential for envelope and filter coefficients.
Computing `1 - exp(-1/(τ·fs))` in that order preserves the upstream envelope
rounding. The voice computes its sample-rate-only coefficients at construction.
The filter uses a small logarithm series for the cutoff and a rational
approximation for tangent over its clamped range of 0 to 0.45π.

In six FM-1 emulator measurements, a voice took 126–131k target instructions
per 256 frames. The same slice-based voice using the reference tangent took
492–499k before replacing its remaining `libm` calls. Across 25,425 frames
at four rates, the maximum absolute difference from the mechanical translation
was 2.69e-7 dry and 1.16e-6 after drive, in units of digital full scale.
The release tail ended on the same frame in the sample-by-sample tests.

## Translation

Enable the Cargo feature `raw` to use the original pointer-based API from
PR #12. It preserves the mechanical translation and its Rust `libm` math.
`cargo test -p wavegraph-303 --features raw` checks the slice API against it.
The commands below reproduce `raw.rs`.

[Cpp2Rust at `67b0ea9`](https://github.com/Cpp2Rust/cpp2rust/tree/67b0ea91f5aa7d45ecac74ebb741a9f933914c2c)
translated one C++17 file including `voice.h` and `fx.h`, both of which include
`dsp.h`. The input is available in the
[original WaveGraph import](https://github.com/fm1-rust/wavegraph/tree/f23c3eeec5559ee31fd46d13b9afd58bf4d5cf4c/wavegraph-303/src)
and [upstream CHOMPI source](https://github.com/theotherson/chompi-x0x/tree/40ec807728d4f5fbc9324d1bf41f76fb75f58854/code/src/x0x).
The CHOMPI board firmware, UI, drum synthesizers and sequencer were outside this translation.

The `unsafe` model preserves inline DSP state and pointer-based buffer interfaces.
The freestanding pass maps nine C math functions to `libm`, emits Rust constants
for C++ `constexpr` data, and replaces the translator's runtime support for
integer increments, decrements and one reverb initialization closure. The
crate builds without a heap or C++ compiler.

To regenerate from the workspace root, set `CPP2RUST` to the absolute path of
the built translator. The following commands recover the headers from Git.

```sh
work=$(mktemp -d)
git archive f23c3eeec5559ee31fd46d13b9afd58bf4d5cf4c:wavegraph-303/src/x0x \
    voice.h fx.h dsp.h | tar -x -C "$work"
printf '%s\n' '#include "voice.h"' '#include "fx.h"' > "$work/model.cpp"
(cd "$work" && "$CPP2RUST" --model=unsafe --file=model.cpp \
    --cxxflags=-std=c++17 --cxxflags="-I$work" -o=model.rs)
python3 wavegraph-303/tools/freestanding.py < "$work/model.rs" > wavegraph-303/src/raw.rs
rustfmt +1.100.0 --edition 2024 wavegraph-303/src/raw.rs
```

The intermediate `model.rs` had SHA-256
`841fcdbabfed5412ce10341f9eae00402e2c68036f65c56bb914c87f6cc4e211` with
Clang 23.1.6 on x86-64 Linux. The original MIT notice is in
[LICENSE](https://github.com/fm1-rust/wavegraph/blob/f23c3eeec5559ee31fd46d13b9afd58bf4d5cf4c/wavegraph-303/LICENSE).
