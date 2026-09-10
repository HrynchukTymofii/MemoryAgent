"""Generate app icons: a dark rounded tile carrying the four-cube mark.

Three cubes at the corners of a triangle, one in the middle, and a line between
every pair — the same figure `apps/desktop/src/components/Logo.tsx` draws, at
the same proportions, because the icon and the mark inside the window have to
be recognisably one thing.

No third-party imaging deps — polygons are rasterised below, PNG is encoded by
hand via zlib, and the .ico wraps PNG payloads (supported since Vista). Rerun
after any brand change.
"""
import struct, zlib, os, math

BG   = (0x1F, 0x23, 0x28)        # cool near-black, matches --a-ink
INK  = (0xFF, 0xFF, 0xFF)        # the three corner cubes
ACC  = (0xE8, 0x85, 0x0C)        # amber accent, the middle cube only

# Face shading, one light source: top brightest, left in shadow. The same three
# numbers as FACE in Logo.tsx.
FACE = (1.0, 0.66, 0.42)         # top, right, left
EDGE = 0.30                      # the six connecting lines

# Geometry in fractions of the canvas, so every size is the same picture. The
# triangle's centre sits below the middle of the tile: the mark hangs further
# below that centre than above it, and the tile is a square.
R      = 0.118                   # half a cube's height
CENTRE = [(0.5000, 0.2788),      # top corner
          (0.7555, 0.7213),      # right corner
          (0.2445, 0.7213),      # left corner
          (0.5000, 0.5738)]      # the middle
PAIRS  = [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)]
STROKE = 0.027                   # width of a connecting line

W, H = 0.866, 0.5                # the isometric projection


def cube_faces(cx, cy, r):
    """Top, right and left faces of an isometric cube, as polygons."""
    return [
        [(cx, cy - r), (cx + W * r, cy - H * r), (cx, cy), (cx - W * r, cy - H * r)],
        [(cx, cy), (cx + W * r, cy - H * r), (cx + W * r, cy + H * r), (cx, cy + r)],
        [(cx - W * r, cy - H * r), (cx, cy), (cx, cy + r), (cx - W * r, cy + H * r)],
    ]


def segment_quad(a, b, width):
    """A line as the rectangle it covers — everything downstream is polygons."""
    (x0, y0), (x1, y1) = a, b
    dx, dy = x1 - x0, y1 - y0
    n = math.hypot(dx, dy) or 1.0
    ox, oy = -dy / n * width / 2, dx / n * width / 2
    return [(x0 + ox, y0 + oy), (x1 + ox, y1 + oy), (x1 - ox, y1 - oy), (x0 - ox, y0 - oy)]


def shapes():
    """Every polygon in the mark, back to front, as (points, rgb, alpha)."""
    out = []
    for a, b in PAIRS:
        out.append((segment_quad(CENTRE[a], CENTRE[b], STROKE), INK, EDGE))
    # The middle cube last: it is what the lines converge on, so nothing may be
    # drawn over it.
    for i in (0, 1, 2, 3):
        colour = ACC if i == 3 else INK
        for face, alpha in zip(cube_faces(*CENTRE[i], R), FACE):
            out.append((face, colour, alpha))
    return out


# ------------------------------------------------------------- rasterising

SS = 4      # subsamples per pixel, vertically and horizontally


def _add_span(row, xa, xb, weight, size):
    """Add horizontal coverage from xa to xb (pixel units) into one row."""
    xa, xb = max(xa, 0.0), min(xb, float(size))
    if xb <= xa:
        return
    ia, ib = int(xa), min(int(xb), size - 1)
    if ia == ib:
        row[ia] += (xb - xa) * weight
        return
    row[ia] += (ia + 1 - xa) * weight
    for x in range(ia + 1, ib):
        row[x] += weight
    row[ib] += (xb - ib) * weight


