"""Convert one WaveEdit WAV to WaveGraph's packed harmonic levels (requires NumPy)."""

import argparse
import wave
from pathlib import Path

import numpy as np


def convert(source, destination):
    with wave.open(str(source), "rb") as wav:
        if (wav.getnchannels(), wav.getsampwidth(), wav.getnframes()) != (1, 2, 64 * 256):
            raise ValueError("expected 64 cycles of 256 mono PCM16 samples")
        pcm = wav.readframes(wav.getnframes())
    frames = np.frombuffer(pcm, dtype="<i2").reshape(64, 256)
    spectrum = np.fft.rfft(frames, axis=1)
    spectrum[:, 0] = 0  # Each cycle must remain DC-free even when held indefinitely.
    levels = []
    for size, partials in [(256, 63), (128, 31), (64, 15), (32, 7), (32, 3), (32, 1)]:
        bins = np.zeros((64, size // 2 + 1), dtype=complex)
        bins[:, 1 : partials + 1] = spectrum[:, 1 : partials + 1]
        levels.append(np.fft.irfft(bins, n=size, axis=1) * (size / 256))
    pcm = np.concatenate(levels, axis=1)
    # A single attenuation for the whole bank preserves dynamics between frames and levels.
    pcm *= min(1.0, 32767.0 / max(1.0, np.abs(pcm).max()))
    # Pack two signed 12-bit samples into three bytes, low nibble first.
    pairs = (np.rint(pcm / 16).clip(-2048, 2047).astype(np.int32) & 0xFFF).reshape(-1, 2)
    a, b = pairs.T
    packed = np.column_stack((a & 255, (a >> 8) | ((b & 15) << 4), b >> 4))
    destination.write_bytes(packed.astype(np.uint8).tobytes())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    convert(args.source, args.destination)
