"""Generate original RGBA PNG fixtures with only the Python standard library."""

import hashlib
import json
from pathlib import Path
import struct
import zlib


SIZE = 128
OUTPUT = Path(__file__).resolve().parents[3] / "artifacts" / "validation" / "p0" / "fixtures"


def chunk(kind, data):
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data))
    )


def png(pixel):
    rows = bytearray()
    for y in range(SIZE):
        rows.append(0)  # PNG filter None
        for x in range(SIZE):
            rows.extend(pixel(x, y))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
        + chunk(b"sRGB", b"\x00")
        + chunk(b"IDAT", zlib.compress(rows, level=0))
        + chunk(b"IEND", b"")
    )


def ramp(x, y):
    value = x * 255 // (SIZE - 1)
    palette = [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 255)]
    if y < 64:
        return (*palette[y // 16], value)
    return (value, value, value, 255)


def moving_disc(center_x, x, y):
    distance_squared = (x - center_x) ** 2 + (y - 64) ** 2
    alpha = max(0, min(255, (576 - distance_squared) * 255 // 92))
    return (64, 192, 255, alpha)


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    hashes = {}

    def write_sample(name, data):
        (OUTPUT / name).write_bytes(data)
        hashes[name] = hashlib.sha256(data).hexdigest()

    write_sample("alpha-color.png", png(ramp))
    for frame in range(30):
        center = 25 + 77 * min(frame, 29 - frame) // 14
        write_sample(f"move-{frame:02}.png", png(lambda x, y: moving_disc(center, x, y)))

    manifest = {
        "size": [SIZE, SIZE],
        "alpha": "straight",
        "color_space": "sRGB",
        "sequence": {"pattern": "move-{frame:02}.png", "frames": 30, "fps": 30},
        "sha256": hashes,
    }
    encoded = json.dumps(manifest, indent=2).encode("ascii") + b"\n"
    (OUTPUT / "manifest.json").write_bytes(encoded)
    print(f"{OUTPUT / 'manifest.json'} SHA-256 {hashlib.sha256(encoded).hexdigest()}")


if __name__ == "__main__":
    main()
