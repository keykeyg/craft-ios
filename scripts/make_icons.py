#!/usr/bin/env python3
"""Original app icons: a glyph plus a big three-letter label on a solid tile.

Labels carry the meaning (not colour), so they read for colour-blind eyes too.
Output: icons/<id>.png, 1024x1024 RGB (App Store icons must not have alpha).
"""
import json, math, os
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.join(os.path.dirname(__file__), "..")
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
BG = {"pdf": (176, 32, 40), "photo": (24, 64, 150), "vector": (196, 98, 0), "layout": (120, 20, 90),
      "raw": (30, 30, 34), "film": (70, 20, 140), "motion": (90, 40, 170), "proto": (20, 110, 90),
      "write": (20, 70, 160), "grid": (16, 110, 50), "deck": (200, 70, 20), "audio": (10, 90, 120), "cad": (60, 70, 80)}
W = 1024
FG = (255, 255, 255)

def glyph(d, kind):
    cx, cy, s = 512, 380, 230  # glyph box centre and half-size
    L = 26
    if kind == "page":
        d.polygon([(cx-s*0.7, cy-s), (cx+s*0.35, cy-s), (cx+s*0.7, cy-s*0.65), (cx+s*0.7, cy+s), (cx-s*0.7, cy+s)], outline=FG, width=L)
        d.line([(cx+s*0.35, cy-s), (cx+s*0.35, cy-s*0.65), (cx+s*0.7, cy-s*0.65)], fill=FG, width=L)
    elif kind == "brush":
        d.line([(cx+s*0.8, cy-s*0.9), (cx-s*0.1, cy+s*0.1)], fill=FG, width=L*2)
        d.ellipse([cx-s*0.75, cy+s*0.05, cx-s*0.05, cy+s*0.85], fill=FG)
    elif kind == "pen":
        d.polygon([(cx, cy-s), (cx+s*0.6, cy+s*0.3), (cx, cy+s), (cx-s*0.6, cy+s*0.3)], outline=FG, width=L)
        d.line([(cx, cy-s*0.2), (cx, cy+s)], fill=FG, width=L//2)
        d.ellipse([cx-30, cy-s*0.2-30, cx+30, cy-s*0.2+30], fill=FG)
    elif kind == "columns":
        for i in range(3):
            x = cx - s + i * (s*2/3) + 12
            d.rectangle([x, cy-s, x + s*2/3 - 40, cy+s], outline=FG, width=L)
    elif kind == "aperture":
        d.ellipse([cx-s, cy-s, cx+s, cy+s], outline=FG, width=L)
        for k in range(6):
            a = k * math.pi / 3
            d.line([(cx + s*0.35*math.cos(a), cy + s*0.35*math.sin(a)), (cx + s*math.cos(a+0.9), cy + s*math.sin(a+0.9))], fill=FG, width=L//1)
    elif kind == "film":
        d.rectangle([cx-s, cy-s*0.75, cx+s, cy+s*0.75], outline=FG, width=L)
        for i in range(5):
            x = cx - s + 40 + i * (s*2-80)/4
            for y in (cy-s*0.75+28, cy+s*0.75-68):
                d.rectangle([x-22, y, x+22, y+40], fill=FG)
    elif kind == "spark":
        pts = []
        for k in range(8):
            r = s if k % 2 == 0 else s*0.3
            a = k * math.pi / 4 - math.pi/2
            pts.append((cx + r*math.cos(a), cy + r*math.sin(a)))
        d.polygon(pts, fill=FG)
    elif kind == "frames":
        d.rectangle([cx-s, cy-s, cx+s*0.3, cy+s*0.3], outline=FG, width=L)
        d.rectangle([cx-s*0.3, cy-s*0.3, cx+s, cy+s], outline=FG, width=L)
    elif kind == "lines":
        for i in range(5):
            y = cy - s + i * s*0.5
            d.line([(cx-s, y), (cx + (s if i < 4 else s*0.2), y)], fill=FG, width=L)
    elif kind == "grid":
        d.rectangle([cx-s, cy-s, cx+s, cy+s], outline=FG, width=L)
        for i in (1, 2):
            d.line([(cx-s + i*s*2/3, cy-s), (cx-s + i*s*2/3, cy+s)], fill=FG, width=L//2*1)
            d.line([(cx-s, cy-s + i*s*2/3), (cx+s, cy-s + i*s*2/3)], fill=FG, width=L//2*1)
    elif kind == "slides":
        d.rectangle([cx-s, cy-s*0.8, cx+s, cy+s*0.5], outline=FG, width=L)
        d.line([(cx, cy+s*0.5), (cx, cy+s)], fill=FG, width=L)
        d.line([(cx-s*0.4, cy+s), (cx+s*0.4, cy+s)], fill=FG, width=L)
    elif kind == "wave":
        pts = [(cx - s + t, cy - s*0.8*math.sin(t / (2*s) * 4 * math.pi) * (1 - abs(t - s) / s)) for t in range(0, int(2*s)+1, 4)]
        d.line(pts, fill=FG, width=L, joint="curve")
    elif kind == "compass":
        d.ellipse([cx-40, cy-s-40, cx+40, cy-s+40], fill=FG)
        d.line([(cx, cy-s), (cx-s*0.7, cy+s)], fill=FG, width=L)
        d.line([(cx, cy-s), (cx+s*0.7, cy+s)], fill=FG, width=L)
        d.arc([cx-s*0.75, cy, cx+s*0.75, cy+s*1.1], 200, 340, fill=FG, width=L//2)

def main():
    os.makedirs(os.path.join(ROOT, "icons"), exist_ok=True)
    font = ImageFont.truetype(FONT, 250)
    for a in json.load(open(os.path.join(ROOT, "apps.json"))):
        img = Image.new("RGB", (W, W), BG[a["id"]])
        d = ImageDraw.Draw(img)
        glyph(d, a["glyph"])
        label = a["label"]
        f = font if len(label) <= 3 else ImageFont.truetype(FONT, 210)
        box = d.textbbox((0, 0), label, font=f)
        d.text(((W - (box[2]-box[0])) / 2 - box[0], 700 - box[1]), label, font=f, fill=FG)
        img.save(os.path.join(ROOT, "icons", a["id"] + ".png"), optimize=True)
        print("icon", a["id"])

main()
