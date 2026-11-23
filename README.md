# wavegraph.org

Select **Start FM-1 emulator**, then press **Space** or click **PLAY** on
the panel for the factory arrangement. For a local preview, serve this branch
with `python3 -m http.server 8000` and open `http://localhost:8000/`.

The [firmware release workflow](https://github.com/fm1-rust/wavegraph/blob/main/.github/workflows/release-fm1.yml)
updates `emulator/firmware/` with the raw image from the highest stable
`wavegraph-fm1` release. The [firmware manifest](emulator/firmware/current.json)
identifies the image and the source link displayed in the player.

The web core and panel come from [fm1-emu 736aded](https://github.com/joakimeriksson/fm1-emu/tree/736aded65c8e2e9d652cf595fca248734483a877).
The worker places the application at XIP offset `0x120` and boots it at
`0x02000120`. Audio and controls stay in the browser.
