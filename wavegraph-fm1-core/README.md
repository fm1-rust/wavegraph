# wavegraph-fm1-core

Run a WaveGraph patch on CPU1 of the M-VAVE FM-1. CPU0 owns RTICX, the DAC,
and application tasks. Each render covers 128 stereo frames at about 44.1 kHz.
The runtime transfers one application-owned block between cores without allocation.

The [tone example](examples/tone.rs) is a complete firmware. It binds a
monosynth to a one-node bytecode program and plays A4. The
[WaveGraph app](../wavegraph-fm1/src/firmware.rs) adds panel controls, async
graph edits, MIDI, scope capture and worker timing to the same driver.

## Build the tone

From the workspace root with the FM-1 patched Rust compiler and LLVM tools,
set `RUSTC`, `PI32V2_BIN`, `PI32V2_TARGET` and `PI32V2_ISA` as for
[the demo build](../wavegraph-fm1/build.sh). Then run

```sh
wavegraph-fm1-core/build.sh -p wavegraph-fm1-core --example tone
image=target/pi32v2-none/firmware/examples/tone
python3 "$PI32V2_ISA/tools/hwtest/fm1app.py" "$image.fwsc" --from-elf "$image"
```

## Start a separate application

With the WaveGraph checkout and your Cargo project in sibling directories,
copy `examples/tone.rs` to your project's `src/main.rs`. Use edition 2024
and these entries in your workspace root manifest for the example.

```toml
[dependencies]
fm1-audio = "0.2.1"
fm1-rticx = "0.2.1"
static_cell = "2.1.1"
wavegraph-dsp = { path = "../wavegraph/wavegraph-dsp" }
wavegraph-fm1-core = { path = "../wavegraph/wavegraph-fm1-core" }
wavegraph-monosynth = { path = "../wavegraph/instruments/wavegraph-monosynth" }
wavegraph-sample = { path = "../wavegraph/wavegraph-sample" }

[profile.firmware]
inherits = "release"
opt-level = 3
codegen-units = 1
lto = true
panic = "abort"
```

The graph uses `wavegraph_sample::I24` and writes signed Q23 `i32` samples
to the DAC. Generate a lockfile with `cargo generate-lockfile`, then run
`../wavegraph/wavegraph-fm1-core/build.sh` in the new project. The helper
links the crate's startup archive and linker script with the FM-1 large-program
code model. The ELF appears under `target/pi32v2-none/firmware/` with your
binary's name. The command above packages a code-only app. The full demo's
build script shows how to package a separate upper-flash asset section.

## Wire in a graph

1. Implement `Render` on the owner of the patch and declare it once with
   `dsp_worker!(static DSP: MyRenderer)`. Its block can be a mono Q23 array,
   a stereo `OutputBlock`, or a struct implementing `AudioBlock::write`.
   Put control snapshots and returned telemetry in that struct. Keep large
   buses and node tables in static storage and move their exclusive references.
2. Call `unsafe { peer::start() }` once at the beginning of RTICX init, before
   `Board::take()` and with interrupts disabled. Status 1 means CPU1 passed
   the BSP handshake. Take the board, split an `EditQueue<P>`, then pass the
   renderer, a static mutable block, and the edit host to `DSP.connect(...)`.
   The edit controller and host both stay on CPU0. CPU1 reads the snapshots
   in the block and owns the graph across calls.
3. Use `driver.request` to prepare the first block and obtain its revision from
   `host.begin_block`. Prime it with `wait_first_blocking()` in init, or
   `wait_first().await` in a task after starting the RTICX deadline clock.
   Call `start_output` while ALNK0 is masked and give the output and driver
   to its handler before unmasking it.
4. In ALNK0, call `driver.refill(&mut output, master)`. On `Ok(Some(half))`,
   use any returned telemetry, then call `driver.request` for the next block.
   `Ok(None)` means no DMA half was due. Rendering can skip frames after a
   missed deadline. Use the supplied `first_frame` to advance sequencers.
   Only current PCM written to the DAC acknowledges a graph edit.
   Use `refill_with_tap` to read the pre-MASTER job and post-MASTER stereo
   samples after a confirmed write. A silent fallback passes `None` for the
   job. A late or unserviced half skips the tap.
5. A terminal worker fault holds CPU1. Drop the driver to close pending edits.
   The demo keeps serving silence and displays the fault. The tone example
   uses `peer::download()` in its panic handler to return to the bootloader.

The linker reserves a 12 KiB CPU1 user stack and checks a 1 KiB margin under
the CPU0 stacks. Use the demo's stack watermark and deadline measurements
when sizing a larger renderer.
