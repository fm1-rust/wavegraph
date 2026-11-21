"""Rebuild the LCD wordmark from the original artwork. Only this tool uses Pillow."""
from pathlib import Path

from PIL import Image

directory = Path(__file__).resolve().parent
with Image.open(directory / "wavegraph.png") as source:
    rgba = source.convert("RGBA")
image = rgba.crop(rgba.getchannel("A").getbbox()).resize((208, 36), Image.Resampling.LANCZOS)
mask = bytearray()
for y in range(image.height):
    for left in range(0, image.width, 4):
        packed = 0
        for x in range(left, left + 4):
            _, green, _, alpha = image.getpixel((x, y))
            # Preserve both the dark letter fill and the luminous rim.
            ink = alpha * (128 + green) // 383
            level = (ink >= 32) + (ink >= 92) + (ink >= 140)
            packed = (packed << 2) | level
        mask.append(packed)
(directory / "wordmark.2bpp").write_bytes(mask)
