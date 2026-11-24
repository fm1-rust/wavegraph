#!/bin/sh
# Link an FM-1 binary or example from the selected Cargo project.
set -eu
: "${RUSTC:?set RUSTC to the patched rustc}"
: "${PI32V2_BIN:?set PI32V2_BIN to the patched LLVM bin directory}"
: "${PI32V2_TARGET:?set PI32V2_TARGET to the BSP pi32v2-none.json}"
export RUSTC_BOOTSTRAP=1
export CARGO_TARGET_PI32V2_NONE_LINKER="$PI32V2_BIN/ld.lld"
# SRAM interrupt and worker code can call any XIP routine using call.xl.
export RUSTFLAGS="-C link-arg=-Twavegraph-fm1.ld -C link-arg=--wrap=_start -C llvm-args=-pi32v2-large-program=true"
exec cargo build --locked --profile firmware --target "$PI32V2_TARGET" \
    -Zbuild-std=core,compiler_builtins -Zbuild-std-features=compiler-builtins-mem \
    -Zjson-target-spec "$@"
