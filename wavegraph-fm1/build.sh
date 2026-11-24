#!/bin/sh
# Build a flashable FM-1 application with the FM-1 BSP's pi32v2 tools.
set -eu
: "${RUSTC:?set RUSTC to the patched rustc}"
: "${PI32V2_BIN:?set PI32V2_BIN to the patched LLVM bin directory}"
: "${PI32V2_TARGET:?set PI32V2_TARGET to the BSP pi32v2-none.json}"
: "${PI32V2_ISA:?set PI32V2_ISA to the pi32v2-isa checkout}"
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
export RUSTC_BOOTSTRAP=1
export CARGO_TARGET_PI32V2_NONE_LINKER="$PI32V2_BIN/ld.lld"
# SRAM interrupt and worker code can call any XIP routine using call.xl.
export RUSTFLAGS="-C link-arg=-Twavegraph-fm1.ld -C llvm-args=-pi32v2-large-program=true"
out=${CARGO_TARGET_DIR:-"$root/target"}
cargo build --manifest-path "$root/Cargo.toml" --locked -p wavegraph-fm1 --profile firmware \
    --target "$PI32V2_TARGET" --target-dir "$out" "$@" \
    -Zbuild-std=core,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec
image="$out/pi32v2-none/firmware/wavegraph-fm1"
PYTHONPATH="$PI32V2_ISA/tools/hwtest" python3 "$root/wavegraph-fm1/assets/package.py" "$image"
