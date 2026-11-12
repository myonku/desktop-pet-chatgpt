"""Generate self-contained PNG packs for P2 behavior and fallback validation."""

import argparse
import hashlib
import json
from pathlib import Path
import runpy
import struct
import zlib

ROOT = Path(__file__).resolve()
CHUNK = runpy.run_path(str(ROOT.parents[1] / "p0" / "generate-p0-fixtures.py"))["chunk"]
SIZE = 64
ACTIONS = (
    ("interact", (241, 160, 49), False, (), None),
    ("peek-right", (240, 117, 171), True, (), None),
    ("move-left", (135, 214, 106), False, ("left",), (250_000, 950_000)),
    ("edge-left", (134, 116, 232), True, (), None),
    ("idle", (87, 167, 235), True, (), None),
    ("move-right", (83, 202, 175), False, ("right",), (250_000, 950_000)),
    ("peek-left", (220, 133, 241), True, (), None),
    ("edge-right", (151, 130, 249), True, (), None),
    ("drag", (247, 218, 113), True, (), None),
    ("land", (245, 126, 107), False, (), None),
)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def frame_rows(name, color, phase):
    center = 24 if name.endswith("left") else 40 if name.endswith("right") else 32
    y_center = 32 + (0, -3, -5, -2, 0, 2)[phase]
    rows = []
    for y in range(SIZE):
        row = bytearray()
        for x in range(SIZE):
            distance = (x - center)**2 + (y - y_center)**2
            facing_x = center - 7 if name.endswith("left") else center + 7
            if (x - facing_x)**2 + (y - y_center + 4)**2 < 10:
                row.extend((18, 22, 37, 255))
            elif distance <= 17**2:
                row.extend((*color, 255 if distance <= 15**2 else 120))
            else:
                row.extend((0, 0, 0, 0))
        rows.append(row)
    return rows


def build(root, sparse):
    root.mkdir(parents=True, exist_ok=True)
    pages, actions, hashes = [], [], []
    for name, color, looping, directions, movement in ACTIONS:
        if sparse and name != "idle":
            continue
        frames = []
        durations = (120_000, 180_000, 200_000, 250_000, 230_000, 220_000)
        for batch in range(2):
            page_id = f"{name}-{batch}"
            images = [frame_rows(name, color, index) for index in range(3 * batch, 3 * batch + 3)]
            raw = b"".join(b"\x00" + b"".join(row) for row in zip(*images))
            png = (b"\x89PNG\r\n\x1a\n"
                   + CHUNK(b"IHDR", struct.pack(">IIBBBBB", SIZE * 3, SIZE, 8, 6, 0, 0, 0))
                   + CHUNK(b"sRGB", b"\0")
                   + CHUNK(b"IDAT", zlib.compress(raw, level=6))
                   + CHUNK(b"IEND", b""))
            relative = f"{page_id}.png"
            (root / relative).write_bytes(png)
            hashes.append(sha(raw))
            pages.append({"id": page_id, "size": [SIZE * 3, SIZE],
                          "png": {"path": relative, "sha256": sha(png)}})
            frames.extend({"page": page_id, "rect": [slot * SIZE, 0, SIZE, SIZE],
                           "duration_us": durations[3 * batch + slot]} for slot in range(3))
        actions.append({
            "id": name, "looping": looping, "logical_size": [SIZE, SIZE],
            "anchor": [0.5, 1.0], "scale": 0.75 if name == "interact" else 1.0,
            "offset": [0.125, -0.25] if name == "interact" else [0.0, 0.0],
            "directions": directions, "movement_us": movement, "frames": frames,
        })
    manifest = {"schema_version": 1, "pack_id": root.name, "pack_version": "0.1.0",
                "color_space": "sRGB", "alpha": "straight", "source_sha256": hashes,
                "tools": {"generator": f"sha256:{sha(ROOT.read_bytes())}",
                          "zlib": zlib.ZLIB_RUNTIME_VERSION},
                "pages": pages, "actions": actions, "resources": []}
    data = (json.dumps(manifest, indent=2) + "\n").encode()
    (root / "manifest.json").write_bytes(data)
    print(f"{root}  {len(actions)} actions  {len(pages)} pages  {sha(data)}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT.parents[3] / "artifacts/validation/p2/fixtures")
    args = parser.parse_args()
    build(args.output / "p2-behavior", sparse=False)
    build(args.output / "p2-idle-only", sparse=True)
