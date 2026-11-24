// Exercise the actual installer against a simulated FM-1 with pre-existing flash data.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [flasher, image] = process.argv.slice(2);
const lib = pathToFileURL(resolve(flasher, "web/lib") + "/");
const { appTable, appRead, appFilesStatus, AppStatus, BOOT_AREA, FLASH_SIZE } =
    await import(new URL("formats.js", lib));
const { CHIP_KEY, FM1, planFirmware } = await import(new URL("unit.js", lib));
const previous = new Uint8Array(FLASH_SIZE).fill(0x5a);
const session = { flash: previous, key: CHIP_KEY, analysis: { board: FM1 } };
const update = readFileSync(image + ".fwsc");
const { target } = planFirmware(session, update);
const raw = readFileSync(image + ".bin");
const files = appTable(target, CHIP_KEY).filter(e => e.attr === 0x82);
assert.deepEqual(files.map(e => e.name), ["app.bin", "assets.bin"]);
const [app, assets] = files;
assert.equal(app.addr, 0x120);
assert.equal(assets.addr + BOOT_AREA, 0x93000);
assert.ok(app.addr + app.len <= assets.addr);
const end = Math.ceil((BOOT_AREA + assets.addr + assets.len) / 4096) * 4096;
assert.ok(end <= 0xe0000);
assert.equal(raw.length, assets.addr + assets.len - app.addr);
assert.ok(target.subarray(BOOT_AREA + app.addr + app.len, 0x93000).every(b => b === 0xff));
assert.deepEqual(target.subarray(0, BOOT_AREA), previous.subarray(0, BOOT_AREA));
assert.deepEqual(target.subarray(end), previous.subarray(end));
assert.equal(appFilesStatus(target, CHIP_KEY), AppStatus.OK);
assert.deepEqual(planFirmware({ ...session, flash: target }, update).sectors, []);
for (const file of files) {
    const installed = appRead(target, BOOT_AREA, file.addr, file.len, CHIP_KEY);
    const offset = file.addr - app.addr;
    assert.deepEqual(Buffer.from(installed), raw.subarray(offset, offset + file.len));
    const damaged = target.slice();
    damaged[BOOT_AREA + file.addr] ^= 1;
    assert.equal(appFilesStatus(damaged, CHIP_KEY), AppStatus.BAD_CRC);
}

// Catch accidental copies left in code, including references to a library's stock banks.
const code = raw.subarray(0, app.len);
const upper = raw.subarray(assets.addr - app.addr);
function checkAsset(name, bytes) {
    const at = upper.indexOf(bytes);
    assert.ok(at >= 0, `${name} missing from assets.bin`);
    assert.equal(upper.indexOf(bytes, at + 1), -1, `${name} repeated in assets.bin`);
    assert.ok(!code.includes(bytes), `${name} duplicated in app.bin`);
}

for (const [path, extension] of [
    ["../../instruments/wavegraph-909/samples/", ".s16le"],
    ["../../instruments/wavegraph-wavetable/banks/", ".wt12"],
    ["../assets/", ".2bpp"],
    ["../assets/panel-type/", ".raw"],
    ["../assets/panel-type/", ".txt"],
    ["../patterns/", ".txt"],
]) {
    const dir = new URL(path, import.meta.url);
    const names = readdirSync(dir).filter(name => name.endsWith(extension));
    assert.ok(names.length, path);
    for (const name of names) {
        checkAsset(name, readFileSync(new URL(name, dir)));
    }
}

// The build generates these three atlases. Use the locked crate's originals
// as independent references when checking the packaged firmware.
const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--format-version=1"], {
    cwd: new URL("../", import.meta.url),
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
}));
const graphics = metadata.packages.filter(pkg => pkg.name === "embedded-graphics");
assert.equal(graphics.length, 1, "expected one locked embedded-graphics version");
const ascii = resolve(dirname(graphics[0].manifest_path), "fonts/raw/ascii");
for (const name of ["font_6x10.raw", "font_7x13_bold.raw", "font_9x15_bold.raw"]) {
    checkAsset(name, readFileSync(resolve(ascii, name)));
}
