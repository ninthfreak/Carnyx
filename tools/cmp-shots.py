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

WHICH ONES, MEASURED RATHER THAN GUESSED. Two renders of one unchanged binary,
back to back, disagreed on SEVENTEEN of the hundred — and a reviewer who does not
know that reads a clean refactor as having repainted a fifth of the app. The list
below is that measurement, and `--unstable` passes it for you:

    tools/cmp-shots.py OLD_DIR NEW_DIR --unstable

The rest are stable to the byte and are where a real change shows up. If a shot
here ever settles down, take it off the list rather than leaving a blind spot; if
a new one starts flapping, measure it the same way — render twice with no source
change and compare the two.
"""
import os
import sys

from PIL import Image

# The shots that differ run to run against an UNCHANGED binary — measured, see
# the module note. Grouped by what moves under them.
UNSTABLE = [
    # The power ring and the band art's own animation.
    "acdc.png", "acdc-dark.png", "acdc-portrait.png", "audio-released.png",
    "driving.png",
    # INTERMITTENT, and it took three full runs to catch. A band theme draws
    # `GlitchText`, whose animation is on the same wall clock as everything else
    # here — so this shot agrees with itself across most pairs of runs and
    # disagrees across some. A shot that flaps rarely is worse than one that
    # flaps always: it reads as a regression in whichever change happens to be
    # in flight. Two full runs are not enough to clear one; the mechanism is.
    "nin.png",
    # Spinners.
    "logo-search-loading.png", "nearby-loading.png",
    # The RadioText strip mid-scroll.
    "long-radiotext.png",
    # The hero morph mid-travel.
    "hero-step-morph.png",
    # The maneuver layer's hairline, which fills on the wall clock.
    "nav-approach.png", "nav-approach-portrait.png", "nav-cruise.png",
    "nav-poll-only.png", "nav-turn-now.png", "nav-turn-now-portrait.png",
    # The diagnostics log stamps its own lines with the time it ran.
    "settings-diagnostics-full.png", "settings-diagnostics-portrait.png",
    "settings-diagnostics-rows.png",
]


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__.strip())
        return 2
    base, new = sys.argv[1], sys.argv[2]
    rest = sys.argv[3:]
    ignore = set(UNSTABLE) if "--unstable" in rest else set()
    ignore.update(a for a in rest if a != "--unstable")
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
