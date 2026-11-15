#!/usr/bin/env python3
"""Loom Studio brand files that stand in for the ArtCraft ones the apps compile in.

Writes shim/brand/<same file names as the apps' docs/brand/> so `include_bytes!` keeps working.
The mark is a woven grid: a rounded frame with warp and weft threads. No text, so it renders
anywhere (SVG text would need fonts at runtime).
"""
import os
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(__file__), "..", "shim", "brand")
DARK, WHITE = "#1d1d1f", "#ffffff"


def mark_svg(x, y, s, color):
    k = s / 100.0
    def L(x1, y1, x2, y2):
        return f'<line x1="{x+x1*k:.2f}" y1="{y+y1*k:.2f}" x2="{x+x2*k:.2f}" y2="{y+y2*k:.2f}"/>'
    parts = [f'<rect x="{x+6*k:.2f}" y="{y+6*k:.2f}" width="{88*k:.2f}" height="{88*k:.2f}" rx="{22*k:.2f}"/>']
    parts += [L(38, 22, 38, 78), L(62, 22, 62, 78)]
    parts += [L(22, 34, 78, 34), L(22, 50, 78, 50), L(22, 66, 78, 66)]
    return (f'<g fill="none" stroke="{color}" stroke-width="{8*k:.2f}" stroke-linecap="round">' + "".join(parts) + "</g>")


def svg(viewbox, body):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{viewbox}" role="img" aria-label="Loom Studio"><title>Loom Studio</title>{body}</svg>\n'


def mark_png(size, color, canvas=None, at=(0, 0)):
    img = canvas or Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    k = size / 100.0
    ox, oy = at
    w = max(1, round(8 * k))
    d.rounded_rectangle([ox + 6 * k, oy + 6 * k, ox + 94 * k, oy + 94 * k], radius=22 * k, outline=color, width=w)
    for x in (38, 62):
        d.line([ox + x * k, oy + 22 * k, ox + x * k, oy + 78 * k], fill=color, width=w)
    for y in (34, 50, 66):
        d.line([ox + 22 * k, oy + y * k, ox + 78 * k, oy + y * k], fill=color, width=w)
    return img


def main():
    os.makedirs(OUT, exist_ok=True)
    # Same canvases as the files they replace: marks are square, logos are a wide strip.
    for name, color in [("artcraft-mark.svg", DARK), ("artcraft-mark-black.svg", "#0b0b0c")]:
        open(os.path.join(OUT, name), "w").write(svg("0 0 100 100", mark_svg(0, 0, 100, color)))
    for name, color in [("artcraft-logo.svg", DARK), ("artcraft-logo-white.svg", WHITE)]:
        open(os.path.join(OUT, name), "w").write(svg("-4 -4 1038.05 108", mark_svg(0, 0, 100, color)))
    mark_png(1024, DARK).save(os.path.join(OUT, "artcraft-mark.png"))
    mark_png(1024, "#0b0b0c").save(os.path.join(OUT, "artcraft-mark-black.png"))
    for name, color in [("artcraft-logo.png", DARK), ("artcraft-logo-white.png", WHITE)]:
        strip = Image.new("RGBA", (3200, 333), (0, 0, 0, 0))
        mark_png(320, color, canvas=strip, at=(6, 6)).save(os.path.join(OUT, name))
    print("brand files:", sorted(os.listdir(OUT)))


main()
