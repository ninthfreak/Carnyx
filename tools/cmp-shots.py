#!/usr/bin/env python3
"""Exact pixel comparison of two shot directories.

    tools/cmp-shots.py OLD_DIR NEW_DIR [ignored.png ...]

COMPARES RAW RGBA BYTES, and the reason is worth the line it costs: for an RGBA
image `PIL`'s `Image.getbbox()` — including the one on an `ImageChops.difference`
result — looks at the ALPHA CHANNEL ALONE. Two opaque frames that differ in every
colour channel report `None` and read as identical. That mistake once reported
63/63 shots unchanged across a change that repainted an entire card.

Some shots differ run to run against an unchanged binary, because Slint's
animations run on a wall clock shared by the whole process: the power ring, the
logo-search spinner, the morph mid-travel and the diagnostics log's own
timestamps. Name those on the command line; the report keeps them separate rather
than hiding them.
"""
import os
import sys

from PIL import Image


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__.strip())
        return 2
    base, new = sys.argv[1], sys.argv[2]
    ignore = set(sys.argv[3:])
    same = 0
    ignored, diff = [], []
    for f in sorted(os.listdir(new)):
        if not f.endswith(".png"):
            continue
        b = os.path.join(base, f)
        if not os.path.exists(b):
            print(f"ONLY IN NEW: {f}")
            continue
        a = Image.open(b).convert("RGBA")
        c = Image.open(os.path.join(new, f)).convert("RGBA")
        if a.size != c.size:
            diff.append(f"{f}: {a.size} -> {c.size}")
            continue
        pa, pc = a.tobytes(), c.tobytes()
        if pa == pc:
            same += 1
            continue
        n = sum(1 for i in range(0, len(pa), 4) if pa[i:i + 4] != pc[i:i + 4])
        mx = max(abs(x - y) for x, y in zip(pa, pc))
        (ignored if f in ignore else diff).append(f"{f}: {n} px, max delta {mx}")
    print(f"identical: {same}   known-unstable: {len(ignored)}   DIFFERING: {len(diff)}")
    for line in ignored:
        print(f"  (unstable) {line}")
    for line in diff:
        print(f"  {line}")
    return 1 if diff else 0


if __name__ == "__main__":
    raise SystemExit(main())
