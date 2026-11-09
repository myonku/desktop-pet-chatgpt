"""Generate two original local packs and compare CPU-decoded BC7 to RGBA8."""

import hashlib
import importlib.metadata
import io
import json
import math
from pathlib import Path
import runpy
import struct
import time
import wave
import zlib

import ispc_texcomp
import texture2ddecoder


ROOT = Path(__file__).resolve().parent
OUTPUT = ROOT.parent / "target" / "p1b-fixtures"
P0 = runpy.run_path(str(ROOT / "generate-p0-fixtures.py"))
SIZE = P0["SIZE"]
PROFILE = "alpha_basic"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def pixels(pixel):
    return bytes(channel for y in range(SIZE) for x in range(SIZE) for channel in pixel(x, y))


def png(rgba, width, height):
    stride = width * 4
    rows = b"".join(b"\0" + rgba[start:start + stride] for start in range(0, len(rgba), stride))
    chunk = P0["chunk"]
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"sRGB", b"\0")
        + chunk(b"IDAT", zlib.compress(rows, level=6))
        + chunk(b"IEND", b"")
    )


def tone(frequency):
    out = io.BytesIO()
    with wave.open(out, "wb") as audio:
        audio.setparams((1, 2, 8000, 0, "NONE", "not compressed"))
        audio.writeframes(b"".join(
            struct.pack("<h", round(3000 * (1 - i / 640) * math.sin(2 * math.pi * frequency * i / 8000)))
            for i in range(640)
        ))
    return out.getvalue()


def build(pack_id, prefix, batch, swapped):
    root = OUTPUT / pack_id
    started = time.perf_counter()
    byte_counts = dict.fromkeys(("png", "rgba8", "bc7"), 0)
    seconds = dict.fromkeys(("png", "bc7", "bc7_decode"), 0.0)
    max_error = {"rgb": 0, "alpha": 0}
    pages = []

    def write(relative, data):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return {"path": relative, "sha256": sha(data)}

    def page(page_id, images):
        width = SIZE * len(images)
        stride = SIZE * 4
        rgba = b"".join(image[y:y + stride] for y in range(0, stride * SIZE, stride) for image in images)
        start = time.perf_counter()
        encoded_png = png(rgba, width, SIZE)
        seconds["png"] += time.perf_counter() - start
        start = time.perf_counter()
        surface = ispc_texcomp.RGBASurface(rgba, width, SIZE, width * 4)
        bc7 = ispc_texcomp.compress_blocks_bc7(surface, ispc_texcomp.BC7EncSettings.from_profile(PROFILE))
        seconds["bc7"] += time.perf_counter() - start
        if len(bc7) != width * SIZE:
            raise ValueError("unexpected BC7 length")
        start = time.perf_counter()
        decoded = texture2ddecoder.decode_bc7(bc7, width, SIZE)
        seconds["bc7_decode"] += time.perf_counter() - start
        if len(decoded) != len(rgba):
            raise ValueError("unexpected BC7 decode length")
        for i in range(0, len(rgba), 4):
            max_error["rgb"] = max(max_error["rgb"], *(abs(rgba[i + c] - decoded[i + 2 - c]) for c in range(3)))
            max_error["alpha"] = max(max_error["alpha"], abs(rgba[i + 3] - decoded[i + 3]))
        item = {"id": page_id, "size": [width, SIZE]}
        for ext, data in (("png", encoded_png), ("rgba8", rgba), ("bc7", bc7)):
            item[ext] = write(f"{prefix}/{page_id}.{ext}", data)
            byte_counts[ext] += len(data)
        pages.append(item)

    def artwork(pixel):
        if swapped:
            return pixels(lambda x, y: swap(pixel(x, y)))
        return pixels(pixel)

    poster = artwork(P0["ramp"])
    movement = [
        artwork(lambda x, y: P0["moving_disc"](25 + 77 * min(i, 29 - i) // 14, x, y))
        for i in range(30)
    ]
    page("poster", [poster])
    frames = []
    for start in range(0, len(movement), batch):
        page_id = f"move-{start:02}"
        images = movement[start:start + batch]
        page(page_id, images)
        for j in range(len(images)):
            i = start + j
            frames.append({
                "page": page_id,
                "rect": [j * SIZE, 0, SIZE, SIZE],
                "duration_us": (i + 1) * 1_000_000 // 30 - i * 1_000_000 // 30,
            })
    audio = tone(660 if swapped else 440)
    label = ("暮色桌宠" if swapped else "晨光桌宠").encode("utf-8")
    client_label = ("ChatGPT · 暮色" if swapped else "ChatGPT · 晨光").encode("utf-8")
    auxiliary = "content/labels" if swapped else "extras"
    manifest = {
        "schema_version": 1,
        "pack_id": pack_id,
        "pack_version": "0.1.0",
        "color_space": "sRGB",
        "alpha": "straight",
        "source_sha256": [sha(data) for data in [poster, *movement, audio, label, client_label]],
        "tools": {
            "generator": f"sha256:{sha(Path(__file__).read_bytes())}",
            "zlib": f"{zlib.ZLIB_RUNTIME_VERSION} level 6",
            "ispc_texcomp": f"{importlib.metadata.version('ispc_texcomp')} {PROFILE}",
            "texture2ddecoder": importlib.metadata.version("texture2ddecoder"),
        },
        "pages": pages,
        "actions": [{
            "id": "move",
            "logical_size": [SIZE, SIZE],
            "looping": True,
            "anchor": [0.5, 1.0],
            "offset": [0.0, 0.0],
            "scale": 1.0,
            "directions": ["left", "right"],
            "movement_us": [0, 1_000_000],
            "frames": frames,
        }],
        "resources": [
            {"key": "character.poster", "content": {"kind": "image", "page": "poster"}},
            {"key": "interaction.click",
             "content": {"kind": "audio", "file": write(f"{auxiliary}/click.wav", audio)}},
            {"key": "character.name",
             "content": {"kind": "text", "file": write(f"{auxiliary}/name.txt", label)}},
            {"key": "character.name", "client_id": "chatgpt",
             "content": {"kind": "text", "file": write(f"{auxiliary}/chatgpt.txt", client_label)}},
        ],
    }
    descriptor = write("manifest.json", (json.dumps(manifest, indent=2) + "\n").encode())
    print(json.dumps({
        "pack": pack_id, "pages": len(pages), "frames": len(frames),
        "manifest_sha256": descriptor["sha256"], "page_bytes": byte_counts,
        "codec_seconds": seconds, "total_seconds": time.perf_counter() - started,
        "max_channel_error": max_error,
    }))


def swap(pixel):
    red, green, blue, alpha = pixel
    return blue, green, red, alpha


if __name__ == "__main__":
    build("dawn", "media", 4, False)
    build("dusk", "content/textures", 5, True)
