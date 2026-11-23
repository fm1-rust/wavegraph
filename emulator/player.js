const hero = document.querySelector(".scope");
const player = document.querySelector("#player");
const canvas = document.querySelector("#panel");
const display = canvas.getContext("2d", { alpha: false });
const status = document.querySelector("#player-status");
const start = document.querySelector("#power-on");
const stop = document.querySelector("#power-off");
const sampleRate = 44100;
let session = null;

async function asset(path, signal) {
  const response = await fetch(new URL(path, import.meta.url), { signal });
  if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
  return response.arrayBuffer();
}

function powerOff(message = "") {
  const active = session;
  session = null;
  active?.abort.abort();
  active?.worker?.terminate();
  if (active?.audio) void active.audio.close().catch(() => {});
  held.clear();
  pointers.clear();
  canvas.hidden = true;
  hero.hidden = !!message;
  player.hidden = !message;
  status.hidden = !message;
  status.textContent = message;
  stop.textContent = message ? "Dismiss" : "Power off";
  start.hidden = false;
  start.focus({ preventScroll: true });
}

function requestAudio(s) {
  if (session !== s || !s.ready || s.pending || s.audio.state !== "running") return;
  // Keep three blocks queued. The AudioContext resamples the FM-1's 44.1 kHz
  // stereo output when the device uses a different sample rate.
  if (s.nextTime - s.audio.currentTime >= 3 * 1024 / sampleRate) return;
  s.pending = true;
  s.worker.postMessage({ type: "render" });
}

function draw(s, frame) {
  const panel = s.panel;
  if (frame.lcd) {
    const ptr = panel.panel_lcd();
    new Uint16Array(panel.memory.buffer, ptr, frame.lcd.length).set(frame.lcd);
  }
  new Float32Array(panel.memory.buffer, panel.panel_leds(), frame.leds.length).set(frame.leds);
  const ptr = panel.panel_draw();
  const pixels = new Uint8ClampedArray(panel.memory.buffer, ptr, canvas.width * canvas.height * 4);
  display.putImageData(new ImageData(pixels, canvas.width, canvas.height), 0, 0);
}

function schedule(s, frame) {
  s.pending = false;
  const buffer = s.audio.createBuffer(2, frame.left.length, sampleRate);
  buffer.copyToChannel(frame.left, 0);
  buffer.copyToChannel(frame.right, 1);
  const source = s.audio.createBufferSource();
  source.buffer = buffer;
  source.connect(s.audio.destination);
  const now = s.audio.currentTime;
  if (s.nextTime <= now) s.nextTime = now + 0.045;
  source.start(s.nextTime);
  s.nextTime += buffer.duration;
  source.onended = () => { source.disconnect(); requestAudio(s); };
  requestAudio(s);
  if (frame.leds) draw(s, frame);
}

async function powerOn() {
  if (session) return;
  const s = session = { abort: new AbortController(), nextTime: 0, pending: false, ready: false,
    clicks: new Int32Array(7), master: 900 };
  start.hidden = true;
  hero.hidden = true;
  player.hidden = false;
  status.hidden = false;
  status.textContent = "Starting the FM-1…";
  stop.textContent = "Power off";
  stop.focus({ preventScroll: true });
  player.scrollIntoView({ block: "start" });
  try {
    // Resume during the click, before awaiting downloads (including on iOS).
    s.audio = new AudioContext({ latencyHint: "interactive" });
    const [module, firmware] = await Promise.all([
      asset("vendor/fm1_web.wasm", s.abort.signal).then(bytes => WebAssembly.compile(bytes)),
      asset("wavegraph-a429eb52.bin", s.abort.signal),
      s.audio.resume(),
    ]);
    if (session !== s) return;
    s.panel = (await WebAssembly.instantiate(module, { env: { fm1_jit_compile: () => 0 } })).exports;
    if (session !== s) return;
    s.panel.panel_theme(2); // Black FM-1
    canvas.width = s.panel.panel_width();
    canvas.height = s.panel.panel_height();
    s.worker = new Worker(new URL("machine.js", import.meta.url), { type: "module" });
    s.worker.onerror = event => {
      if (session === s) powerOff(`Couldn’t run the FM-1. ${event.message || "Try starting it again."}`);
    };
    s.worker.onmessage = ({ data }) => {
      if (session !== s) return;
      try {
        if (data.type === "error") throw new Error(data.message);
        if (data.type === "ready") {
          s.ready = true;
          canvas.hidden = false;
          status.hidden = true;
          canvas.focus({ preventScroll: true });
          requestAudio(s);
        }
        if (data.type === "audio") schedule(s, data);
      } catch (error) { powerOff(`Couldn’t run the FM-1. ${error.message}`); }
    };
    s.audio.onstatechange = () => requestAudio(s);
    s.worker.postMessage({ type: "boot", module, firmware }, [firmware]);
  } catch (error) {
    if (session === s) powerOff(`Couldn’t start the FM-1. ${error.message}`);
  }
}