def fill(px, poly, colour, alpha, size):
    """Scanline-fill a polygon over the image, antialiased and alpha-blended.

    Coverage is accumulated one pixel row at a time from SS subsample rows, so
    a shape costs its own area rather than the whole canvas, and no full-size
    buffer is held per shape. Blending is per shape rather than per polygon
    edge, which is why a cube's three faces are separate shapes: they meet
    exactly, and blending each in turn leaves no seam because each is opaque
    along the join.
    """
    pts = [(x * size, y * size) for x, y in poly]
    y0 = max(int(min(p[1] for p in pts)), 0)
    y1 = min(int(max(p[1] for p in pts)) + 1, size)
    edges = [(pts[i], pts[(i + 1) % len(pts)]) for i in range(len(pts))]

    for y in range(y0, y1):
        row = [0.0] * size
        touched = False
        for s in range(SS):
            yc = y + (s + 0.5) / SS
            xs = []
            for (ax, ay), (bx, by) in edges:
                if (ay <= yc < by) or (by <= yc < ay):
                    xs.append(ax + (yc - ay) / (by - ay) * (bx - ax))
            if not xs:
                continue
            xs.sort()
            for i in range(0, len(xs) - 1, 2):
                _add_span(row, xs[i], xs[i + 1], 1.0 / SS, size)
                touched = True
        if not touched:
            continue
        for x in range(size):
            a = row[x] * alpha
            if a <= 0.001:
                continue
            a = min(a, 1.0)
            r, g, b, oa = px[y][x]
            px[y][x] = (int(r + (colour[0] - r) * a),
                        int(g + (colour[1] - g) * a),
                        int(b + (colour[2] - b) * a),
                        max(oa, int(255 * a)) if oa < 255 else 255)


def render(size):
    px = [[(0, 0, 0, 0)] * size for _ in range(size)]
    r = size * 0.22                      # corner radius
    for y in range(size):
        for x in range(size):
            # rounded-rect coverage with a little antialiasing at the corners
            dx = max(r - x, 0, x - (size - 1 - r))
            dy = max(r - y, 0, y - (size - 1 - r))
            d = math.hypot(dx, dy)
            a = 1.0 if d <= r - 0.5 else max(0.0, min(1.0, r - d + 0.5))
            if a > 0:
                px[y][x] = (BG[0], BG[1], BG[2], int(255 * a))
    for poly, colour, alpha in shapes():
        fill(px, poly, colour, alpha, size)
    return px


def png_bytes(px):
    size = len(px)
    raw = b"".join(
        b"\x00" + b"".join(struct.pack("4B", *px[y][x]) for x in range(size))
        for y in range(size)
    )

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9))
            + chunk(b"IEND", b""))


def icns_bytes(types_to_png):
    """Pack PNGs into an .icns.

    Written by hand rather than shelled out to `iconutil`, because that only
    exists on macOS and this script has to keep producing every icon from
    whichever machine the brand changed on. The container is trivial: a magic,
    a total length, then typed chunks whose payload a modern macOS reads as
    PNG directly.
    """
    body = b""
    for tag, data in types_to_png:
        body += tag + struct.pack(">I", len(data) + 8) + data
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def ico_bytes(sizes_to_png):
    n = len(sizes_to_png)
    header = struct.pack("<HHH", 0, 1, n)
    entries, blobs, offset = b"", b"", 6 + 16 * n
    for size, data in sizes_to_png:
        entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0,
                               1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    return header + entries + blobs


out = os.path.join(os.path.dirname(__file__), "..", "apps", "desktop", "src-tauri", "icons")
os.makedirs(out, exist_ok=True)

pngs = {}
for s in (32, 64, 128, 256, 512, 1024):
    pngs[s] = png_bytes(render(s))

open(os.path.join(out, "32x32.png"), "wb").write(pngs[32])
open(os.path.join(out, "128x128.png"), "wb").write(pngs[128])
open(os.path.join(out, "128x128@2x.png"), "wb").write(pngs[256])
open(os.path.join(out, "icon.png"), "wb").write(pngs[1024])
open(os.path.join(out, "icon.ico"), "wb").write(
    ico_bytes([(32, pngs[32]), (64, pngs[64]), (128, pngs[128])])
)
# macOS reads the Dock and Finder icon from here and nowhere else. The Retina
# variants are separate entries rather than scaled copies, because macOS picks
# by type and falls back to a blurry upscale of the largest it finds.
open(os.path.join(out, "icon.icns"), "wb").write(
    icns_bytes([
        (b"ic11", pngs[32]),    # 16pt @2x
        (b"ic12", pngs[64]),    # 32pt @2x
        (b"ic07", pngs[128]),
        (b"ic13", pngs[256]),   # 128pt @2x
        (b"ic08", pngs[256]),
        (b"ic14", pngs[512]),   # 256pt @2x
        (b"ic09", pngs[512]),
        (b"ic10", pngs[1024]),  # 512pt @2x
    ])
)
print("wrote icons to", os.path.normpath(out))
for f in sorted(os.listdir(out)):
    print("  ", f, os.path.getsize(os.path.join(out, f)), "bytes")
