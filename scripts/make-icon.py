#!/usr/bin/env python3
"""Draw the chubby DeepSeek whale Dock icon.

Usage:  python3 scripts/make-icon.py
Writes: assets/icon.png (1024), assets/preview-32.png, assets/preview-128.png,
        assets/icon.iconset/*  (then iconutil -> assets/icon.icns)
Everything is drawn at 4x and downsampled, so edges stay clean.
"""
import math
import os
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFilter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")
OUT = 1024
SS = 4
S = OUT * SS

# ---------- palette ----------
BLUE = (77, 107, 254)        # #4D6BFE main
BLUE_LIGHT = (124, 147, 255)  # #7C93FF
BLUE_DARK = (47, 73, 209)    # #2F49D1
BLUE_DEEP = (32, 46, 140)
INK = (16, 27, 60)           # eye / mouth
BLUSH = (255, 143, 163)      # #FF8FA3
BG_TOP = (27, 31, 46)        # #1B1F2E
BG_BOT = (42, 51, 80)        # #2A3350
SPOUT = (198, 224, 255)


def sc(v):
    return v * SS


def rgba(c, a=255):
    return (c[0], c[1], c[2], a)


def cubic(p0, c1, c2, p1, n=28):
    pts = []
    for i in range(1, n + 1):
        t = i / n
        mt = 1 - t
        x = mt**3 * p0[0] + 3 * mt**2 * t * c1[0] + 3 * mt * t**2 * c2[0] + t**3 * p1[0]
        y = mt**3 * p0[1] + 3 * mt**2 * t * c1[1] + 3 * mt * t**2 * c2[1] + t**3 * p1[1]
        pts.append((x, y))
    return pts


def path(p0, segs, close=True):
    """segs: list of ('c', c1, c2, p) or ('l', p)."""
    pts = [p0]
    cur = p0
    for seg in segs:
        if seg[0] == "c":
            pts += cubic(cur, seg[1], seg[2], seg[3])
            cur = seg[3]
        else:
            pts.append(seg[1])
            cur = seg[1]
    return [(sc(x), sc(y)) for x, y in pts]


def ellipse_pts(cx, cy, rx, ry, angle_deg, n=96):
    a = math.radians(angle_deg)
    ca, sa = math.cos(a), math.sin(a)
    out = []
    for i in range(n):
        t = 2 * math.pi * i / n
        x, y = rx * math.cos(t), ry * math.sin(t)
        out.append((sc(cx + x * ca - y * sa), sc(cy + x * sa + y * ca)))
    return out


def oval(draw, cx, cy, rx, ry, fill=None, outline=None, width=1):
    draw.ellipse([sc(cx - rx), sc(cy - ry), sc(cx + rx), sc(cy + ry)],
                 fill=fill, outline=outline, width=width)


def stroke(draw, pts, width, color):
    w = sc(width)
    draw.line(pts, fill=color, width=int(w), joint="curve")
    r = w / 2.0
    for p in (pts[0], pts[-1]):
        draw.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=color)


def vgrad(size, top, bot):
    w, h = size
    img = Image.new("RGB", (1, h))
    px = img.load()
    for y in range(h):
        t = y / max(1, h - 1)
        px[0, y] = tuple(int(top[i] + (bot[i] - top[i]) * t) for i in range(3))
    return img.resize((w, h), Image.BILINEAR)


