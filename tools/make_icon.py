"""Erzeugt assets/storage-map.ico: eine kleine Speicherkarte. Nur Standardbibliothek."""
import struct
import zlib
from pathlib import Path

BG = (20, 20, 18, 255)
TILES = [
    # x, y, b, h (Anteile), Farbe
    (0.125, 0.125, 0.4375, 0.75, (200, 241, 53, 255)),
    (0.625, 0.125, 0.25, 0.3125, (57, 135, 229, 255)),
    (0.625, 0.5, 0.25, 0.1875, (217, 89, 38, 255)),
    (0.625, 0.75, 0.25, 0.125, (137, 135, 129, 255)),
]


def render(size):
    radius = max(1, round(size * 0.14))
    rows = []
    for y in range(size):
        row = bytearray([0])  # PNG-Filter "keiner"
        for x in range(size):
            dx = max(radius - x, x - (size - 1 - radius), 0)
            dy = max(radius - y, y - (size - 1 - radius), 0)
            color = BG if dx * dx + dy * dy <= radius * radius else (0, 0, 0, 0)
            for tx, ty, tw, th, c in TILES:
                if tx * size <= x < (tx + tw) * size and ty * size <= y < (ty + th) * size:
                    color = c
            row.extend(color)
        rows.append(bytes(row))
    return b"".join(rows)


def png(size):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(render(size), 9)) + chunk(b"IEND", b"")


def main():
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [png(s) for s in sizes]
    out = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    for s, img in zip(sizes, images):
        dim = 0 if s == 256 else s
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(img), offset)
        offset += len(img)
    out += b"".join(images)
    target = Path(__file__).resolve().parent.parent / "assets" / "storage-map.ico"
    target.parent.mkdir(exist_ok=True)
    target.write_bytes(out)
    print(f"{target} ({len(out)} Bytes)")


if __name__ == "__main__":
    main()
