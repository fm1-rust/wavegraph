"""Render WaveGraph's rising rave siren as a mono WAV on stdout (CC0-1.0)."""

import math
import struct
import sys
import wave


def render():
    rate = 14_700
    frames = 19_404
    dry = []
    phase = 0.0
    detuned = 0.18
    harmonics = (0.87, 0.34, 0.30, 0.17, 0.09, 0.08)
    for i in range(frames):
        t = i / rate
        hz = 355 + 512 * (1 - math.exp(-2.75 * t))
        phase += math.tau * hz / rate
        detuned += math.tau * hz * 1.007 / rate
        buzz = sum(gain * math.sin(n * phase) for n, gain in enumerate(harmonics, 1))
        dry.append(buzz + 0.22 * math.sin(detuned) + 0.10 * math.sin(3 * detuned))

    # Short, darker repeats let earlier pitches overlap the rising oscillator.
    taps = ((0.17, 0.39), (0.34, 0.17))
    samples = []
    for i, direct in enumerate(dry):
        value = direct
        for seconds, gain in taps:
            j = i - round(seconds * rate)
            if j > 0:
                value += gain * (dry[j] + dry[j - 1]) / 2
        fade = min(1, i / (0.006 * rate), (frames - 1 - i) / (0.007 * rate))
        samples.append(math.tanh(0.86 * value) * fade)
    scale = 29196 / max(map(abs, samples))
    with wave.open(sys.stdout.buffer, "wb") as output:
        output.setparams((1, 2, rate, frames, "NONE", ""))
        output.writeframes(b"".join(struct.pack("<h", round(s * scale)) for s in samples))


if __name__ == "__main__":
    render()
