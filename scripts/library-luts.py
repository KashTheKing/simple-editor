#!/usr/bin/env python3
"""Generate library/luts/*.cube from the colour recipes below (stdlib only).

    python scripts/library-luts.py [out_dir]          rewrite every LUT (default library/luts)
    python scripts/library-luts.py [out_dir] --check  also verify the files and print where key colours land

Change a recipe here, not a .cube file. Output is deterministic. All maths is on display-referred
sRGB values 0..1; only `exposure` goes through linear light.
"""
import colorsys
import math
import os
import re
import sys

N = 17
LW = (0.2126, 0.7152, 0.0722)


# ---------------------------------------------------------------- helpers
def luma(c): return LW[0] * c[0] + LW[1] * c[1] + LW[2] * c[2]
def cl(x): return 0.0 if x < 0 else 1.0 if x > 1 else x
def clamp3(c): return (cl(c[0]), cl(c[1]), cl(c[2]))
def mix(a, b, t): return a + (b - a) * t
def sstep(a, b, x):
    t = cl((x - a) / (b - a))
    return t * t * (3 - 2 * t)
def tup(v): return v if isinstance(v, (tuple, list)) else (v, v, v)
def to_lin(x): return x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4
def to_srgb(x):
    x = max(x, 0.0)
    return x * 12.92 if x <= 0.0031308 else 1.055 * x ** (1 / 2.4) - 0.055


# ---------------------------------------------------------------- ops (each returns f(rgb) -> rgb)
def exposure(stops):
    k = 2.0 ** stops
    return lambda c: tuple(to_srgb(to_lin(cl(v)) * k) for v in c)


def contrast(a, passes=1):
    """Smooth S-curve blend around 0.5; a<0 flattens. Monotonic for |a|<=1 per pass."""
    a = tup(a)
    def f(c):
        out = []
        for v, k in zip(c, a):
            v = cl(v)
            for _ in range(passes):
                v = v + k * (sstep(0, 1, v) - v)
            out.append(v)
        return tuple(out)
    return f


def lgg(lift=0.0, gamma=1.0, gain=1.0):
    lift, gamma, gain = tup(lift), tup(gamma), tup(gain)
    return lambda c: tuple(
        max(cl(v) * g + l * (1 - cl(v)), 0.0) ** gm for v, l, gm, g in zip(c, lift, gamma, gain)
    )


def fade(lift=0.0, roll=0.0):
    """Faded blacks (lift) and lowered white point (roll)."""
    return lambda c: tuple(lift + (1 - lift - roll) * cl(v) for v in c)


def shoulder(k, start=0.65):
    """Soft highlight roll-off above `start` (k < 0.5 keeps it monotonic)."""
    def one(x):
        x = cl(x)
        return x - k * max(0.0, x - start) ** 2 / (1 - start)
    return lambda c: tuple(one(v) for v in c)


def sat(s):
    def f(c):
        y = luma(c)
        return tuple(y + (v - y) * s for v in c)
    return f


def vibrance(v):
    """Saturate the dull colours more than the vivid ones."""
    def f(c):
        mx, mn = max(c), min(c)
        cur = (mx - mn) / mx if mx > 1e-6 else 0.0
        return sat(1 + v * (1 - cur))(c)
    return f


def hsel(center, width, dh=0.0, ds=0.0, dl=0.0):
    """Hue-selective shift in HSL: hue by dh degrees, saturation x(1+ds), lightness +dl, weighted by
    hue closeness (cosine window of half-width `width`), saturation and mid-tone-ness."""
    def f(c):
        c = clamp3(c)
        h, l, s = colorsys.rgb_to_hls(*c)
        d = abs((h * 360 - center + 180) % 360 - 180)
        if d >= width or s < 1e-3:
            return c
        w = 0.5 * (1 + math.cos(math.pi * d / width)) * min(1.0, s * 4) * min(1.0, 6 * l * (1 - l))
        h = ((h * 360 + dh * w) % 360) / 360
        s = cl(s * (1 + ds * w))
        l = cl(l + dl * w)
        return colorsys.hls_to_rgb(h, l, s)
    return f


def _tint_vec(hue, strength):
    r, g, b = colorsys.hsv_to_rgb((hue % 360) / 360, 1, 1)
    m = (r + g + b) / 3
    v = [(x - m) * 2 for x in (r, g, b)]
    y = luma(v)  # luma-neutral so a tint never changes brightness
    return tuple((x - y) * strength for x in v)


