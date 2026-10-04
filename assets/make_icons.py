#!/usr/bin/env python3
"""Draws the URL Router icon and writes every format the builds need.

    assets/icon.png        1024 px preview (README)
    assets/urlrouter.ico   Windows: embedded in the .exe by build.rs
    macos/AppIcon.icns     Mac: copied into the .app by macos/build-app.sh

The mark is original: a disc split three ways in the colour families of
Firefox (orange/magenta), Edge (blue/green) and Chrome (green/yellow), with
one link forking into three in the middle. It deliberately doesn't reuse the
browsers' own logos, which their owners don't allow to be altered or combined.

Only needs Pillow (`pip install pillow`); the outputs are committed, so the
builds themselves don't. Run from anywhere: python3 assets/make_icons.py
"""

import io
import math
import struct
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SS = 4  # supersampling factor, for smooth edges

# (start colour, end colour) per wedge, clockwise from 12 o'clock.
WEDGES = [
    ((0xFF, 0x98, 0x00), (0xE3, 0x1B, 0x6D)),  # Firefox: orange -> magenta
    ((0x0F, 0x6C, 0xBD), (0x2F, 0xC2, 0x8B)),  # Edge: blue -> green
    ((0x2E, 0xA0, 0x4F), (0xF9, 0xC0, 0x0F)),  # Chrome: green -> yellow
]
INK = (0x1F, 0x29, 0x37)
MAC_BG_TOP = (0x2B, 0x33, 0x48)
MAC_BG_BOTTOM = (0x14, 0x18, 0x24)


def linear_gradient(size, c1, c2, angle_deg):
    """size x size image fading c1 -> c2 along angle_deg (screen coords)."""
    a = math.radians(angle_deg)
    dx, dy = math.cos(a), math.sin(a)
    # Project each pixel onto the direction and map to 0..1. Done on a small
    # grid and scaled up: gradients have no detail to lose.
    n = 64
    small = Image.new("RGB", (n, n))
    px = small.load()
    for y in range(n):
        for x in range(n):
            t = ((x / (n - 1) - 0.5) * dx + (y / (n - 1) - 0.5) * dy) / math.hypot(0.5, 0.5) / 2 + 0.5
            t = min(max(t, 0.0), 1.0)
            px[x, y] = tuple(round(c1[i] + (c2[i] - c1[i]) * t) for i in range(3))
    return small.resize((size, size), Image.BICUBIC)


def draw_mark(size, glyph):
    """The three-way disc on transparency, size x size."""
    s = size * SS
    c = s / 2
    r = s / 2
    out = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    for i, (c1, c2) in enumerate(WEDGES):
        start = -90 + 120 * i
        mask = Image.new("L", (s, s), 0)
        ImageDraw.Draw(mask).pieslice((0, 0, s - 1, s - 1), start, start + 120, fill=255)
        grad = linear_gradient(s, c1, c2, start + 60 + 90)
        out.paste(grad, (0, 0), mask)

    # Gaps between the wedges, cut back to transparency.
    gap = max(s * 0.035, SS * 1.0)
    cut = Image.new("L", (s, s), 255)
    d = ImageDraw.Draw(cut)
    for i in range(3):
        a = math.radians(-90 + 120 * i)
        d.line((c, c, c + math.cos(a) * r * 1.1, c + math.sin(a) * r * 1.1), fill=0, width=round(gap))
    alpha = Image.composite(out.getchannel("A"), Image.new("L", (s, s), 0), cut)
    out.putalpha(alpha)

    # White hub.
    hub = r * (0.40 if glyph else 0.30)
    d = ImageDraw.Draw(out)
    d.ellipse((c - hub - gap / 2, c - hub - gap / 2, c + hub + gap / 2, c + hub + gap / 2), fill=(0, 0, 0, 0))
    d.ellipse((c - hub, c - hub, c + hub, c + hub), fill=(255, 255, 255, 255))

    if glyph:
        # One link comes in from below and forks into three arrows.
        w = hub * 0.12
        fork = (c, c + hub * 0.20)
        base = (c, c + hub * 0.70)
        d.line((*base, *fork), fill=INK, width=round(w))
        for ang, length in ((-90, 0.80), (-145, 0.74), (-35, 0.74)):
            a = math.radians(ang)
            length *= hub
            back = w * 1.9  # arrowhead length
            spread = w * 1.3  # arrowhead half-width
            tip = (fork[0] + math.cos(a) * length, fork[1] + math.sin(a) * length)
            bx, by = tip[0] - math.cos(a) * back, tip[1] - math.sin(a) * back
            d.line((*fork, bx, by), fill=INK, width=round(w))
            px, py = -math.sin(a) * spread, math.cos(a) * spread
            d.polygon([tip, (bx + px, by + py), (bx - px, by - py)], fill=INK)
        # Round off the stem's end and the joint.
        for x, y in (base, fork):
            d.ellipse((x - w / 2, y - w / 2, x + w / 2, y + w / 2), fill=INK)

    return out.resize((size, size), Image.LANCZOS)


