"""Generate app icons: a dark rounded tile with the four-bar waveform mark.

No third-party imaging deps — PNG is encoded by hand via zlib, and the .ico
wraps PNG payloads (supported since Vista). Rerun after any brand change.
"""
import struct, zlib, os, math

BG   = (0x1F, 0x23, 0x28, 255)   # cool near-black, matches --a-ink
BAR  = (0xFF, 0xFF, 0xFF, 255)
ACC  = (0xE8, 0x85, 0x0C, 255)   # amber accent, last bar only

# bar geometry as fractions of the canvas: (x, width, top, bottom)
BARS = [(0.255, 0.085, 0.44, 0.62),
        (0.395, 0.085, 0.32, 0.74),
        (0.535, 0.085, 0.39, 0.67),
        (0.675, 0.085, 0.26, 0.80)]


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
    for i, (bx, bw, bt, bb) in enumerate(BARS):
        col = ACC if i == len(BARS) - 1 else BAR
        x0, x1 = int(bx * size), int((bx + bw) * size)
        y0, y1 = int(bt * size), int(bb * size)
        for y in range(y0, y1):
            for x in range(x0, max(x1, x0 + 1)):
                if 0 <= x < size and 0 <= y < size:
                    px[y][x] = col
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
