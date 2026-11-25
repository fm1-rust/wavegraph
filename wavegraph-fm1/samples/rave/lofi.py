"""Rebuild the low-rate factory hits from their PCM16 masters with FFmpeg."""

from pathlib import Path
from struct import iter_unpack, pack
from subprocess import PIPE, run


HERE = Path(__file__).resolve().parent
# Name, master rate, playback rate and quantizer depth before IMA encoding.
HITS = [
    ("house-m7", 16_000, 8_000, 10),
    ("stab", 14_706, 7_353, 8),
    ("zap", 22_059, 14_706, 12),
]


def ffmpeg(pcm, rate, *args):
    return run(
        ["ffmpeg", "-nostdin", "-loglevel", "error", "-f", "s16le", "-ar", str(rate),
         "-ac", "1", "-i", "pipe:0", "-map", "0:a", *args, "pipe:1"],
        input=pcm, stdout=PIPE, check=True,
    ).stdout


def render(name, master_hz, hz, bits):
    master = (HERE / f"{name}.s16le").read_bytes()
    pcm = ffmpeg(master, master_hz, "-af", f"aresample={hz}:dither_method=none", "-f", "s16le")
    step = 1 << (16 - bits)
    pcm = b"".join(
        pack("<h", min(32_768 - step, (sample + step // 2) // step * step))
        for (sample,) in iter_unpack("<h", pcm)
    )
    # A mono 64-byte IMA block contains one predictor and 120 differences.
    data = ffmpeg(pcm, hz, "-c:a", "adpcm_ima_wav", "-block_size", "64", "-f", "data")
    frames = len(pcm) // 2
    assert len(data) == (frames + 120) // 121 * 64
    (HERE / f"{name}-lofi.ima").write_bytes(data)
    print(f"{name}: {frames} frames at {hz} Hz, {len(data)} bytes")


if __name__ == "__main__":
    for hit in HITS:
        render(*hit)
