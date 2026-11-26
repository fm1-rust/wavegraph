#!/bin/sh
# Build a flashable FM-1 application with the FM-1 BSP's pi32v2 tools.
set -eu
: "${PI32V2_ISA:?set PI32V2_ISA to the pi32v2-isa checkout}"
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
out=${CARGO_TARGET_DIR:-"$root/target"}
# Keep checkout and dependency cache paths out of limited application flash.
"$root/wavegraph-fm1-core/build.sh" --manifest-path "$root/Cargo.toml" \
    -p wavegraph-fm1 --target-dir "$out" -Ztrim-paths \
    --config 'profile.firmware.trim-paths="object"' "$@"
image="$out/pi32v2-none/firmware/wavegraph-fm1"
PYTHONPATH="$PI32V2_ISA/tools/hwtest" python3 "$root/wavegraph-fm1/assets/package.py" "$image"