def split(shadow=None, high=None):
    """Split toning: shadow=(hue, strength), high=(hue, strength), weighted by luma.
    Pure black and pure white are left alone."""
    sv = _tint_vec(*shadow) if shadow else (0, 0, 0)
    hv = _tint_vec(*high) if high else (0, 0, 0)
    def f(c):
        l = cl(luma(c))
        ws = (1 - l) ** 2 * sstep(0.0, 0.3, l)
        wh = l ** 2 * (1 - sstep(0.7, 1.0, l))
        return tuple(v + ws * s + wh * h for v, s, h in zip(c, sv, hv))
    return f


def mixer(m):
    return lambda c: tuple(sum(m[i][j] * c[j] for j in range(3)) for i in range(3))


def temp(t, tint=0.0):
    """t>0 warmer, t<0 cooler; tint>0 magenta, <0 green."""
    return lambda c: (c[0] * (1 + 0.10 * t), c[1] * (1 + 0.02 * t) * (1 - 0.06 * tint), c[2] * (1 - 0.10 * t))


def bw(w=LW):
    return lambda c: (lambda y: (y, y, y))(w[0] * c[0] + w[1] * c[1] + w[2] * c[2])


def duotone(stops):
    """Map luma through a colour gradient [(pos, (r,g,b)), ...]."""
    def f(c):
        y = cl(luma(c))
        for (p0, c0), (p1, c1) in zip(stops, stops[1:]):
            if y <= p1:
                t = (y - p0) / (p1 - p0)
                return tuple(mix(a, b, t) for a, b in zip(c0, c1))
        return stops[-1][1]
    return f


