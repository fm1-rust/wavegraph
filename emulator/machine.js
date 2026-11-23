// Keep emulation off the UI thread. The browser requests another block as its
// audio queue drains, so the FM-1 advances at the sound device's pace.
import { jitImports } from "./vendor/jit.js";

const clockMHz = 240;
const quantum = 128;
const blockSize = 1024;
let machine;
let blocks = 0;
let lcdRevision = -1;
const pressedAt = new Map();
const releaseAt = new Map();

async function boot(module, firmware) {
  const instance = await WebAssembly.instantiate(module, { env: jitImports(() => machine) });
  machine = instance.exports;
  machine.fm1_jit(1, 16);
  machine.fm1_ahead(1024);

  // The raw CI application is linked 0x120 bytes into the XIP window.
  const offset = 0x120;
  const size = offset + firmware.byteLength;
  const ptr = machine.fm1_alloc(size);
  const image = new Uint8Array(machine.memory.buffer, ptr, size);
  image.fill(0, 0, offset);
  image.set(new Uint8Array(firmware), offset);
  machine.fm1_boot(ptr, size, 0x02000120, clockMHz);
  postMessage({ type: "ready" });
}

function setKey(id, down) {
  if (down) {
    releaseAt.delete(id);
    pressedAt.set(id, machine.fm1_ms());
    machine.fm1_key(id, 1);
  } else if (pressedAt.has(id)) {
    // Even a tap between two audio requests must reach the scanned key matrix.
    releaseAt.set(id, pressedAt.get(id) + 22);
    pressedAt.delete(id);
  }
}

function render() {
  const left = new Float32Array(blockSize);
  const right = new Float32Array(blockSize);
  for (let offset = 0; offset < blockSize; offset += quantum) {
    for (const [id, when] of releaseAt) {
      if (machine.fm1_ms() >= when) {
        machine.fm1_key(id, 0);
        releaseAt.delete(id);
      }
    }
    machine.fm1_render(quantum, clockMHz * 20000);
    if (machine.fm1_halted()) throw new Error("The emulated FM-1 halted.");
    const mem = machine.memory.buffer;
    left.set(new Float32Array(mem, machine.fm1_out_l(), quantum), offset);
    right.set(new Float32Array(mem, machine.fm1_out_r(), quantum), offset);
  }

  const message = { type: "audio", left, right };
  const transfer = [left.buffer, right.buffer];
  if (blocks++ % 2 === 0) {
    const mem = machine.memory.buffer;
    const revision = machine.fm1_lcd_revision();
    if (revision !== lcdRevision) {
      message.lcd = new Uint16Array(mem, machine.fm1_lcd(), 240 * 240).slice();
      transfer.push(message.lcd.buffer);
      lcdRevision = revision;
    }
    message.leds = new Float32Array(mem, machine.fm1_leds(), 41).slice();
    transfer.push(message.leds.buffer);
  }
  postMessage(message, transfer);
}

self.onmessage = async ({ data }) => {
  try {
    switch (data.type) {
      case "boot": await boot(data.module, data.firmware); break;
      case "render": render(); break;
      case "key": setKey(data.id, data.down); break;
      case "encoder": machine.fm1_enc(data.id, data.clicks); break;
      case "master": machine.fm1_pot(data.value); break;
    }
  } catch (error) {
    postMessage({ type: "error", message: error.message });
  }
};
