#!/usr/bin/env python3
"""Build the app icon from a Zhihu image (DeepSeek "blue fat fish" maid meme).

Source: https://www.zhihu.com/question/2093751116613866755  (image saved as
assets/icon-source-zhihu.jpg; another candidate, the chibi-at-laptop meme, is
assets/icon-source-zhihu-alt.jpg)

Usage: python3 scripts/make-icon-from-image.py [variant]
       variant: a (default, laughing face) | b (chibi at laptop) | c (whale hoodie)
Needs Pillow: ~/.dsh/dsh-runtimes/dsh-primary-runtime/dependencies/python/bin/python3
"""
import os
import shutil
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFilter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")
SIZE = 1024
# Apple's macOS icon grid: the rounded body is 824x824 inside a 1024 canvas with
# a transparent margin, which is why full-bleed artwork looks oversized in the
# Dock next to system icons.
BODY = 824

# name -> (source file, square crop on the original)
VARIANTS = {
    "a": ("icon-source-zhihu.jpg", (230, 148, 682, 600)),
    "b": ("icon-source-zhihu-alt.jpg", (108, 22, 438, 352)),
    "c": ("icon-source-zhihu-alt2.jpg", (608, 20, 1048, 460)),
}


def squircle(size, ratio=0.225, ss=4):
    mask = Image.new("L", (size * ss, size * ss), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, size * ss - 1, size * ss - 1], radius=int(size * ss * ratio), fill=255)
    return mask.resize((size, size), Image.LANCZOS)


def render(src_name, box):
    src = os.path.join(ASSETS, src_name)
    im = Image.open(src).convert("RGB").crop(box)
    w, h = im.size
    side = max(w, h)
    canvas = Image.new("RGB", (side, side), (20, 26, 56))
    canvas.paste(im, ((side - w) // 2, (side - h) // 2))
    canvas = canvas.resize((SIZE, SIZE), Image.LANCZOS).filter(
        ImageFilter.UnsharpMask(radius=2, percent=55, threshold=3))
    art = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    art.paste(canvas, (0, 0), squircle(SIZE))
    # Corner radius scales with the body, so shrinking the 1024 body to 824
    # keeps it at 185.4 px, the value Apple uses on the 1024 grid.
    icon = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    offset = (SIZE - BODY) // 2
    icon.alpha_composite(art.resize((BODY, BODY), Image.LANCZOS), (offset, offset))
    return icon


def icns(icon):
    iconset = os.path.join(ASSETS, "icon.iconset")
    shutil.rmtree(iconset, ignore_errors=True)
    os.makedirs(iconset)
    for base in (16, 32, 128, 256, 512):
        icon.resize((base, base), Image.LANCZOS).save(
            os.path.join(iconset, "icon_%dx%d.png" % (base, base)))
        icon.resize((base * 2, base * 2), Image.LANCZOS).save(
            os.path.join(iconset, "icon_%dx%d@2x.png" % (base, base)))
    out = os.path.join(ASSETS, "icon.icns")
    subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out], check=True)
    shutil.rmtree(iconset, ignore_errors=True)
    print("wrote assets/icon.icns", os.path.getsize(out), "bytes")


def main():
    want = (sys.argv[1] if len(sys.argv) > 1 else "a").lower()
    if want not in VARIANTS:
        sys.exit("variant must be one of %s" % ", ".join(VARIANTS))
    cand = os.path.join(ASSETS, "icon-candidates")
    os.makedirs(cand, exist_ok=True)
    for name, (src, box) in VARIANTS.items():
        if not os.path.exists(os.path.join(ASSETS, src)):
            continue
        render(src, box).save(os.path.join(cand, "%s.png" % name))
    icon = render(*VARIANTS[want])
    icon.save(os.path.join(ASSETS, "icon.png"))
    for px in (32, 128):
        icon.resize((px, px), Image.LANCZOS).save(
            os.path.join(ASSETS, "preview-%d.png" % px))
    print("wrote assets/icon.png from variant", want)
    icns(icon)


if __name__ == "__main__":
    main()