# ---------------------------------------------------------------- recipes
# (id, name, description, tags, [ops])
RECIPES = [
    ("teal-and-orange", "Teal and Orange", "Warm skin, teal shadows: the blockbuster look.", "cinematic, warm, teal",
     [contrast(0.3), split((190, 0.11), (32, 0.08)), hsel(120, 60, 35, -0.15), hsel(225, 40, -12, 0.1),
      hsel(30, 25, 0, 0.1), sat(1.08)]),
    ("warm-film", "Warm Film", "Soft, golden, slightly faded colour with gentle contrast.", "film, warm, soft, nostalgic",
     [temp(0.3), contrast(0.22), fade(0.025, 0.02), sat(0.92), split((175, 0.03), (42, 0.05)), shoulder(0.2)]),
    ("cool-noir", "Cool Noir", "Almost colourless, steel-blue and contrasty like a rainy night street.", "noir, cool, dark, moody",
     [sat(0.22), temp(-0.45), contrast(0.55), lgg(gamma=1.1), split((215, 0.07), (210, 0.03))]),
    ("bleach-bypass", "Bleach Bypass", "Silvery, desaturated and gritty with deep blacks.", "gritty, desaturated, war, contrast",
     [sat(0.5), contrast(0.6), lgg(gamma=1.05), temp(-0.05)]),
    ("faded-matte", "Faded Matte", "Lifted blacks and soft colour for a flat, matte film look.", "matte, faded, lifted, indie",
     [contrast(0.12), sat(0.85), fade(0.09, 0.05), split((190, 0.03), (40, 0.03))]),
    ("vintage-70s", "Vintage 70s", "Warm amber cast, creamy highlights and olive greens.", "retro, vintage, warm, amber",
     [temp(0.35, 0.1), contrast(0.18), fade(0.04, 0.025), sat(0.9), hsel(105, 50, -30, -0.3),
      split((25, 0.04), (60, 0.05)), shoulder(0.2)]),
    ("moody-green", "Moody Green", "Dark, cool, green-tinted shadows with rich foliage.", "moody, green, dark, cinematic",
     [lgg(gamma=1.15), contrast(0.35), sat(0.88), hsel(100, 60, 12, 0.2, -0.03), split((150, 0.07), (95, 0.02)),
      temp(-0.1)]),
    ("neon-night", "Neon Night", "Deep blue-violet shadows and electric colour for city lights.", "neon, night, vivid, purple",
     [lgg(gamma=1.25), contrast(0.5), sat(1.3), split((270, 0.12), (320, 0.04)), hsel(280, 50, 0, 0.2)]),
    ("golden-hour", "Golden Hour", "Low sun glow: warm highlights, bright and flattering.", "warm, golden, sunset, glow",
     [exposure(0.1), temp(0.5), lgg(gamma=0.92), contrast(0.18), sat(1.1), split((20, 0.03), (42, 0.09)), shoulder(0.2)]),
    ("winter-blue", "Winter Blue", "Crisp, cold and bright with icy blue shadows.", "cold, blue, winter, crisp",
     [temp(-0.5), lgg(gamma=0.96), contrast(0.2), sat(0.85), split((222, 0.06), (200, 0.03))]),
    ("punchy-vlog", "Punchy Vlog", "Bright, clean and contrasty with lively colour; made for faces.", "vlog, bright, punchy, clean",
     [temp(0.06), exposure(0.12), lgg(gamma=0.93), contrast(0.25), vibrance(0.45), sat(1.0)]),
    ("gaming-vibrant", "Gaming Vibrant", "Saturated, hard-hitting colour with cool shadows for gameplay.", "gaming, vibrant, saturated, bold",
     [contrast(0.42), vibrance(0.35), sat(1.3), split((245, 0.05), (320, 0.02)), hsel(220, 45, 0, 0.2), hsel(300, 40, 0, 0.2)]),
    ("horror", "Horror", "Sickly desaturated green-grey with crushed, cold shadows.", "horror, dark, creepy, desaturated",
     [sat(0.4), lgg(gamma=1.25), contrast(0.5), split((150, 0.09), (110, 0.04)), hsel(5, 30, 0, 0.35)]),
    ("day-for-night", "Day for Night", "Dim, blue moonlight look made from daytime footage.", "night, blue, dark, moonlight",
     [lgg(gamma=1.35), sat(0.95), temp(-0.18), contrast(0.25), split((228, 0.08), (210, 0.02))]),
    ("sepia", "Sepia", "Classic brown-toned monochrome.", "sepia, brown, old, monochrome",
     [bw((0.3, 0.59, 0.11)), contrast(0.2),
      duotone([(0.0, (0.06, 0.035, 0.02)), (0.5, (0.50, 0.39, 0.27)), (1.0, (0.98, 0.93, 0.80))])]),
    ("bw-high-contrast", "B&W High Contrast", "Punchy black and white with dark skies and deep blacks.", "black and white, contrast, dramatic, mono",
     [bw((0.45, 0.45, 0.10)), contrast(1.0), contrast(0.35)]),
    ("bw-soft", "B&W Soft", "Gentle black and white with lifted blacks and smooth tones.", "black and white, soft, gentle, mono",
     [bw((0.30, 0.59, 0.11)), lgg(gamma=0.96), contrast(0.12), fade(0.03, 0.02)]),
    ("classic-warm-stock", "Classic Warm Stock", "Rich reds and yellows, warm shadows and a golden cast, like a vintage print stock.",
     "film, warm, rich, classic",
     [temp(0.25), contrast(0.3), sat(1.12), hsel(100, 55, -25, 0.05), hsel(10, 25, 0, 0.12), split((20, 0.04), (50, 0.05)),
      shoulder(0.15)]),
    ("cross-process", "Cross Process", "Blue-cyan shadows, yellow-green highlights and hard contrast.", "cross process, retro, edgy, colour shift",
     [contrast((0.65, 0.35, -0.15)), lgg(lift=(0.0, 0.01, 0.05), gain=(1.0, 1.0, 0.93)), sat(1.1)]),
    ("pastel-dream", "Pastel Dream", "Soft, airy, lifted colour in candy pinks and blues.", "pastel, soft, dreamy, airy",
     [lgg(gamma=0.85), contrast(-0.1), sat(0.82), fade(0.1, 0.05), split((305, 0.05), (190, 0.03))]),
    ("cyberpunk", "Cyberpunk", "Magenta shadows, cyan highlights and glowing neon accents.", "cyberpunk, neon, magenta, cyan, scifi",
     [contrast(0.4), sat(1.18), lgg(gamma=1.1), split((310, 0.1), (185, 0.06)), hsel(30, 25, 0, -0.1)]),
    ("desert-heat", "Desert Heat", "Dry ochre earth, burnt oranges and bleached sky.", "desert, hot, warm, dry",
     [temp(0.5), lgg(gamma=0.93), contrast(0.28), hsel(220, 40, -15, -0.4), hsel(100, 60, -35, -0.2),
      split((14, 0.05), (45, 0.08)), shoulder(0.2)]),
    ("forest-deep", "Forest Deep", "Dense, deep greens and cool shade for woodland scenes.", "forest, green, nature, deep",
     [lgg(gamma=1.08), contrast(0.3), hsel(110, 50, 8, 0.25, -0.05), split((165, 0.05), (80, 0.02)), temp(-0.05)]),
    ("ocean-teal", "Ocean Teal", "Turquoise water and cool shadows with warm skin kept natural.", "ocean, teal, water, cool",
     [temp(-0.1), contrast(0.22), hsel(205, 50, -15, 0.3), hsel(120, 40, 25, 0, 0), split((185, 0.07), (38, 0.03)),
      sat(1.05)]),
    ("rose-gold", "Rose Gold", "Soft pink-gold glow with creamy skin and light shadows.", "rose, pink, gold, soft, glam",
     [temp(0.15, 0.3), lgg(gamma=0.92), contrast(0.12), sat(0.95), fade(0.02, 0.01), split((340, 0.06), (12, 0.1)),
      shoulder(0.15)]),
    ("clean-skin", "Clean Skin", "Gentle and flattering: slightly warm, smooth skin, natural everything else.", "skin, portrait, natural, flattering",
     [temp(0.12), lgg(gamma=0.93), contrast(0.1), hsel(25, 28, 0, -0.1, 0.02), sat(0.97), fade(0.01, 0.0)]),
    ("food-pop", "Food Pop", "Warm, juicy and appetising: bright reds, oranges and greens.", "food, warm, saturated, appetising",
     [temp(0.3), lgg(gamma=0.95), contrast(0.28), vibrance(0.3), hsel(25, 40, 0, 0.3, 0.02), hsel(100, 50, -8, 0.15),
      hsel(225, 50, 0, -0.35), sat(1.1)]),
    ("travel-vivid", "Travel Vivid", "Deep blue skies, rich greens and warm light for holiday footage.", "travel, vivid, bright, holiday",
     [temp(0.05), lgg(gamma=0.96), contrast(0.28), vibrance(0.3), hsel(215, 40, 0, 0.45, -0.07), hsel(190, 30, 0, 0.3), hsel(105, 50, 0, 0.15),
      sat(1.1)]),
    ("muted-documentary", "Muted Documentary", "Low-saturation, natural and unfussy, with a touch of cool shade.", "documentary, muted, natural, neutral",
     [sat(0.7), contrast(0.2), fade(0.02, 0.03), split((200, 0.02), (45, 0.02))]),
    ("sunset-magenta", "Sunset Magenta", "Pink and magenta dusk glow over deep violet shadows.", "sunset, magenta, pink, dusk",
     [temp(0.25, 0.3), contrast(0.28), sat(1.1), split((280, 0.07), (340, 0.09)), hsel(30, 25, -6, 0.1), shoulder(0.15)]),
]

