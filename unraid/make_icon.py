#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F
"""
Generate a minimal 256×256 placeholder icon for the Unraid template.

The placeholder is intentionally simple — a solid GitHub-dark
background with a centred white "GB" monogram drawn as filled blocks
— so a contributor can render it in a few hundred lines of stdlib
Python without pulling in Pillow or cairo.  Replace it with a real
designed icon before submitting to Community Applications.

Run:
    python3 unraid/make_icon.py
"""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

WIDTH = 256
HEIGHT = 256
BG = (13, 17, 23, 255)      # GitHub dark "canvas-default"
FG = (240, 246, 252, 255)   # GitHub dark "fg-default"


def make_pixels() -> bytes:
    """Return an `RGBA` bytearray of the icon image, row-major."""
    pixels = bytearray()
    # Pre-compute the monogram mask so we don't branch per pixel.
    mask = _monogram_mask(WIDTH, HEIGHT)
    for y in range(HEIGHT):
        # PNG requires a filter byte (0 = None) at the start of every row.
        pixels.append(0)
        for x in range(WIDTH):
            r, g, b, a = FG if mask[y * WIDTH + x] else BG
            pixels.extend((r, g, b, a))
    return bytes(pixels)


def _monogram_mask(w: int, h: int) -> bytearray:
    """Build a binary mask of a "GB" monogram inside a soft-rounded square."""
    mask = bytearray(w * h)
    # The two letters live inside a 224×224 box, centred.
    pad = (w - 224) // 2
    # Block-pixel font: each row of the bitmap is a 14×14-cell glyph.
    # Letter 'G':
    g = [
        "..1111111111..",
        ".11111111111..",
        "111.........11",
        "111...........",
        "111...........",
        "111...........",
        "111...111111..",
        "111...1111111.",
        "111.......111.",
        "111.......111.",
        "111.......111.",
        "111.......111.",
        ".11111111111..",
        "..1111111111..",
    ]
    # Letter 'B':
    b = [
        "1111111111....",
        "11111111111...",
        "111.......111.",
        "111.......111.",
        "111.......111.",
        "111.......111.",
        "11111111111...",
        "11111111111...",
        "111.......111.",
        "111.......111.",
        "111.......111.",
        "111.......111.",
        "11111111111...",
        "1111111111....",
    ]
    # Render each cell of the glyph as a 14×14 pixel block.
    cell = 14
    rows = len(g)
    cols = len(g[0])
    glyph_w = cols * cell
    # Position the two letters: G on the left, B on the right.
    g_x0 = pad
    g_y0 = (h - rows * cell) // 2
    b_x0 = w - pad - glyph_w
    b_y0 = g_y0
    for cy in range(rows):
        for cx in range(cols):
            for letter, x0, y0 in ((g, g_x0, g_y0), (b, b_x0, b_y0)):
                if letter[cy][cx] != "1":
                    continue
                for py in range(cell):
                    for px in range(cell):
                        mask[(y0 + cy * cell + py) * w + x0 + cx * cell + px] = 1
    return mask


def write_png(path: Path, raw: bytes) -> None:
    """Encode raw RGBA rows (with per-row filter byte) as a PNG file."""
    def chunk(kind: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
        )

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = chunk(
        b"IHDR",
        struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 6, 0, 0, 0),
    )
    idat = chunk(b"IDAT", zlib.compress(raw, level=9))
    iend = chunk(b"IEND", b"")
    path.write_bytes(sig + ihdr + idat + iend)


if __name__ == "__main__":
    out = Path(__file__).with_name("icon.png")
    write_png(out, make_pixels())
    print(f"wrote {out} ({out.stat().st_size} bytes)")
