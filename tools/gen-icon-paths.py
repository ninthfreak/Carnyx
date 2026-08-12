"""Precompute the icon path data that CarFM builds at runtime in JS.

Slint's Path takes SVG command strings but has no loops, no trig and no
stroke-dasharray, so three things are baked out here instead of being written by
hand: the signal glyph's dotted (lossy) arcs, the nearby magnifier's barrel-warped
tower, and the GPS satellite's -28 degree rotation.

Everything is derived from the SAME numbers as the React source
(src/components/carfm/icons.tsx) so the output is the identical geometry.
"""
import math

# ── Signal glyph (icons.tsx PAIRS) ──────────────────────────────────────────
PAIRS = [
    dict(rx=6.5,   ry=5.5,   xL=23.8,  xR=34.2,  y0=11.6, y1=20.4),
    dict(rx=11.31, ry=9.57,  xL=19.25, xR=38.75, y0=8.3,  y1=23.7),
    dict(rx=16.25, ry=13.75, xL=14.7,  xR=43.3,  y0=5,    y1=27),
    dict(rx=21.19, ry=17.93, xL=10.15, xR=47.85, y0=1.7,  y1=30.3),
]
CY = 16.0
DOT_PERIOD = 4.8      # icons.tsx DOT_DASH = [0.01, 4.79]
DOT_R = 1.1           # round cap of a 2.2 stroke


def arc_centre(rx, ry, x_end, y0, side):
    """Centre of the ellipse carrying one arc of a pair.

    Both endpoints share an x and straddle y=16, so the centre sits on y=16 and
    is offset horizontally by the amount that puts the endpoint on the ellipse.
    The left arc bows left, so its centre is to the RIGHT of it, and vice versa.
    """
    t = (y0 - CY) / ry
    dx = rx * math.sqrt(max(0.0, 1.0 - t * t))
    return (x_end + dx) if side == 'L' else (x_end - dx)


def arc_angles(rx, ry, cx, x_end, y0, y1, side):
    a0 = math.atan2((y0 - CY) / ry, (x_end - cx) / rx)
    a1 = math.atan2((y1 - CY) / ry, (x_end - cx) / rx)
    # sweep 0 (left arc) runs in the negative direction; sweep 1 (right) positive.
    if side == 'L':
        while a1 > a0:
            a1 -= 2 * math.pi
    else:
        while a1 < a0:
            a1 += 2 * math.pi
    return a0, a1


def sample(rx, ry, cx, a):
    return (cx + rx * math.cos(a), CY + ry * math.sin(a))


def dots_for_arc(rx, ry, cx, a0, a1, n=800):
    """Dot centres every DOT_PERIOD of arc length, starting at the path start."""
    pts, lengths = [], [0.0]
    prev = sample(rx, ry, cx, a0)
    for i in range(1, n + 1):
        a = a0 + (a1 - a0) * i / n
        p = sample(rx, ry, cx, a)
        lengths.append(lengths[-1] + math.hypot(p[0] - prev[0], p[1] - prev[1]))
        prev = p
    total = lengths[-1]
    d, j = 0.0, 0
    while d <= total + 1e-9:
        while j < n and lengths[j + 1] < d:
            j += 1
        seg = lengths[j + 1] - lengths[j]
        f = 0.0 if seg <= 0 else (d - lengths[j]) / seg
        a = a0 + (a1 - a0) * (j + f) / n
        pts.append(sample(rx, ry, cx, a))
        d += DOT_PERIOD
    return pts


def circle_cmd(cx, cy, r):
    return (f"M{cx - r:.2f} {cy:.2f} "
            f"A{r} {r} 0 1 0 {cx + r:.2f} {cy:.2f} "
            f"A{r} {r} 0 1 0 {cx - r:.2f} {cy:.2f} Z")


def signal_paths():
    solid, dotted = [], []
    for p in PAIRS:
        s, d = [], []
        for side, x_end in (('L', p['xL']), ('R', p['xR'])):
            sweep = 0 if side == 'L' else 1
            s.append(f"M{x_end} {p['y0']} A {p['rx']} {p['ry']} 0 0 {sweep} {x_end} {p['y1']}")
            cx = arc_centre(p['rx'], p['ry'], x_end, p['y0'], side)
            a0, a1 = arc_angles(p['rx'], p['ry'], cx, x_end, p['y0'], p['y1'], side)
            for (dx, dy) in dots_for_arc(p['rx'], p['ry'], cx, a0, a1):
                d.append(circle_cmd(dx, dy, DOT_R))
        solid.append(' '.join(s))
        dotted.append(' '.join(d))
    return solid, dotted