# looks allowed to push skin grey / green / off-colour
SKIN_EXEMPT = {"horror", "cool-noir", "bw-high-contrast", "bw-soft", "sepia"}
# looks where black / white may leave the corners on purpose
CORNER_EXEMPT = {"faded-matte", "pastel-dream", "sepia", "cool-noir", "horror", "day-for-night", "bw-soft", "muted-documentary",
                 "vintage-70s", "warm-film", "rose-gold", "cross-process"}
SKIN = [(0.80, 0.60, 0.50), (0.45, 0.30, 0.22)]
BRANDS = re.compile(r"kodak|fuji|agfa|ilford|arri|red\b|sony|canon|panavision|technicolor|cinestill|polaroid", re.I)


def apply(ops, c):
    for op in ops:
        c = op(c)
    return clamp3(c)


def render(rec):
    rid, name, desc, tags, ops = rec
    lines = [f"# @name {name}", f"# @desc {desc}", f"# @tags {tags}", f'TITLE "{name}"', f"LUT_3D_SIZE {N}"]
    for b in range(N):
        for g in range(N):
            for r in range(N):
                o = apply(ops, (r / (N - 1), g / (N - 1), b / (N - 1)))
                lines.append("%.6f %.6f %.6f" % o)
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------- check
def parse(path):
    text = open(path, "rb").read().decode("utf-8")
    assert "\r" not in text and not text.startswith("﻿"), "CR or BOM"
    ls = text.split("\n")
    assert ls[-1] == "", "no trailing LF"
    ls = ls[:-1]
    assert ls[0].startswith("# @name ") and ls[1].startswith("# @desc ") and ls[2].startswith("# @tags "), "header"
    assert ls[3].startswith('TITLE "') and ls[4] == f"LUT_3D_SIZE {N}", "TITLE/size"
    assert not any(l.startswith("DOMAIN") for l in ls)
    rows = [tuple(float(x) for x in l.split()) for l in ls[5:]]
    assert len(rows) == N ** 3, f"{len(rows)} rows"
    assert all(len(r) == 3 and all(0 <= v <= 1 for v in r) for r in rows), "range"
    assert all(re.fullmatch(r"\d\.\d{6} \d\.\d{6} \d\.\d{6}", l) for l in ls[5:]), "row format"
    return ls[:5], rows


