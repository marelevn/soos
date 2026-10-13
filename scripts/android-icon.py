#!/usr/bin/env python3
"""Makes the Android launcher icon's foreground, one PNG per density, from a
PNG of the logo: `assets/icons/hicolor/256x256.png`, or a bigger one if you
have it, which only makes the edges cleaner.

An adaptive icon is two layers that the launcher masks into its own shape
(a circle, a squircle...), so the logo's rounded square can't be used as it
is. The glyphs (white `8`, light grey `s`) are the logo's only colours that
the orange background doesn't have: the background's blue channel is at most
about 50 and the glyphs' is 224 or more, so that channel alone separates the
two. The glyphs become the foreground, with the soft edge they had; the
background is `res/drawable/ic_launcher_background.xml`, the logo's
gradient. The same foreground is the monochrome layer of Android 13's
themed icons.

The glyphs are scaled to fit the circle every launcher keeps visible (66 of
the layer's 108 dp, with a little room), not to fill the layer.

Needs Pillow:
    python3 scripts/android-icon.py assets/icons/hicolor/256x256.png
"""

import sys
from pathlib import Path

from PIL import Image

RES = Path(__file__).resolve().parent.parent / "android" / "app" / "src" / "main" / "res"
DENSITIES = {"mdpi": 1.0, "hdpi": 1.5, "xhdpi": 2.0, "xxhdpi": 3.0, "xxxhdpi": 4.0}
LAYER_DP = 108
# The glyphs' farthest corner from their centre, in dp: inside the 33 dp
# radius of the safe circle, with a margin for the launcher's own scaling.
RADIUS_DP = 30
# Where the background's blue channel ends and the glyphs' begins.
BACKGROUND_BLUE, GLYPH_BLUE = 50, 224


def glyphs(logo):
    """The logo's glyphs as grey on transparent, cropped to their edges."""
    logo = logo.convert("RGBA")
    out = Image.new("RGBA", logo.size)
    pixels = []
    for r, g, b, a in logo.getdata():
        cover = (b - BACKGROUND_BLUE) / (GLYPH_BLUE - BACKGROUND_BLUE)
        alpha = a * max(0.0, min(1.0, cover)) / 255
        # The `s` are 230 grey and the `8` white; an edge pixel is partly
        # orange, so it takes the nearest of the two.
        grey = min(255, max(b, 230))
        pixels.append((grey, grey, grey, round(alpha * 255)))
    out.putdata(pixels)
    return out.crop(out.getchannel("A").point(lambda v: 255 if v > 127 else 0).getbbox())


def smoothstep(alpha):
    """Sharpens an upscaled edge back toward the crispness it had."""
    t = max(0.0, min(1.0, (alpha / 255 - 0.25) / 0.5))
    return round(255 * t * t * (3 - 2 * t))


def main(path):
    art = glyphs(Image.open(path))
    width, height = art.size
    radius = (width**2 + height**2) ** 0.5 / 2
    for name, density in DENSITIES.items():
        layer = round(LAYER_DP * density)
        scale = RADIUS_DP * density / radius
        size = (round(width * scale), round(height * scale))
        scaled = art.resize(size, Image.LANCZOS)
        scaled.putalpha(scaled.getchannel("A").point(smoothstep))
        canvas = Image.new("RGBA", (layer, layer))
        canvas.paste(scaled, ((layer - size[0]) // 2, (layer - size[1]) // 2))
        folder = RES / f"mipmap-{name}"
        folder.mkdir(parents=True, exist_ok=True)
        canvas.save(folder / "ic_launcher_foreground.png", optimize=True)
        print(folder / "ic_launcher_foreground.png", canvas.size)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
