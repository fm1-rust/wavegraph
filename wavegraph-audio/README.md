# wavegraph-audio

Route a WaveGraph master to an audio device, or read a selected input into a graph.
The portable interface uses the FM-1's native render size, 128 stereo frames of
signed Q23 PCM in i32 accumulators. A graph can keep its mono buses and processor
state across device changes. It only borrows the output block during a refill.

## Play on a Mac

From the workspace root with the Xcode command line tools installed:

```sh
brew install sdl3 pkgconf
cargo run --release --locked -p wavegraph-audio --features coreaudio --example play -- --list
cargo run --release --locked -p wavegraph-audio --features coreaudio --example play
```

The second command plays a four-note monosynth phrase through a WaveGraph
bytecode patch with a gain node. Change the default output in System Settings
while it plays. To pin to channels 3/4 of an interface, append
`-- --output "Interface Name" 3`. The CLI uses one-based numbers. The Rust API
uses zero-based channel indices. Ctrl-C stops playback.

## Where mixing belongs

| Operation | macOS | FM-1 |
| --- | --- | --- |
| Instrument levels, inserts, post-fader sends, return sum, master dynamics | WaveGraph owns the signal order and sample-timed controls. Use the existing `wavegraph-fx` nodes. | Run the same graph, including `ChannelMixer` and `MasterBus` where the patch uses them. |
| Mix this application's output with other clients | Submit one WaveGraph master. SDL can mix logical streams bound to a device. CoreAudio handles shared hardware playback with other applications. | Firmware owns the DAC. Sum additional local sources in the staging mix before MASTER. `fm1_audio::mix_stereo` already adds planar I24 sources with i32 headroom. |
| Final level and device PCM | The adapter clips Q23 to signed 24-bit range and aligns it in an S32 stream. SDL converts the stream to the negotiated device format and rate. System output volume remains under the user's control. | `fm1_audio::Output` applies the panel MASTER ramp and final Q23 clip once before copying to the released DMA half. |
| Clock and buffering | The graph chooses its rate. SDL buffers and converts between the stream and device. CoreAudio Audio Queue Services handles hardware I/O. | Tune to the exact `24_000_000 / 544` clock. CPU0 services timed refills. CPU1 keeps exclusive ownership of outstanding DSP work. |
| Device changes | SDL's CoreAudio backend listens for device-list and default-device changes. The application re-scans and reconciles its selection. | The local adapter exposes one fixed stereo DAC. WaveGraph FM-1 also sends the pre-MASTER stereo mix to a connected computer over USB. |

Apple also ships mixer Audio Units. Substituting one for a WaveGraph track mixer
would change who implements the patch's gain law, headroom and automation.
The Mac example renders the patch in Rust. The backend receives the resulting master.

## Routing and lifetime

Import `DeviceHost` and use `host.devices()` on the control thread for input and output selectors. The
snapshot carries display names, session-local IDs, channel counts, backend-reported
rates, and the current default names. Keep the selected `Route` as application state.
Call `Input::sync` or `Output::sync` at startup, after a route selection and on a
periodic scan. The example scans every 250 ms and services PCM between scans.

- `Route::Default` follows the system default with a logical stereo stream.
- `Route::Device(id, Channels::Pair(2))` selects the third and fourth channels in
  SDL's device layout. A mono input is duplicated and a mono output averages L/R.
  An explicit port goes offline when its device or channels disappear. A replug can
  get a new instance ID and requires reselection. Display names can collide.
- `Playback::render` calls the renderer only when there is room. The desktop queue
  targets two device periods with a floor of four graph blocks. `Written` means the
  queue accepted PCM. It provides no DAC timestamp. Changing the selected route
  can discard queued samples. Keep synth state outside the port.
- `Capture::read` yields a complete stereo block or leaves the destination alone.
  Open an input only when recording is desired, drain it regularly and drop it when
  finished. SDL buffers unattended capture. Opening a microphone requires macOS
  permission. Input and output can have independent clocks. This release does not
  provide drift-compensated monitoring between two devices.
- On FM-1, service `Dac(output, master).render(...)` in the ALNK0 task.
  It passes the BSP's exact `RefillInterval` before sample selection, preserves
  `Written`, `Late` and `WouldBlock`, and borrows the existing staging block.
  The firmware's handoff rejects stale CPU1 completions, leaves the cleared half
  silent on a miss, and acknowledges edits only for a current, written block.

To connect a capture port to a graph source, keep the block with the host and
hand it to the source node only after a full read. For example, inside a Mac
application with the `coreaudio` feature:

```rust,no_run
# #[cfg(all(target_os = "macos", feature = "coreaudio"))]
# fn capture() -> std::io::Result<()> {
use core::num::NonZeroU32;
use wavegraph_audio::{Capture, DeviceHost, FRAMES, coreaudio::{Host, Input, Route}};

let host = Host::new()?;
let mut input = Input::new(NonZeroU32::new(48_000).unwrap());
input.sync(&host, &host.devices()?, Route::Default)?;
let mut block = [[0; 2]; FRAMES];
// In the application's audio service loop:
if input.read(&mut block)? {
    let left = block.map(|[left, _]| left);
    // The graph's input node can now consume the complete left plane.
    # core::hint::black_box(left);
}
# Ok(())
# }
```

The crate forbids unsafe Rust. Desktop reads and writes cross the SDL boundary
as synchronous copies of bounded byte slices. A user's renderer can panic or
keep state on the stack without leaving an outstanding Rust pointer in CoreAudio.
Forgetting a port can leak the owned native stream. It cannot leave the backend
with a borrow of an application's buffer. The FM-1 adapter relies on the safe
CPU0 refill API in `fm1-audio`. SDL3, its Rust bindings and the FM-1 BSP are
trusted dependencies. Desktop queue operations can take locks and allocate
inside SDL, so service them outside a hard-real-time audio callback.

## Backend references

- [Apple Audio Queue Services](https://developer.apple.com/documentation/audiotoolbox/audio-queue-services)
- [Apple guide to mixing and converting audio](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/CoreAudioOverview/ARoadmaptoCommonTasks/ARoadmaptoCommonTasks.html)
- [SDL logical devices, default-device migration and mixing](https://wiki.libsdl.org/SDL3/SDL_OpenAudioDevice)
- [SDL stream copy contract](https://wiki.libsdl.org/SDL3/SDL_PutAudioStreamData)
- [SDL CoreAudio driver](https://github.com/libsdl-org/SDL/blob/release-3.4.20/src/audio/coreaudio/SDL_coreaudio.m)
