#!/usr/bin/env python3
"""Generate Aureate's icon set without external dependencies.

Draws a dark ink rounded square with a gold "A" monogram, supersampled 4x,
then writes PNG / ICO / ICNS containers by hand.  Everything stays inside the
repository.
"""

import math
import os
import struct
import zlib

SCALE = 4
SIZE = 1024
OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   "crates", "aur-app", "icons")


def rounded_rect_distance(x, y, x0, y0, x1, y1, r):
    """Signed distance to a rounded rectangle."""
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    hx, hy = (x1 - x0) / 2, (y1 - y0) / 2
    dx = abs(x - cx) - (hx - r)
    dy = abs(y - cy) - (hy - r)
    ax, ay = max(dx, 0.0), max(dy, 0.0)
    return math.sqrt(ax * ax + ay * ay) + min(max(dx, dy), 0.0) - r


def seg_distance(px, py, ax, ay, bx, by):
    vx, vy = bx - ax, by - ay
    wx, wy = px - ax, py - ay
    denom = vx * vx + vy * vy
    t = 0.0 if denom == 0 else max(0.0, min(1.0, (wx * vx + wy * vy) / denom))
    dx, dy = px - (ax + t * vx), py - (ay + t * vy)
    return math.sqrt(dx * dx + dy * dy)


def mix(c0, c1, t):
    return tuple(int(round(c0[i] + (c1[i] - c0[i]) * t)) for i in range(3))


def render(size):
    """Render at `size * SCALE` then box-downsample."""
    hi = size * SCALE
    buf = bytearray(hi * hi * 4)

    ink_top = (0x24, 0x20, 0x1A)
    ink_bottom = (0x12, 0x10, 0x0D)
    gold_top = (0xF6, 0xDF, 0xA6)
    gold_bottom = (0xB0, 0x7C, 0x24)
    cream = (0xFA, 0xF4, 0xE6)

    # Geometry in unit space.
    pad = 0.045
    radius = 0.215
    stroke = 0.020
    leg_w = 0.037
    cross_w = 0.026

    apex = (0.500, 0.246)
    left_foot = (0.286, 0.760)
    right_foot = (0.714, 0.760)
    cross_y = 0.612
    # Crossbar spans the inner width of the A at cross_y.
    t = (cross_y - apex[1]) / (left_foot[1] - apex[1])
    half = (apex[0] - left_foot[0]) * t
    cross_x0, cross_x1 = 0.5 - half * 0.86, 0.5 + half * 0.86

    for py in range(hi):
        y = (py + 0.5) / hi
        for px in range(hi):
            x = (px + 0.5) / hi
            i = (py * hi + px) * 4

            # Rounded square plate.
            d = rounded_rect_distance(x, y, pad, pad, 1 - pad, 1 - pad, radius)
            if d > 0:
                buf[i + 3] = 0
                continue

            bg = mix(ink_top, ink_bottom, (y - pad) / (1 - 2 * pad))
            # Subtle radial light from the top-left.
            glow = max(0.0, 1.0 - math.dist((x, y), (0.28, 0.16)) / 0.95)
            bg = mix(bg, (0x3A, 0x33, 0x28), glow * 0.35)
            col = bg
            alpha = 255

            # Hairline inner border.
            if abs(d + stroke * 1.35) < stroke * 0.28:
                col = mix(col, (0xB0, 0x7C, 0x24), 0.55)

            # Gold monogram.
            gold = mix(gold_top, gold_bottom, max(0.0, min(1.0, (y - 0.22) / 0.56)))
            on_gold = False
            if seg_distance(x, y, *apex, *left_foot) <= leg_w:
                on_gold = True
            elif seg_distance(x, y, *apex, *right_foot) <= leg_w:
                on_gold = True
            elif cross_x0 <= x <= cross_x1 and abs(y - cross_y) <= cross_w:
                on_gold = True

            if on_gold:
                # Bevel: lighter at the top edge of each stroke.
                col = gold
                # Fine highlight line along the left leg.
                if abs(seg_distance(x, y, *apex, *left_foot) - leg_w * 0.55) < 0.0035:
                    col = mix(col, cream, 0.5)

            if d > -stroke * 0.5:
                alpha = 255
            buf[i] = col[0]
            buf[i + 1] = col[1]
            buf[i + 2] = col[2]
            buf[i + 3] = alpha

    # Downsample.
    out = bytearray(size * size * 4)
    n = SCALE * SCALE
    for y in range(size):
        for x in range(size):
            r = g = b = a = 0
            for sy in range(SCALE):
                base = ((y * SCALE + sy) * hi + x * SCALE) * 4
                for sx in range(SCALE):
                    k = base + sx * 4
                    r += buf[k]
                    g += buf[k + 1]
                    b += buf[k + 2]
                    a += buf[k + 3]
            o = (y * size + x) * 4
            out[o] = r // n
            out[o + 1] = g // n
            out[o + 2] = b // n
            out[o + 3] = a // n
    return bytes(out)


def png(size, raw):
    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data +
                struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    stride = size * 4
    lines = b"".join(b"\x00" + raw[y * stride:(y + 1) * stride] for y in range(size))
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) +
            chunk(b"IDAT", zlib.compress(lines, 9)) + chunk(b"IEND", b""))


def ico(images):
    """images: list of (size, png_bytes)."""
    header = struct.pack("<HHH", 0, 1, len(images))
    entries = b""
    offset = 6 + 16 * len(images)
    blobs = b""
    for size, data in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        blobs += data
    return header + entries + blobs


def icns(images):
    """images: list of (icns_type, png_bytes)."""
    body = b""
    for kind, data in images:
        body += kind + struct.pack(">I", len(data) + 8) + data
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def main():
    os.makedirs(OUT, exist_ok=True)
    sizes = [16, 32, 48, 64, 128, 256, 512, 1024]
    rendered = {}
    for s in sizes:
        rendered[s] = png(s, render(s))
        print("rendered", s)

    names = {
        32: "32x32.png",
        128: "128x128.png",
        256: "128x128@2x.png",
        512: "icon.png",
        1024: "icon-1024.png",
    }
    for s, name in names.items():
        with open(os.path.join(OUT, name), "wb") as f:
            f.write(rendered[s])

    with open(os.path.join(OUT, "icon.ico"), "wb") as f:
        f.write(ico([(s, rendered[s]) for s in [16, 32, 48, 64, 128, 256]]))

    icns_map = [
        (b"icp4", 16), (b"icp5", 32), (b"icp6", 64),
        (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024),
    ]
    with open(os.path.join(OUT, "icon.icns"), "wb") as f:
        f.write(icns([(k, rendered[s]) for k, s in icns_map]))

    # Tauri dev-window icon.
    with open(os.path.join(OUT, "icon-dev.png"), "wb") as f:
        f.write(rendered[128])

    print("wrote icons to", OUT)


if __name__ == "__main__":
    main()
