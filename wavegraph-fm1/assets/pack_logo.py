"""Pack the LCD title and model lettering. Only this tool uses Pillow."""
from pathlib import Path

from PIL import Image

directory = Path(__file__).resolve().parent


def pack(source, name, size, level):
    with Image.open(directory / source) as original:
        rgba = original.convert("RGBA")
    image = rgba.crop(rgba.getchannel("A").getbbox()).resize(size, Image.Resampling.LANCZOS)
    mask = bytearray()
    for y in range(image.height):
        for left in range(0, image.width, 4):
            packed = 0
            for x in range(left, left + 4):
                packed = (packed << 2) | level(*image.getpixel((x, y)))
            mask.append(packed)
    (directory / name).write_bytes(mask)


def green_level(red, green, blue, alpha):
    # Preserve both the dark letter fill and the luminous rim.
    ink = alpha * (128 + green) // 383
    return (ink >= 32) + (ink >= 92) + (ink >= 140)


def pink_level(red, green, blue, alpha):
    # Magenta covers the opaque strokes. Pale pink follows the brush highlights.
    if alpha < 40:
        return 0
    if alpha < 152:
        return 1
    return 3 if green >= 77 else 2


pack("wavegraph.png", "wordmark.2bpp", (208, 36), green_level)
pack("fm1.png", "fm1.2bpp", (104, 38), pink_level)