def windows_icon(size):
    """Full-bleed disc, as Windows icons usually are."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    margin = max(0, round(size * 0.02))
    mark = draw_mark(size - 2 * margin, glyph=size >= 40)
    img.paste(mark, (margin, margin), mark)
    return img


def mac_icon(size):
    """Apple's Big Sur layout: a rounded square on a 1024 grid with a 100 px
    inset and a soft shadow, the disc centred on it."""
    s = size * SS
    k = s / 1024
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    box = (100 * k, 100 * k, 924 * k, 924 * k)
    radius = 185 * k
    shadow = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (box[0], box[1] + 12 * k, box[2], box[3] + 12 * k), radius, fill=(0, 0, 0, 90)
    )
    img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(14 * k)))

    tile_mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(tile_mask).rounded_rectangle(box, radius, fill=255)
    tile = linear_gradient(s, MAC_BG_TOP, MAC_BG_BOTTOM, 90).convert("RGBA")
    img.paste(tile, (0, 0), tile_mask)

    d = round(640 * k)
    mark = draw_mark(d // SS, glyph=size >= 64).resize((d, d), Image.LANCZOS)
    off = round((s - d) / 2)
    img.alpha_composite(mark, (off, off))
    return img.resize((size, size), Image.LANCZOS)


def png_bytes(img):
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def write_ico(path, sizes):
    """ICO with PNG-compressed entries (supported by Windows Vista and later)."""
    images = [png_bytes(windows_icon(n)) for n in sizes]
    header = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    entries = b""
    for n, data in zip(sizes, images):
        dim = 0 if n >= 256 else n  # 0 means 256
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    path.write_bytes(header + entries + b"".join(images))


def write_icns(path):
    """ICNS with PNG payloads, which macOS 10.7 and later read."""
    blocks = [
        (b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128), (b"ic08", 256),
        (b"ic09", 512), (b"ic10", 1024), (b"ic11", 32), (b"ic12", 64), (b"ic13", 256),
        (b"ic14", 512),
    ]
    cache = {}
    body = b""
    for kind, n in blocks:
        if n not in cache:
            cache[n] = png_bytes(mac_icon(n))
        body += kind + struct.pack(">I", 8 + len(cache[n])) + cache[n]
    path.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def main():
    (ROOT / "assets").mkdir(exist_ok=True)
    windows_icon(1024).save(ROOT / "assets" / "icon.png", optimize=True)
    mac_icon(1024).save(ROOT / "assets" / "icon-mac.png", optimize=True)
    write_ico(ROOT / "assets" / "urlrouter.ico", [16, 20, 24, 32, 40, 48, 64, 128, 256])
    write_icns(ROOT / "macos" / "AppIcon.icns")
    print("wrote assets/icon.png, assets/icon-mac.png, assets/urlrouter.ico, macos/AppIcon.icns")


if __name__ == "__main__":
    main()
