#!/usr/bin/env python3
"""Find doc comments that got separated from the item they document.

    tools/check-docs.py

EIGHT TIMES, AND EVERY ONE THE SAME MOVE. Inserting a function directly above an
existing one puts the new function — and its own doc comment — BETWEEN the old
doc comment and the item it belonged to. The old item is left undocumented and
its note reads as the new function's opening paragraph. Nothing catches it:
`rustc` and `javac` are both happy, every test passes, and the only symptom is a
comment that describes the wrong thing to whoever reads it next.

The eight, all found after the fact:

  * `preset_loop_sub`        — stolen by `come_forward_sub`
  * `android_main`           — stolen by `log_note`
  * `LEVEL_WATCH_MS`         — a stale doc for the constant it replaced
  * `pump_rds_until_settled` — its note stranded on `morph_frames_for_bench`
  * `push_clock`             — stranded on `clock_reading`
  * `close_logo_search`      — stranded on `close_dark_pick`
  * `ingest_position`        — stranded on the motion constants
  * `clockHourMinute`        — stolen by `processAgeSeconds`

TWO SHAPES, AND BOTH ARE HERE.

  ADJACENT BLOCKS (Java). Two `/** ... */` comments with nothing between them.
  Javadoc binds only the last one, so the first is dead. This is exact: there is
  no legitimate reason to write two in a row, so every hit is a defect.

  MERGED BLOCKS (Rust). One `///` run holding what were two comments, because
  the inserted item's doc was appended to the previous one with no blank `///`
  between. This is a HEURISTIC — prose can legitimately look like it — so hits
  are printed for a human to judge and the two known-good ones are listed below
  rather than being silently skipped.

Exit status is 1 only for the exact shape. The heuristic reports and never fails
a build, because a check that cries wolf gets switched off.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Ordinary prose that the merged-block heuristic flags and should not. Measured,
# not guessed: each was read and found to be one paragraph continuing, not two
# comments joined. Keep this list SHORT — a long one means the heuristic is wrong
# and wants narrowing, not more exceptions.
KNOWN_PROSE = {
    ("src/clock.rs", "A test walks all sixty."),
    ("src/settings.rs", "So it is offered, and it is not the default."),
}


def java_adjacent_blocks():
    """Two Javadoc comments with no item between them. Exact; every hit is real."""
    hits = []
    for path in sorted(ROOT.glob("**/*.java")):
        if "/build/" in str(path):
            continue
        rel = path.relative_to(ROOT).as_posix()
        lines = path.read_text(encoding="utf-8").split("\n")
        closed_at = None
        for i, raw in enumerate(lines):
            line = raw.strip()
            if line.endswith("*/"):
                closed_at = i
                continue
            if not line:
                continue
            if line.startswith("/**") and closed_at is not None:
                hits.append((rel, i + 1, lines[closed_at].strip(), line))
            # Any other non-blank line is an item, an annotation or code, and
            # ends the window in which a second comment would be suspicious.
            closed_at = closed_at if line.startswith("*") or line.endswith("*/") else None
    return hits


def rust_merged_blocks():
    """One `///` run that reads like two comments joined. Heuristic; for review."""
    hits = []
    for path in sorted(ROOT.glob("src/**/*.rs")):
        rel = path.relative_to(ROOT).as_posix()
        lines = path.read_text(encoding="utf-8").split("\n")
        i = 0
        while i < len(lines):
            if not lines[i].lstrip().startswith("///"):
                i += 1
                continue
            j = i
            while j < len(lines) and lines[j].lstrip().startswith("///"):
                j += 1
            block = [l.lstrip()[3:].strip() for l in lines[i:j]]
            for k in range(1, len(block) - 1):
                prev, cur = block[k - 1], block[k]
                if not prev or not cur:
                    continue
                # A summary line looks like: short, capitalised, ends a sentence,
                # and follows one that also ended a sentence with no blank
                # `///` separating them.
                if (
                    prev.endswith(".")
                    and cur[:1].isupper()
                    and cur.endswith(".")
                    and len(cur.split()) <= 12
                    and (rel, cur) not in KNOWN_PROSE
                ):
                    hits.append((rel, i + k + 1, prev, cur))
            i = j
    return hits


def main() -> int:
    exact = java_adjacent_blocks()
    maybe = rust_merged_blocks()

    if exact:
        print(f"ADJACENT JAVADOC BLOCKS — {len(exact)}, each one a stranded comment:")
        for rel, line, prev, cur in exact:
            print(f"  {rel}:{line}")
            print(f"      closes: {prev}")
            print(f"      opens : {cur}")
    else:
        print("adjacent javadoc blocks: none")

    if maybe:
        print(f"\nPOSSIBLE MERGED RUST DOC BLOCKS — {len(maybe)}, for a human to judge:")
        for rel, line, prev, cur in maybe:
            print(f"  {rel}:{line}")
            print(f"      after: {prev}")
            print(f"      here : {cur}")
        print("\n  Ordinary prose can look like this. If one of these IS prose,")
        print("  add it to KNOWN_PROSE with the reading that cleared it.")
    else:
        print("\npossible merged rust doc blocks: none")

    return 1 if exact else 0


if __name__ == "__main__":
    raise SystemExit(main())
