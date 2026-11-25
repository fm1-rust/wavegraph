// Exercise the actual installer against a simulated FM-1 with pre-existing flash data.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [flasher, image, mode = "packed", samplerMode = mode, voicing =
    mode === "packed" && samplerMode === "packed" ? "lofi" : "original"] = process.argv.slice(2);
assert.ok(["raw", "packed"].includes(mode), "expected raw or packed cymbals");
assert.ok(["raw", "packed"].includes(samplerMode), "expected raw or packed sampler");
assert.ok(["original", "lofi"].includes(voicing), "expected original or lofi sampler voicing");
const lib = pathToFileURL(resolve(flasher, "web/lib") + "/");
const { appTable, appRead, appFilesStatus, AppStatus, BOOT_AREA, FLASH_SIZE, SECTOR } =
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
const appEnd = BOOT_AREA + app.addr + app.len;
const assetStart = BOOT_AREA + assets.addr;
const assetEnd = assetStart + assets.len;
assert.equal(assetStart, Math.ceil(appEnd / SECTOR) * SECTOR);
const end = Math.ceil(assetEnd / SECTOR) * SECTOR;
assert.ok(end <= 0xe0000);
assert.equal(raw.length, assets.addr + assets.len - app.addr);
assert.ok(target.subarray(appEnd, assetStart).every(b => b === 0xff));
assert.ok(target.subarray(assetEnd, end).every(b => b === 0xff));
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

function checkAbsent(name, bytes) {
    assert.ok(!upper.includes(bytes) && !code.includes(bytes), `${name} unexpectedly included`);
}

function pcm1Header(frames) {
    const header = Buffer.alloc(8);
    header.write("PCM1");
    header.writeUInt32LE(frames, 4);
    return header;
}

// Find the PCM1 clip by its frame count, then check the installed bit stream
// against the source PCM, including the final, possibly short group.
function checkPackedClip(pcm, name, predictive) {
    const frames = pcm.length / 2;
    const header = pcm1Header(frames);
    const start = upper.indexOf(header);
    assert.ok(start >= 0, name);
    assert.equal(upper.indexOf(header, start + 1), -1, name);
    assert.ok(!upper.includes(pcm), `${name} also present as raw PCM`);
    let offset = start + header.length;
    let sample = 0, delta = 0;
    for (let first = 0; first < frames; first += 32) {
        const width = upper.readUInt8(offset++);
        assert.ok(width >= 1 && width <= 16, name);
        const count = Math.min(32, frames - first);
        for (let i = 0; i < count; i++) {
            let value = 0;
            for (let b = 0; b < width; b++) {
                const bit = i * width + b;
                value |= ((upper.readUInt8(offset + (bit >> 3)) >> (bit & 7)) & 1) << b;
            }
            const sign = 1 << (width - 1);
            let decoded = (value ^ sign) - sign;
            if (predictive) {
                if ((first + i) % 128 === 0) sample = delta = 0;
                delta = (delta + decoded) << 16 >> 16;
                sample = (sample + delta) << 16 >> 16;
                decoded = sample;
            }
            assert.equal(decoded, pcm.readInt16LE(2 * (first + i)), name);
        }
        offset += Math.ceil(count * width / 8);
    }
    assert.ok(!code.includes(upper.subarray(start, offset)), `${name} duplicated in app.bin`);
}

for (const [path, extension] of [
    ["../../instruments/wavegraph-909/samples/", ".s16le"],
    ["../../instruments/wavegraph-wavetable/banks/", ".wt12"],
    ["../samples/rave/", ".s16le"],
    ["../samples/rave/", ".ima"],
    ["../samples/rave/", ".txt"],
    ["../assets/", ".2bpp"],
    ["../assets/panel-type/", ".raw"],
    ["../assets/panel-type/", ".txt"],
    ["../patterns/", ".txt"],
]) {
    const dir = new URL(path, import.meta.url);
    const names = readdirSync(dir).filter(name => name.endsWith(extension));
    assert.ok(names.length, path);
    for (const name of names) {
        const bytes = readFileSync(new URL(name, dir));
        const rave = path === "../samples/rave/";
        const hitMaster = rave && ["house-m7.s16le", "zap.s16le", "stab.s16le"].includes(name);
        const packing = rave ? samplerMode : mode;
        const fullBreak = voicing === "original" && mode === "packed" && samplerMode === "packed";
        const breakName = fullBreak ? "amen.ima" : "amen-compact.ima";
        if (hitMaster && voicing === "lofi") {
            checkAbsent(name, bytes);
            checkAbsent(name + " PCM1", pcm1Header(bytes.length / 2));
        } else if (name.endsWith("-lofi.ima") && voicing === "original") {
            checkAbsent(name, bytes);
        } else if (name.startsWith("amen") && extension === ".ima" && name !== breakName) {
            checkAbsent(name, bytes);
        } else if (extension === ".s16le" && (packing === "packed" || (mode === "raw" && hitMaster))) {
            assert.ok(!code.includes(bytes), `${name} duplicated in app.bin`);
            checkPackedClip(bytes, name, rave);
        } else {
            checkAsset(name, bytes);
        }
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
