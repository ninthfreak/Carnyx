#!/usr/bin/env python3
"""Render AC/DC's horns from CarFM's SVG into the PNGs `ui/hero.slint` draws.

WHY BAKE RATHER THAN DRAW. Each horn is sixty round-capped strokes of steadily
tapering width under a Gaussian drop-shadow. Slint can draw all of that — the
signal meter next door is `Path` elements with the same SVG command strings —
but it would be a hundred and twenty `Path` items across the pair, on a unit
measured at 131 ms per frame. Nothing about the art changes at run time, so it
is a picture.

THE SOURCE IS CarFM's `bandArt.tsx`, read directly rather than copied, so the
two cannot drift. Re-run this if that file's HORN_LEFT/HORN_RIGHT ever change:

    python3 tools/bake-acdc-horns.py [path/to/bandArt.tsx]

Output: ui/art/acdc-horn-{left,right}.png at 4x the SVG's 70x96, which is enough
for the largest the card ever draws them.
"""
import math
import re
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

DEFAULT_SRC = "/home/user/VibeSDR-CarFM/src/components/carfm/bandArt.tsx"
OUT = Path(__file__).resolve().parent.parent / "ui" / "art"

# Every horn segment is a two-point line with its own width — no curves.
SEG = re.compile(r'<path d="M([\d.]+) ([\d.]+) L([\d.]+) ([\d.]+)" stroke-width="([\d.]+)"')

# From the SVG: stroke #E31E24, and a feDropShadow of rgba(255,59,48,0.7) at
# stdDeviation 2.5. Carried across as numbers rather than eyeballed.
INK = (227, 30, 36, 255)
GLOW = (255, 59, 48, 179)
BLUR = 2.5
VB_W, VB_H = 70, 96


def bake(svg: str, out: Path, scale: int = 4) -> int:
    rot = re.search(r'transform="rotate\((-?[\d.]+) ([\d.]+) ([\d.]+)\)"', svg)
    ang, cx, cy = (
        (float(rot.group(1)), float(rot.group(2)), float(rot.group(3))) if rot else (0.0, 35.0, 48.0)
    )
    a = math.radians(ang)

    def xf(x, y):
        dx, dy = x - cx, y - cy
        return (cx + dx * math.cos(a) - dy * math.sin(a), cy + dx * math.sin(a) + dy * math.cos(a))

    w, h = VB_W * scale, VB_H * scale
    glow = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    core = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    dg, dc = ImageDraw.Draw(glow), ImageDraw.Draw(core)

    segs = SEG.findall(svg)
    for x1, y1, x2, y2, sw in segs:
        p1, p2 = xf(float(x1), float(y1)), xf(float(x2), float(y2))
        a1 = (p1[0] * scale, p1[1] * scale)
        a2 = (p2[0] * scale, p2[1] * scale)
        lw = max(1, round(float(sw) * scale))
        for draw, col in ((dg, GLOW), (dc, INK)):
            draw.line([a1, a2], fill=col, width=lw)
            # stroke-linecap="round". PIL's line has butt caps, and without these
            # the joins between sixty tapering segments show as a row of notches.
            r = lw / 2
            for p in (a1, a2):
                draw.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=col)

    glow = glow.filter(ImageFilter.GaussianBlur(BLUR * scale))
    Image.alpha_composite(glow, core).save(out)
    return len(segs)


def main() -> None:
    src = Path(sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SRC)
    if not src.exists():
        sys.exit(f"cannot read {src} — pass the path to CarFM's bandArt.tsx")
    text = src.read_text()
    OUT.mkdir(parents=True, exist_ok=True)
    for name, fn in (("HORN_LEFT", "acdc-horn-left.png"), ("HORN_RIGHT", "acdc-horn-right.png")):
        m = re.search(r"export const %s = `(.*?)`;" % name, text, re.S)
        if not m:
            sys.exit(f"{name} not found in {src}")
        n = bake(m.group(1), OUT / fn)
        print(f"{fn}: {n} segments")


if __name__ == "__main__":
    main()
