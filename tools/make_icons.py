"""Generates the Glance .ico files from the Lucide "scan-text" glyph.

Standard library only. Run from the repository root:

    python tools/make_icons.py

Writes:
    assets/glance.ico      app icon: light glyph on a graphite tile
    assets/tray-dark.ico   tray icon for a dark taskbar (white glyph)
    assets/tray-light.ico  tray icon for a light taskbar (dark glyph)
    assets/glance.png      256px app icon for the README
"""

import math
import struct
import sys
import zlib
from pathlib import Path

# Lucide "scan-text" (ISC license), 24x24 grid, stroke-width 2, round caps.
# Each entry: ("seg", x0, y0, x1, y1) or ("arc", cx, cy, r, start_deg, end_deg).
GLYPH = [
    ("seg", 3, 7, 3, 5), ("arc", 5, 5, 2, 180, 270), ("seg", 5, 3, 7, 3),
    ("seg", 17, 3, 19, 3), ("arc", 19, 5, 2, 270, 360), ("seg", 21, 5, 21, 7),
    ("seg", 21, 17, 21, 19), ("arc", 19, 19, 2, 0, 90), ("seg", 19, 21, 17, 21),
    ("seg", 7, 21, 5, 21), ("arc", 5, 19, 2, 90, 180), ("seg", 3, 19, 3, 17),
    ("seg", 7, 8, 15, 8),
    ("seg", 7, 12, 17, 12),
    ("seg", 7, 16, 13, 16),
]

SIZES = [16, 20, 24, 32, 40, 48, 64, 256]


def dist_segment(px, py, x0, y0, x1, y1):
    dx, dy = x1 - x0, y1 - y0
    t = ((px - x0) * dx + (py - y0) * dy) / (dx * dx + dy * dy)
    t = max(0.0, min(1.0, t))
    return math.hypot(px - (x0 + t * dx), py - (y0 + t * dy))


def dist_arc(px, py, cx, cy, r, a0, a1):
    # Angles in screen space (y down): 0 = +x, 90 = +y.
    ang = math.degrees(math.atan2(py - cy, px - cx)) % 360
    lo, hi = a0 % 360, a1 % 360 if a1 % 360 else 360
    if lo <= ang <= hi or (hi == 360 and ang == 0):
        return abs(math.hypot(px - cx, py - cy) - r)
    ends = [(cx + r * math.cos(math.radians(a)), cy + r * math.sin(math.radians(a))) for a in (a0, a1)]
    return min(math.hypot(px - ex, py - ey) for ex, ey in ends)


def glyph_coverage(gx, gy, half_width):
    d = min(
        dist_segment(gx, gy, *p[1:]) if p[0] == "seg" else dist_arc(gx, gy, *p[1:])
        for p in GLYPH
    )
    return d - half_width


def rounded_rect_sd(px, py, x0, y0, x1, y1, r):
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    hx, hy = (x1 - x0) / 2 - r, (y1 - y0) / 2 - r
    qx, qy = abs(px - cx) - hx, abs(py - cy) - hy
    outside = math.hypot(max(qx, 0), max(qy, 0))
    return outside + min(max(qx, qy), 0) - r


def blend(dst, src, a):
    return tuple(d * (1 - a) + s * a for d, s in zip(dst, src))


def render(size, kind):
    """Returns straight-alpha RGBA rows for one icon size."""
    rows = []
    if kind == "app":
        inset = size * 0.0
        tile_r = size * 0.22
        glyph_box = (size * 0.18, size * 0.18, size * 0.82, size * 0.82)
        glyph_rgb = (250, 250, 250)
    else:
        glyph_box = (0, 0, size, size)
        glyph_rgb = (255, 255, 255) if kind == "tray-dark" else (26, 26, 26)
    gx0, gy0, gx1, gy1 = glyph_box
    scale = (gx1 - gx0) / 24.0
    # Keep strokes at least ~1.4px at the small tray sizes.
    stroke = max(2.0, 1.4 / scale) if kind != "app" else 2.0
    for y in range(size):
        row = []
        for x in range(size):
            px, py = x + 0.5, y + 0.5
            color, alpha = (0.0, 0.0, 0.0), 0.0
            if kind == "app":
                sd = rounded_rect_sd(px, py, inset, inset, size - inset, size - inset, tile_r)
                tile_a = max(0.0, min(1.0, 0.5 - sd))
                if tile_a > 0:
                    # Iron tile with a faint top light and a half-pixel white rim.
                    shade = 36 + 10 * (1 - py / size)
                    color, alpha = (shade, shade, shade), tile_a
                    rim = max(0.0, 1.0 - abs(sd + 0.5)) * 0.18
                    color = blend(color, (255, 255, 255), rim)
            g = (px - gx0) / scale, (py - gy0) / scale
            sd_glyph = glyph_coverage(g[0], g[1], stroke / 2) * scale
            ga = max(0.0, min(1.0, 0.5 - sd_glyph))
            if ga > 0:
                if alpha > 0:
                    color = blend(color, glyph_rgb, ga)
                else:
                    color, alpha = glyph_rgb, ga
            row.append((round(color[0]), round(color[1]), round(color[2]), round(alpha * 255)))
        rows.append(row)
    return rows


def png_bytes(rows):
    size = len(rows)
    raw = b"".join(b"\x00" + bytes(c for px in row for c in px) for row in rows)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def dib_bytes(rows):
    size = len(rows)
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    pixels = b"".join(bytes((b, g, r, a)) for row in reversed(rows) for (r, g, b, a) in row)
    mask_stride = ((size + 31) // 32) * 4
    mask = b"\x00" * (mask_stride * size)
    return header + pixels + mask


def write_ico(path, kind):
    images = []
    for size in SIZES:
        rows = render(size, kind)
        images.append((size, png_bytes(rows) if size >= 256 else dib_bytes(rows)))
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for size, data in images:
        dim = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    out += b"".join(data for _, data in images)
    path.write_bytes(out)
    print(f"wrote {path} ({len(out)} bytes)")


def main():
    assets = Path(__file__).resolve().parent.parent / "assets"
    assets.mkdir(exist_ok=True)
    write_ico(assets / "glance.ico", "app")
    write_ico(assets / "tray-dark.ico", "tray-dark")
    write_ico(assets / "tray-light.ico", "tray-light")
    (assets / "glance.png").write_bytes(png_bytes(render(256, "app")))
    # `--preview DIR` also writes PNGs of the app and tray icons for eyeballing.
    if len(sys.argv) == 3 and sys.argv[1] == "--preview":
        preview = Path(sys.argv[2])
        preview.mkdir(parents=True, exist_ok=True)
        (preview / "app-256.png").write_bytes(png_bytes(render(256, "app")))
        (preview / "app-32.png").write_bytes(png_bytes(render(32, "app")))
        (preview / "tray-32.png").write_bytes(png_bytes(render(32, "tray-dark")))


if __name__ == "__main__":
    main()
