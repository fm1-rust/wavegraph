"""Generate packed coverage masks with Pillow, outside the firmware build.

Pass 'Orbitron Bold.ttf' from theleagueof/orbitron at
13e6a5222aa6818d81c9acd27edd701a2d744152. Run in this directory.
"""
from pathlib import Path
import sys

from PIL import Image, ImageDraw, ImageFont

font = ImageFont.truetype(sys.argv[1], 96)
cap = -font.getbbox("H", anchor="ls")[1]


def raster(text, width, height, cap_height, split=None):
    left, _, right, _ = font.getbbox(text, anchor="ls")
    canvas = Image.new("L", (max(1, right - left), cap + 32))
    ImageDraw.Draw(canvas).text((-left, cap), text, font=font, anchor="ls", fill=255)
    w = max(1, min(width, round(canvas.width * cap_height / cap)))
    h = round(canvas.height * cap_height / cap)
    canvas = canvas.resize((w, h), Image.Resampling.LANCZOS)
    tile = Image.new("L", (width, height))
    x = (width - w) // 2
    tile.paste(canvas, (x, (height - cap_height) // 2))
    if split:
        boundary = x + round((font.getlength(text[:split]) - left) * w / max(1, right - left))
        print(text, "color boundary", boundary)
    return tile


def pack(image):
    levels = [(value * 3 + 127) // 255 for value in image.tobytes()]
    levels += [0] * (-len(levels) % 4)
    return bytes(a << 6 | b << 4 | c << 2 | d for a,b,c,d in zip(*[iter(levels)]*4))


for name, w, h, caps in [("legend",6,10,7), ("heading",7,13,9), ("grid",12,16,11)]:
    Path(name + ".raw").write_bytes(b"".join(
        pack(raster(chr(c), w - 1, h, caps).crop((0, 0, w, h))) for c in range(32,127)
    ))

for text, w, h, caps, split in [
    ("TB-303",78,20,15,3), ("TR-808",100,32,19,3),
    ("TR-909",112,32,22,3), ("JP-8080",100,16,14,None)
]:
    Path(text + ".raw").write_bytes(pack(raster(text,w,h,caps,split)))

Path("badge.raw").write_bytes(b"".join(pack(raster(c,24,32,23)) for c in "1234"))
Path("Bass-Line.raw").write_bytes(pack(raster("Bass Line", 90, 18, 14)))