# ── Nearby magnifier-over-tower (icons.tsx MagnifierTower) ──────────────────
CX, APEX, BASE = 14.8, 12.7, 22.3
LCX, LCY, R, K = 14.8, 14.3, 12.0, 0.075


def warp(x, y):
    dx, dy = x - LCX, y - LCY
    r = math.hypot(dx, dy) or 1e-4
    f = 1 + K * (1 - (r / R) ** 2)
    return LCX + dx * f, LCY + dy * f


def poly(x1, y1, x2, y2, n=8):
    out = ''
    for i in range(n + 1):
        t = i / n
        x, y = warp(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t)
        out += ('%s%.2f %.2f' % (' L' if i else 'M', x, y))
    return out


def half(y):
    return 0.8 + 3.4 * ((y - APEX) / (BASE - APEX))


def magnifier():
    xl = lambda y: CX - half(y)
    xr = lambda y: CX + half(y)
    y_top, y_bot, tip_y = APEX + 3.8, BASE - 0.6, 9.1
    wtx, wty = warp(CX, tip_y)

    def wave(r, side):
        return (f"M{wtx + side * r * 0.62:.2f} {wty - r * 0.66:.2f} "
                f"A {r} {r} 0 0 {1 if side > 0 else 0} "
                f"{wtx + side * r * 0.62:.2f} {wty + r * 0.66:.2f}")

    return {
        'legs': poly(CX - half(BASE), BASE, CX - half(APEX), APEX) + ' ' +
                poly(CX + half(BASE), BASE, CX + half(APEX), APEX),
        'brace': poly(xl(y_top), y_top, xr(y_bot), y_bot) + ' ' +
                 poly(xr(y_top), y_top, xl(y_bot), y_bot),
        'mast': poly(CX, APEX, CX, tip_y),
        'tip': circle_cmd(wtx, wty, 0.9),
        'waves': ' '.join([wave(2.6, -1), wave(4.4, -1), wave(2.6, 1), wave(4.4, 1)]),
    }


# ── GPS satellite (icons.tsx GpsSatellite, rotate(-28 12 12) baked in) ──────
GPS_ROT = math.radians(-28)


def rot(x, y, ox=12.0, oy=12.0):
    dx, dy = x - ox, y - oy
    c, s = math.cos(GPS_ROT), math.sin(GPS_ROT)
    return ox + dx * c - dy * s, oy + dx * s + dy * c


def rline(x1, y1, x2, y2):
    a, b = rot(x1, y1)
    c, d = rot(x2, y2)
    return f"M{a:.2f} {b:.2f} L{c:.2f} {d:.2f}"


def rrect(x, y, w, h):
    pts = [rot(x, y), rot(x + w, y), rot(x + w, y + h), rot(x, y + h)]
    return 'M' + ' L'.join('%.2f %.2f' % p for p in pts) + ' Z'


def rarc(x1, y1, r, x2, y2, sweep):
    a, b = rot(x1, y1)
    c, d = rot(x2, y2)
    return f"M{a:.2f} {b:.2f} A {r} {r} 0 0 {sweep} {c:.2f} {d:.2f}"


def gps():
    parts = [
        rrect(0.5, 9.9, 7.3, 4.2), rline(2.9, 9.9, 2.9, 14.1), rline(5.3, 9.9, 5.3, 14.1),
        rline(0.5, 12, 7.8, 12),
        rrect(16.2, 9.9, 7.3, 4.2), rline(18.6, 9.9, 18.6, 14.1), rline(21, 9.9, 21, 14.1),
        rline(16.2, 12, 23.5, 12),
        rline(7.8, 12, 9.1, 12), rline(14.9, 12, 16.2, 12),
        rrect(9.1, 8.3, 5.8, 7.4),
        rline(12, 15.7, 12, 18.8),
        rarc(9.9, 18.7, 3, 14.1, 18.7, 0),
        rarc(8.4, 20.4, 6, 15.6, 20.4, 0),
    ]
    return ' '.join(parts)


if __name__ == '__main__':
    solid, dotted = signal_paths()
    for i, (s, d) in enumerate(zip(solid, dotted)):
        print(f'PAIR{i}_SOLID = "{s}"')
        print(f'PAIR{i}_DOTS = "{d}"')
    for key, val in magnifier().items():
        print(f'MAG_{key.upper()} = "{val}"')
    print(f'GPS = "{gps()}"')
