"""Render WaveGraph's original Dm7 house stab as mono PCM16 WAV (CC0-1.0)."""

import math
import struct
import sys
import wave

RATE = 16_000
FRAMES = 8_000
# D4 F4 A4 C5. Small differences in strike time keep the tines articulate.
VOICING = ((62, 0.00, 1.0), (65, 0.0018, 0.81), (69, 0.0006, 0.85), (72, 0.0031, 0.74))


def render():
    dry = [0.0] * FRAMES
    for key, onset, level in VOICING:
        hz = 440 * 2 ** ((key - 69) / 12)
        for i in range(round(onset * RATE), FRAMES):
            t = max(0.0, i / RATE - onset)
            p = math.tau * hz * t
            # A bright FM tine loses its upper partials faster than its body.
            tine = math.sin(p + (0.23 + 2.1 * math.exp(-t / 0.049)) * math.sin(2 * p))
            bark = math.sin(2 * p + 0.65 * math.sin(3 * p)) * math.exp(-t / 0.040)
            hammer = (1 - math.exp(-t / 0.0016)) * math.exp(-t / 0.145)
            # Two slightly detuned reed ranks bloom under the struck chord.
            reed = 0.0
            for cents, phase in ((-6, 0.8), (5, 2.1)):
                angle = p * 2 ** (cents / 1200) + phase
                reed += sum(math.sin(h * angle) / h for h in (1, 2, 3, 4, 5, 6))
            bloom = (1 - math.exp(-t / 0.012)) * math.exp(-t / 0.118)
            dry[i] += level * (hammer * (tine + 0.28 * bark) + 0.19 * bloom * reed)
            if key == 62:
                dry[i] += 0.75 * hammer * math.sin(p / 2)  # D3 under the chord

    # Print a dark early reflection and a softer repeat into the one-shot.
    body = [math.tanh(0.50 * v) for v in dry]
    printed = body.copy()
    for delay, gain in ((0.026, 0.24), (0.052, 0.10)):
        offset = round(delay * RATE)
        for i in range(offset + 1, FRAMES):
            j = i - offset
            printed[i] += gain * (body[j] + body[j - 1]) / 2
    peak = max(map(abs, printed))
    pcm = bytearray()
    for i, value in enumerate(printed):
        fade = min(1.0, (FRAMES - 1 - i) / (RATE * 0.045))
        pcm.extend(struct.pack("<h", round(12_000 * value * fade / peak)))
    with wave.open(sys.stdout.buffer, "wb") as out:
        out.setparams((1, 2, RATE, FRAMES, "NONE", ""))
        out.writeframes(pcm)


if __name__ == "__main__":
    render()