const held = new Map();
const pointers = new Map();
function press(source, id) {
  if (held.has(source)) return;
  const alreadyDown = [...held.values()].includes(id);
  held.set(source, id);
  if (alreadyDown) return;
  session.panel.panel_set(0, id, 1);
  session.worker.postMessage({ type: "key", id, down: true });
}
function release(source) {
  const id = held.get(source);
  if (!held.delete(source) || [...held.values()].includes(id)) return;
  session.panel.panel_set(0, id, 0);
  session.worker.postMessage({ type: "key", id, down: false });
}
function turn(index, clicks) {
  const s = session;
  if (index === 7) {
    s.master = Math.max(0, Math.min(1023, s.master + clicks * 24));
    s.panel.panel_set(2, 0, s.master);
    s.worker.postMessage({ type: "master", value: s.master });
  } else {
    s.clicks[index] += clicks;
    s.panel.panel_set(1, index, s.clicks[index]);
    s.worker.postMessage({ type: "encoder", id: s.panel.panel_enc_id(index), clicks });
  }
}
function hit(event) {
  const rect = canvas.getBoundingClientRect();
  return session.panel.panel_hit((event.clientX - rect.left) * canvas.width / rect.width,
    (event.clientY - rect.top) * canvas.height / rect.height);
}

canvas.addEventListener("pointerdown", event => {
  if (!session?.ready || event.button !== 0) return;
  const control = hit(event);
  if (control < 0) return;
  canvas.focus({ preventScroll: true });
  canvas.setPointerCapture(event.pointerId);
  pointers.set(event.pointerId, { control, y: event.clientY });
  if (control >= 100) press(event.pointerId, control - 100);
});
canvas.addEventListener("pointermove", event => {
  const pointer = pointers.get(event.pointerId);
  if (!pointer || pointer.control > 7) return;
  const clicks = Math.trunc((pointer.y - event.clientY) / 6);
  if (clicks) {
    turn(pointer.control, clicks);
    pointer.y -= clicks * 6;
  }
});
for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
  canvas.addEventListener(type, event => { pointers.delete(event.pointerId); release(event.pointerId); });
}
const notes = ["KeyA", "KeyW", "KeyS", "KeyE", "KeyD", "KeyF", "KeyT", "KeyG", "KeyY", "KeyH",
  "KeyU", "KeyJ", "KeyK", "KeyO", "KeyL", "KeyP", "Semicolon", "Quote"];
canvas.addEventListener("keydown", event => {
  if (!session?.ready || event.altKey || event.ctrlKey || event.metaKey) return;
  const note = notes.indexOf(event.code);
  if (note < 0 && event.code !== "Space") return;
  event.preventDefault();
  if (!event.repeat) press(event.code, note < 0 ? 12 : 21 + note);
});
canvas.addEventListener("keyup", event => release(event.code));
function releaseAll() {
  for (const source of held.keys()) release(source);
  pointers.clear();
}
canvas.addEventListener("blur", releaseAll);
window.addEventListener("blur", releaseAll);
window.addEventListener("pagehide", () => { if (session) powerOff(); });
start.addEventListener("click", powerOn);
stop.addEventListener("click", () => powerOff());
start.hidden = false;