def sample(rows, c):
    p = [cl(v) * (N - 1) for v in c]
    i0 = [min(int(math.floor(v)), N - 2) for v in p]
    f = [v - i for v, i in zip(p, i0)]
    out = [0.0] * 3
    for dz in (0, 1):
        for dy in (0, 1):
            for dx in (0, 1):
                w = (f[0] if dx else 1 - f[0]) * (f[1] if dy else 1 - f[1]) * (f[2] if dz else 1 - f[2])
                s = rows[(i0[0] + dx) + (i0[1] + dy) * N + (i0[2] + dz) * N * N]
                for k in range(3):
                    out[k] += s[k] * w
    return tuple(out)


def hs(c):
    h, s, v = colorsys.rgb_to_hsv(*c)
    h = h * 360
    return (h - 360 if h > 180 else h), s, v


def check(out_dir):
    bad = 0
    print(f"{'look':20s} {'black':>14s} {'mid grey':>14s} {'white':>14s}   skin light (hue/sat)  skin dark (hue/sat)")
    for rec in RECIPES:
        rid = rec[0]
        path = os.path.join(out_dir, rid + ".cube")
        try:
            head, rows = parse(path)
            assert not BRANDS.search(" ".join(head) + rec[1] + rec[2] + rec[3]), "brand name"
            grey = [rows[i * (1 + N + N * N)] for i in range(N)]
            for a, b in zip(grey, grey[1:]):
                assert luma(b) >= luma(a) - 1e-6, "grey axis luma not monotonic"
                assert all(b[k] >= a[k] - 1e-6 for k in range(3)), "grey axis channel not monotonic"
            blk, mid, wht = grey[0], grey[N // 2], grey[-1]
            if rid not in CORNER_EXEMPT:
                assert max(blk) < 0.06 and min(wht) > 0.9, "black/white drifted"
            assert max(blk) < 0.2 and min(wht) > 0.75, "black/white far off"
            sk = []
            for s in SKIN:
                o = sample(rows, s)
                h, sa, _ = hs(o)
                sk.append((o, h, sa))
                if rid not in SKIN_EXEMPT:
                    assert -18 <= h <= 52 and sa >= 0.2, f"skin {s} -> hue {h:.0f} sat {sa:.2f}"
            f = lambda c: "%.2f,%.2f,%.2f" % c
            print(f"{rid:20s} {f(blk):>14s} {f(mid):>14s} {f(wht):>14s}   " +
                  "   ".join(f"{f(o)} ({h:+.0f}/{sa:.2f})" for o, h, sa in sk))
        except AssertionError as e:
            bad += 1
            print(f"{rid:20s} FAIL: {e}")
    names = [r[1] for r in RECIPES]
    assert len(RECIPES) == 30 and len(set(names)) == 30 and len({r[0] for r in RECIPES}) == 30
    files = sorted(f for f in os.listdir(out_dir) if f.endswith(".cube"))
    assert files == sorted(r[0] + ".cube" for r in RECIPES), "unexpected files in " + out_dir
    print("CHECK", "FAILED (%d)" % bad if bad else "OK: 30 files, 4913 rows each")
    return bad == 0


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out_dir = args[0] if args else os.path.join(root, "library", "luts")
    os.makedirs(out_dir, exist_ok=True)
    for rec in RECIPES:
        with open(os.path.join(out_dir, rec[0] + ".cube"), "w", encoding="utf-8", newline="\n") as fh:
            fh.write(render(rec))
    print(f"wrote {len(RECIPES)} LUTs to {out_dir}")
    if "--check" in sys.argv and not check(out_dir):
        sys.exit(1)


if __name__ == "__main__":
    main()
