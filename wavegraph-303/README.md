# wavegraph-303

A freestanding Rust translation of the CHOMPI x0x bass voice and effects. The
`raw` module exposes the oscillator, diode ladder, envelopes, accent and slide
from `voice.h`, along with the post-VCA bass drive and the rest of `fx.h`.

## Translation

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
Clang 23.1.6 on x86-64 Linux. The original MIT notice is in [LICENSE](LICENSE).
