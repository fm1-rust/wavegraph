# wavegraph.org

Select **Start FM-1 emulator**, then press **Space** or click **PLAY** on
the panel for the factory arrangement. For a local preview, serve this branch
with `python3 -m http.server 8000` and open `http://localhost:8000/`.

The demo pins the raw `wavegraph-fm1.bin` from [main build a429eb52](https://github.com/fm1-rust/wavegraph/actions/runs/45244034966)
and the web core and panel from [fm1-emu 736aded](https://github.com/joakimeriksson/fm1-emu/tree/736aded65c8e2e9d652cf595fca248734483a877).
The worker places the application at XIP offset `0x120` and boots it at
`0x02000120`. Audio and controls stay in the browser.

To update the demo by hand, copy the successful main build’s raw binary
into `emulator/` under its commit-specific name. Update the filename in
`player.js`, the build link in `index.html` and the reference above.
