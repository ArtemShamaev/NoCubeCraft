#!/usr/bin/env python3
"""Turns ambientCG texture sets (CC0, https://ambientcg.com) into the terrain textures in
assets/textures/terrain: 512 px colour, OpenGL normal, and roughness + occlusion packed into
red and green. Usage: prepare_textures.py <folder with unpacked *_1K-JPG sets>"""

import pathlib
import sys

from PIL import Image

SIZE = 512
# Game material -> ambientCG set. The order is the texture array layer order.
LAYERS = [
    ("grass", "Grass004"),
    ("dirt", "Ground103"),
    ("stone", "Rock030"),
    ("sand", "Ground080"),
    ("planks", "Planks037A"),
    ("bark", "Bark014"),
    ("leaves", "Moss002"),
    ("snow", "Snow010A"),
    ("bricks", "Bricks113A"),
]


def load(folder: pathlib.Path, name: str, suffix: str, mode: str, default):
    matches = list(folder.glob(f"{name}*/*_{suffix}.jpg"))
    if not matches:
        return Image.new(mode, (SIZE, SIZE), default)
    return Image.open(matches[0]).convert(mode).resize((SIZE, SIZE), Image.LANCZOS)


def main() -> None:
    src = pathlib.Path(sys.argv[1])
    out = pathlib.Path(__file__).resolve().parent.parent / "assets" / "textures" / "terrain"
    out.mkdir(parents=True, exist_ok=True)
    for material, name in LAYERS:
        color = load(src, name, "Color", "RGB", (128, 128, 128))
        normal = load(src, name, "NormalGL", "RGB", (128, 128, 255))
        rough = load(src, name, "Roughness", "L", 200)
        ao = load(src, name, "AmbientOcclusion", "L", 255)
        packed = Image.merge("RGB", (rough, ao, Image.new("L", (SIZE, SIZE), 0)))
        color.save(out / f"{material}_color.jpg", quality=90)
        normal.save(out / f"{material}_normal.jpg", quality=95)
        packed.save(out / f"{material}_rough_ao.jpg", quality=90)
        # Average colour in linear space, for things drawn without the texture (debris, icons).
        data = color.resize((32, 32)).tobytes()
        lin = [sum((data[i] / 255) ** 2.2 for i in range(k, len(data), 3)) / (len(data) // 3) for k in range(3)]
        print(f"{material}: linear average {lin[0]:.3f} {lin[1]:.3f} {lin[2]:.3f}")


if __name__ == "__main__":
    main()
