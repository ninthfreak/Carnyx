#!/usr/bin/env python3
"""Render the launcher icon ladder from `docs/design/carnyx-icon.svg`.

    python3 tools/gen-launcher-icons.py [--store]

Writes `android/app/src/main/res/mipmap-*/ic_launcher.png` at the five densities
Android asks for, and with `--store` a 512px `docs/design/carnyx-icon-512.png`
for a listing page.

── WHY THE PNGs ARE COMMITTED ───────────────────────────────────────────────

Gradle packages what is in `res/`; it does not run this script. So the bitmaps
are checked in and this exists to REGENERATE them when the SVG changes — the
same arrangement as `tools/bake-band-art.py`, and for the same reason: the art
is fixed at build time and nothing about it is decided at run time.

── THE ONE DEPENDENCY, AND WHY IT IS NOT IN THE TREE ────────────────────────

`cairosvg`, and only here. Every other tool in this directory draws with `PIL`
from numbers it parses itself, because their sources are a handful of straight
strokes. This source is not: it has elliptical arcs, a clip-path and stroked
subpaths, and reimplementing an SVG rasteriser to avoid `pip install cairosvg`
would be a worse trade than the dependency. Nothing at build time or run time
needs it — only a person changing the icon.

    pip install cairosvg

── THE METADATA IS STRIPPED FIRST ───────────────────────────────────────────

The SVG carries a C2PA content-credentials manifest in `<metadata>`: 12,100
bytes of file, of which 4,353 is the drawing. It is inert, but it is two thirds
of the source and has no business inside an APK, so it comes out before the
render rather than being shipped in five resolutions.

── THE DENSITIES, AND WHICH ONE THE UNIT ACTUALLY USES ──────────────────────

mdpi 48, hdpi 72, xhdpi 96, xxhdpi 144, xxxhdpi 192 — Android's own ladder.
The head unit is 1280x720 and this codebase works throughout at density 1
(see the measurement table under #127 in docs/TASKS.md), which puts it in the
mdpi bucket: on that launcher the icon most people will ever see is the 48.
That has not been confirmed against the device's own `DisplayMetrics`, so treat
it as the likely case rather than a fact — but generate all five regardless,
because the newer head unit this app has to keep working on is not this one.
"""
import re
import sys
from pathlib import Path

SRC = Path("docs/design/carnyx-icon.svg")
RES = Path("android/app/src/main/res")
STORE = Path("docs/design/carnyx-icon-512.png")

# Android's launcher ladder: directory suffix -> pixels.
DENSITIES = {
    "mdpi": 48,
    "hdpi": 72,
    "xhdpi": 96,
    "xxhdpi": 144,
    "xxxhdpi": 192,
}


def main() -> int:
    try:
        import cairosvg
    except ImportError:
        print("this script needs cairosvg: pip install cairosvg", file=sys.stderr)
        return 2

    root = Path(__file__).resolve().parent.parent
    src = root / SRC
    if not src.exists():
        print(f"no icon at {SRC}", file=sys.stderr)
        return 2

    # The manifest is a single <metadata> element; nothing else in the file uses
    # that tag, so this cannot take a piece of the drawing with it.
    svg = re.sub(r"<metadata>.*?</metadata>", "", src.read_text(), flags=re.S)
    kept = len(svg)

    for suffix, px in DENSITIES.items():
        out = root / RES / f"mipmap-{suffix}" / "ic_launcher.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        cairosvg.svg2png(
            bytestring=svg.encode(),
            write_to=str(out),
            output_width=px,
            output_height=px,
        )
        print(f"{out.relative_to(root)}  {px}x{px}")

    if "--store" in sys.argv[1:]:
        out = root / STORE
        cairosvg.svg2png(
            bytestring=svg.encode(), write_to=str(out), output_width=512, output_height=512
        )
        print(f"{out.relative_to(root)}  512x512")

    print(f"source {src.stat().st_size} bytes, {kept} after the manifest came out")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
