#!/usr/bin/env python3
"""Makes `assets/icons/soos.icns` and `soos.ico` from a 1024 px PNG of
`assets/logo.svg`.

The Mac icon follows Apple's macOS 11+ grid: an 824 px body centred on a
1024 px canvas, with the template's 185 px corner and a soft shadow in the
margin. `logo.svg` fills its canvas edge to edge, which looks oversized
in the Dock. Windows has no such grid, so the `.ico` keeps the full-bleed
logo, at the sizes Explorer and the taskbar ask for.

Needs Pillow and `iconutil` (macOS):
    python3 scripts/macos-icon.py logo-1024.png
"""

import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ICONS = Path(__file__).resolve().parent.parent / "assets" / "icons"
CANVAS, BODY = 1024, 824
RADIUS = 185


def rounded_mask(size):
    """An antialiased rounded square, drawn at 4x and scaled down."""
    mask = Image.new("L", (size * 4, size * 4))
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size * 4 - 1, size * 4 - 1), RADIUS * 4, 255)
    return mask.resize((size, size), Image.LANCZOS)


def mac_icon(logo):
    # The body is the logo's opaque part; a few pixels in, past its
    # antialiased edge, since the new corner cuts deeper anyway.
    left, top, right, bottom = logo.getchannel("A").point(lambda a: 255 if a >= 128 else 0).getbbox()
    inset = 3
    body = logo.crop((left + inset, top + inset, right - inset, bottom - inset))
    body = body.resize((BODY, BODY), Image.LANCZOS)
    body.putalpha(rounded_mask(BODY))

    margin = (CANVAS - BODY) // 2
    shadow = Image.new("RGBA", (CANVAS, CANVAS))
    tint = Image.new("RGBA", (BODY, BODY), (0, 0, 0, 77))
    shadow.paste(tint, (margin, margin + 10), body.getchannel("A"))
    icon = shadow.filter(ImageFilter.GaussianBlur(10))
    icon.alpha_composite(body, (margin, margin))
    return icon


def main():
    logo = Image.open(sys.argv[1]).convert("RGBA")
    if logo.size != (CANVAS, CANVAS):
        sys.exit(f"expected a {CANVAS}x{CANVAS} PNG, got {logo.size}")

    icon = mac_icon(logo)
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "soos.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale, suffix in ((1, ""), (2, "@2x")):
                px = size * scale
                icon.resize((px, px), Image.LANCZOS).save(iconset / f"icon_{size}x{size}{suffix}.png")
        subprocess.run(["iconutil", "-c", "icns", "-o", ICONS / "soos.icns", iconset], check=True)

    # 24 is the taskbar at 150%; 64 is Explorer's large icons.
    sizes = [(s, s) for s in (16, 24, 32, 48, 64, 256)]
    logo.save(ICONS / "soos.ico", sizes=sizes)


if __name__ == "__main__":
    main()