def main():
    os.makedirs(ASSETS, exist_ok=True)
    canvas = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # ---- background: squircle with vertical gradient + glow ----
    bg = vgrad((S, S), BG_TOP, BG_BOT).convert("RGBA")
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gd.ellipse([sc(470 - 430), sc(600 - 430), sc(470 + 430), sc(600 + 430)],
               fill=rgba(BLUE, 46))
    glow = glow.filter(ImageFilter.GaussianBlur(sc(120)))
    bg = Image.alpha_composite(bg, glow)

    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1],
                                           radius=int(sc(OUT * 0.22)), fill=255)
    canvas.paste(bg, (0, 0), mask)
    d = ImageDraw.Draw(canvas)

    # inner hairline so the tile reads on both light and dark docks
    d.rounded_rectangle([sc(6), sc(6), sc(OUT - 6), sc(OUT - 6)],
                        radius=int(sc(OUT * 0.22 - 6)), outline=(255, 255, 255, 34),
                        width=int(sc(5)))

    # ---- ground shadow ----
    sh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(sh).ellipse([sc(210), sc(838), sc(760), sc(920)], fill=(0, 0, 0, 105))
    sh = sh.filter(ImageFilter.GaussianBlur(sc(26)))
    canvas = Image.alpha_composite(canvas, sh)
    d = ImageDraw.Draw(canvas)

    # ---- tail (behind body): two-lobe fluke ----
    tail = path((744, 494), [
        ("c", (800, 476), (848, 460), (880, 442)),
        ("c", (910, 426), (920, 458), (898, 486)),
        ("c", (874, 514), (852, 534), (846, 556)),
        ("c", (852, 578), (874, 600), (898, 630)),
        ("c", (920, 658), (910, 688), (880, 670)),
        ("c", (848, 650), (800, 634), (744, 618)),
        ("c", (726, 578), (726, 534), (744, 494)),
    ])
    d.polygon(tail, fill=rgba(BLUE_DARK))
    tl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(tl).polygon(tail, fill=rgba(BLUE_LIGHT, 60))
    tl = tl.filter(ImageFilter.GaussianBlur(sc(14)))
    canvas = Image.alpha_composite(canvas, tl)
    d = ImageDraw.Draw(canvas)

    # ---- body: fat, near-circular ----
    body_box = [sc(155), sc(270), sc(785), sc(870)]
    body_mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(body_mask).ellipse(body_box, fill=255)
    body_fill = vgrad((S, S), BLUE_LIGHT, BLUE_DARK).convert("RGBA")
    canvas.paste(body_fill, (0, 0), body_mask)
    d = ImageDraw.Draw(canvas)

    # belly patch
    belly = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(belly).ellipse([sc(255), sc(510), sc(690), sc(848)],
                                  fill=(196, 210, 255, 92))
    belly = belly.filter(ImageFilter.GaussianBlur(sc(16)))
    canvas = Image.alpha_composite(canvas, belly)
    d = ImageDraw.Draw(canvas)

    # soft top sheen
    rim = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(rim).ellipse([sc(232), sc(296), sc(660), sc(600)],
                                fill=(226, 235, 255, 74))
    rim = rim.filter(ImageFilter.GaussianBlur(sc(46)))
    rim.putalpha(Image.composite(rim.getchannel("A"),
                                 Image.new("L", (S, S), 0), body_mask))
    canvas = Image.alpha_composite(canvas, rim)
    d = ImageDraw.Draw(canvas)

    # ---- pectoral fins ----
    for cx, cy, ang in ((202, 700, 20), (742, 700, -20)):
        fin = ellipse_pts(cx, cy, 76, 42, ang)
        d.polygon(fin, fill=rgba(BLUE_DARK))
        fm = Image.new("L", (S, S), 0)
        ImageDraw.Draw(fm).polygon(fin, fill=255)
        fh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        ImageDraw.Draw(fh).polygon(ellipse_pts(cx - 6, cy - 14, 58, 20, ang),
                                   fill=rgba(BLUE_LIGHT, 150))
        fh = fh.filter(ImageFilter.GaussianBlur(sc(10)))
        fh.putalpha(Image.composite(fh.getchannel("A"), Image.new("L", (S, S), 0), fm))
        canvas = Image.alpha_composite(canvas, fh)
        d = ImageDraw.Draw(canvas)

    # ---- eyes ----
    for ex in (366, 576):
        oval(d, ex, 546, 56, 62, fill=rgba(INK))
    hl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    hd = ImageDraw.Draw(hl)
    for ex in (366, 576):
        oval(hd, ex - 17, 520, 21, 21, fill=(255, 255, 255, 255))
        oval(hd, ex + 15, 572, 9, 9, fill=(255, 255, 255, 190))
    canvas = Image.alpha_composite(canvas, hl)
    d = ImageDraw.Draw(canvas)

    # ---- smile ----
    smile = path((424, 636), [
        ("c", (440, 690), (500, 690), (516, 636)),
    ], close=False)
    stroke(d, smile, 15, rgba(INK, 235))

    # ---- blush ----
    bl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    bd = ImageDraw.Draw(bl)
    for bx in (288, 654):
        oval(bd, bx, 642, 60, 34, fill=rgba(BLUSH, 150))
    bl = bl.filter(ImageFilter.GaussianBlur(sc(15)))
    canvas = Image.alpha_composite(canvas, bl)
    d = ImageDraw.Draw(canvas)

    # ---- water spout: column + three droplets ----
    sp = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sp)
    for cy, r, a in ((238, 20, 240), (190, 15, 240), (146, 11, 230)):
        oval(sd, 470, cy, r, r * 1.12, fill=rgba(SPOUT, a))
        oval(sd, 470 - r * 0.34, cy - r * 0.4, r * 0.36, r * 0.36,
             fill=(255, 255, 255, a))
    canvas = Image.alpha_composite(canvas, sp)

    # ---- downsample ----
    icon = canvas.resize((OUT, OUT), Image.LANCZOS)
    icon.save(os.path.join(ASSETS, "icon.png"))
    icon.resize((128, 128), Image.LANCZOS).save(os.path.join(ASSETS, "preview-128.png"))
    icon.resize((32, 32), Image.LANCZOS).save(os.path.join(ASSETS, "preview-32.png"))

    # ---- iconset + icns ----
    iconset = os.path.join(ASSETS, "icon.iconset")
    os.makedirs(iconset, exist_ok=True)
    for base in (16, 32, 128, 256, 512):
        icon.resize((base, base), Image.LANCZOS).save(
            os.path.join(iconset, "icon_%dx%d.png" % (base, base)))
        icon.resize((base * 2, base * 2), Image.LANCZOS).save(
            os.path.join(iconset, "icon_%dx%d@2x.png" % (base, base)))
    subprocess.run(["iconutil", "-c", "icns", iconset,
                    "-o", os.path.join(ASSETS, "icon.icns")], check=True)
    print("wrote assets/icon.png", icon.size, icon.mode)


if __name__ == "__main__":
    main()
