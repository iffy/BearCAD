#!/usr/bin/env python3
"""Print the callout marker positions for a captured Context-pane screenshot.

The docs annotate pane shots with `<PaneCallouts>` (docs-site/src/components), which
places each numbered marker at a percentage of the image. Eyeballing those numbers
row by row is tedious and drifts whenever a pane gains a control, so this reads them
off the PNG instead: it finds the horizontal bands that actually contain something,
and prints the centre of each as a percentage of the image height.

Usage:
    scripts/pane-callout-rows.py docs-site/static/img/screenshots/pane-move-snap.png

Every band is reported, including the "Context" heading and the tool name — pick the
ones that are controls. `--x` sets the marker column (default 37, the gap between a
row's label and its input, which is clear in every pane).
"""

import argparse
import struct
import sys
import zlib


def read_png(path):
    """Decode a non-interlaced 8-bit RGB/RGBA PNG into (width, height, rows-of-pixels)."""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path}: not a PNG")

    pos, idat, width, height, channels = 8, bytearray(), None, None, None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            width, height, depth, color = struct.unpack(">IIBB", body[:10])
            if depth != 8 or color not in (2, 6) or body[12] != 0:
                sys.exit(f"{path}: need an 8-bit non-interlaced RGB/RGBA PNG")
            channels = 3 if color == 2 else 4
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
        pos += 12 + length

    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    rows, prev = [], bytearray(stride)
    pos = 0
    for _ in range(height):
        filt = raw[pos]
        line = bytearray(raw[pos + 1 : pos + 1 + stride])
        pos += 1 + stride
        for i in range(stride):
            a = line[i - channels] if i >= channels else 0
            b = prev[i]
            c = prev[i - channels] if i >= channels else 0
            if filt == 1:
                line[i] = (line[i] + a) & 0xFF
            elif filt == 2:
                line[i] = (line[i] + b) & 0xFF
            elif filt == 3:
                line[i] = (line[i] + (a + b) // 2) & 0xFF
            elif filt == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 0xFF
        rows.append(line)
        prev = line
    return width, height, channels, rows


def row_bands(width, height, channels, rows, tolerance=10):
    """Rows differing from the pane's background, grouped into contiguous bands."""
    # Ignore the pane's own edges — the border runs down every row, so counting it
    # would make the whole image one band.
    margin = 12
    lo, hi = margin, max(margin + 1, width - margin)

    # The background is whatever colour the pane's left margin is — a column that no
    # control reaches into.
    bg = tuple(rows[height // 2][margin // 2 * channels : margin // 2 * channels + 3])

    interesting = []
    for y in range(height):
        line = rows[y]
        differing = 0
        for x in range(lo, hi):
            px = line[x * channels : x * channels + 3]
            if any(abs(px[i] - bg[i]) > tolerance for i in range(3)):
                differing += 1
                if differing >= 4:  # a few stray pixels aren't a control
                    break
        interesting.append(differing >= 4)

    bands, start = [], None
    for y, hit in enumerate(interesting):
        if hit and start is None:
            start = y
        elif not hit and start is not None:
            bands.append((start, y - 1))
            start = None
    if start is not None:
        bands.append((start, height - 1))
    return bands


def write_preview(path, width, height, channels, rows, marks):
    """Write a copy of the shot with a dot on each marker position, to eyeball placement."""
    radius = max(6, width // 40)
    out = [bytearray(row) for row in rows]
    for cx, cy in marks:
        for dy in range(-radius, radius + 1):
            y = int(cy) + dy
            if not 0 <= y < height:
                continue
            for dx in range(-radius, radius + 1):
                x = int(cx) + dx
                if not 0 <= x < width or dx * dx + dy * dy > radius * radius:
                    continue
                i = x * channels
                out[y][i : i + 3] = bytes((255, 64, 160))

    raw = bytearray()
    for row in out:
        raw.append(0)  # filter: none
        raw += row
    color = 2 if channels == 3 else 6

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, color, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 6))
    png += chunk(b"IEND", b"")
    open(path, "wb").write(png)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("png")
    ap.add_argument("--x", type=float, default=37.0, help="marker column, %% of width (default 37)")
    ap.add_argument("--min-height", type=int, default=6, help="ignore bands thinner than this (px)")
    ap.add_argument("--preview", help="write a copy with the markers drawn on, to check placement")
    args = ap.parse_args()

    width, height, channels, rows = read_png(args.png)
    bands = [b for b in row_bands(width, height, channels, rows) if b[1] - b[0] + 1 >= args.min_height]

    print(f"{args.png}: {width}x{height}, {len(bands)} bands")
    centres = []
    for i, (top, bottom) in enumerate(bands):
        centre = (top + bottom) / 2
        centres.append(centre)
        print(f"  band {i}: y {top}-{bottom}  ->  {{x: {args.x:g}, y: {centre / height * 100:.0f}}}")

    if args.preview:
        marks = [(width * args.x / 100, c) for c in centres]
        write_preview(args.preview, width, height, channels, rows, marks)
        print(f"preview -> {args.preview}")


if __name__ == "__main__":
    main()
