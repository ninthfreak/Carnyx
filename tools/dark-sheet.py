#!/usr/bin/env python3
"""Lay the dark-logo pipeline's output out on the dark card, for looking at.

The host has no image codec, so the pipeline's own tests are about numbers. This
is the other half: it decodes real logo files with Pillow, runs every one through
`examples/darkprobe.rs` (the shipping pipeline, unmodified), and draws a contact
sheet — the original on white, then each treatment as the face would draw it,
with the pipeline's pick outlined.

    cargo build --release --example darkprobe
    python3 tools/dark-sheet.py OUT.png LOGO [LOGO ...] [--bg RRGGBB] [--zoom N]
                                [--dump DIR]

`--dump DIR` also writes every treatment at its own size on the card, as
`DIR/<logo>.<TREATMENT>.png`, for looking at single pixels.

Needs Pillow. Logos are third-party artwork: keep them out of the repository.
"""
import argparse
import os
import struct
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROBE = os.path.join(ROOT, "target", "release", "examples", "darkprobe")
ORDER = ["REMAP", "HALO", "AS_IS", "PLATE"]
CARD = (0x33, 0x37, 0x3D)      # Pal.panel, dark — `logos::dark::LOGO_DARK_BG`
SLAB = (0xE6, 0xE6, 0xE6)      # Pal.logo-plate
CELL_W, CELL_H, PAD, LABEL_H = 300, 180, 12, 34
MAX_EDGE = 1024                # `logos::prep::DECODE_MAX_EDGE`


def font(size):
    for name in ("DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            pass
    return ImageFont.load_default()


def to_rgba_file(im, path):
    with open(path, "wb") as f:
        f.write(struct.pack(">II", im.width, im.height))
        f.write(im.tobytes())


def from_rgba_file(path):
    with open(path, "rb") as f:
        w, h = struct.unpack(">II", f.read(8))
        return Image.frombytes("RGBA", (w, h), f.read())


def decode(path):
    im = Image.open(path).convert("RGBA")
    longest = max(im.size)
    if longest > MAX_EDGE:
        s = MAX_EDGE / longest
        im = im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.BOX)
    return im


def fit(im, w, h):
    s = min(w / im.width, h / im.height)
    size = (max(1, round(im.width * s)), max(1, round(im.height * s)))
    return im.resize(size, Image.LANCZOS if s < 1 else Image.NEAREST)


def cell(im, ground, slab=False, picked=False):
    """One treatment as the face draws it: contained, centred, on its ground."""
    out = Image.new("RGB", (CELL_W, CELL_H), ground)
    box_w, box_h = CELL_W - 2 * PAD, CELL_H - 2 * PAD
    if slab:
        # `plate_params()`: 9% padding, 14% radius, behind the keyed mark.
        inner = fit(im, box_w * 0.82, box_h * 0.82)
        pad = round(max(inner.size) * 0.09)
        sw, sh = inner.width + 2 * pad, inner.height + 2 * pad
        x, y = (CELL_W - sw) // 2, (CELL_H - sh) // 2
        ImageDraw.Draw(out).rounded_rectangle(
            [x, y, x + sw, y + sh], radius=round(min(sw, sh) * 0.14), fill=SLAB)
        out.paste(inner, (x + pad, y + pad), inner)
    else:
        inner = fit(im, box_w, box_h)
        out.paste(inner, ((CELL_W - inner.width) // 2, (CELL_H - inner.height) // 2), inner)
    if picked:
        ImageDraw.Draw(out).rectangle([0, 0, CELL_W - 1, CELL_H - 1], outline=(0x4D, 0xA3, 0xFF), width=4)
    return out


def run(path, bg, tmp):
    src = decode(path)
    raw = os.path.join(tmp, "in.rgba")
    to_rgba_file(src, raw)
    prefix = os.path.join(tmp, "out")
    for t in ORDER:
        try:
            os.remove(f"{prefix}.{t}.rgba")
        except FileNotFoundError:
            pass
    text = subprocess.run([PROBE, raw, prefix, bg], check=True, capture_output=True, text=True).stdout
    facts = {"gates": {}}
    for line in text.splitlines():
        k, _, v = line.partition(" ")
        if k == "gate":
            t, _, note = v.partition(" ")
            facts["gates"][t] = note
        else:
            facts[k] = v
    cands = {t: from_rgba_file(f"{prefix}.{t}.rgba") for t in ORDER if os.path.exists(f"{prefix}.{t}.rgba")}
    return src, cands, facts


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("logos", nargs="+")
    ap.add_argument("--bg", default="33373D")
    ap.add_argument("--zoom", type=int, default=1, help="scale the finished sheet up, nearest-neighbour")
    ap.add_argument("--dump", help="also write each treatment at native size on the card, into this folder")
    a = ap.parse_args()
    if not os.path.exists(PROBE):
        sys.exit("build it first: cargo build --release --example darkprobe")
    card = tuple(int(a.bg[i:i + 2], 16) for i in (0, 2, 4))

    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        for path in a.logos:
            src, cands, facts = run(path, a.bg, tmp)
            rows.append((os.path.basename(path), src, cands, facts))
            if a.dump:
                os.makedirs(a.dump, exist_ok=True)
                stem = os.path.splitext(os.path.basename(path))[0]
                for t, im in cands.items():
                    flat = Image.new("RGB", im.size, SLAB if t == "PLATE" else card)
                    flat.paste(im, (0, 0), im)
                    flat.save(os.path.join(a.dump, f"{stem}.{t}.png"))

    cols = 1 + len(ORDER)
    W = cols * CELL_W + (cols + 1) * PAD
    H = len(rows) * (CELL_H + LABEL_H + PAD) + PAD + 30
    sheet = Image.new("RGB", (W, H), (0x1E, 0x20, 0x24))
    d = ImageDraw.Draw(sheet)
    f, small = font(13), font(11)
    for c, title in enumerate(["ORIGINAL (on white)"] + ORDER):
        d.text((PAD + c * (CELL_W + PAD), 8), title, fill=(200, 200, 200), font=f)
    y = 30
    for name, src, cands, facts in rows:
        d.text((PAD, y), f"{name}  {src.width}x{src.height}  pick {facts.get('pick')}  "
               f"coverage {facts.get('coverage')}  {facts.get('ms')} ms  [{facts.get('key')}]",
               fill=(230, 230, 230), font=f)
        y += 18
        sheet.paste(cell(src, (255, 255, 255)), (PAD, y))
        for c, t in enumerate(ORDER, start=1):
            x = PAD + c * (CELL_W + PAD)
            if t in cands:
                sheet.paste(cell(cands[t], card, slab=(t == "PLATE"), picked=(facts.get("pick") == t)), (x, y))
                d.text((x + 4, y + CELL_H + 1), facts["gates"].get(t, ""), fill=(150, 150, 150), font=small)
            else:
                d.rectangle([x, y, x + CELL_W - 1, y + CELL_H - 1], outline=(70, 70, 70))
                d.text((x + 8, y + 8), "not routed", fill=(110, 110, 110), font=small)
        y += CELL_H + LABEL_H - 18 + PAD
    if a.zoom > 1:
        sheet = sheet.resize((sheet.width * a.zoom, sheet.height * a.zoom), Image.NEAREST)
    sheet.save(a.out)
    for name, _, _, facts in rows:
        print(f"{name}: pick {facts.get('pick')}  coverage {facts.get('coverage')}  {facts['gates']}")


if __name__ == "__main__":
    main()
