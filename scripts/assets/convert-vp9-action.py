"""Convert a constant-rate transparent VP9 action to v1 material pages."""

import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import subprocess
import time
import zlib

from pack_png import png

PROFILE = "alpha_basic"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ffmpeg", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fps", type=int, choices=(24, 30), required=True)
    parser.add_argument("--size", type=int, default=320)
    parser.add_argument("--batch", type=int, default=4)
    parser.add_argument("--png-only", action="store_true")
    args = parser.parse_args()
    if not args.png_only:
        import ispc_texcomp
    size, batch = args.size, args.batch
    if (size < 4 or size % 4 or batch < 1 or size * batch > 4096
            or size * size * batch * 5 > 6 * 1024**2):
        parser.error("page layout exceeds alignment, dimensions or playback budget")
    args.output.mkdir(parents=True, exist_ok=False)
    command = [str(args.ffmpeg.resolve()), "-hide_banner", "-loglevel", "error", "-nostdin",
               "-threads", "2", "-c:v", "libvpx-vp9", "-i", str(args.source.resolve()),
               "-map", "0:v:0", "-an", "-sn", "-dn", "-filter_threads", "1",
               "-vf", f"scale={size}:{size}:flags=bicubic,format=rgba",
               "-fps_mode", "passthrough", "-f", "rawvideo", "-threads", "1", "pipe:1"]

    def write(relative, data):
        path = args.output / relative
        path.write_bytes(data)
        return {"path": relative, "sha256": hashlib.sha256(data).hexdigest()}

    with args.source.open("rb") as source:
        source_sha256 = hashlib.file_digest(source, "sha256").hexdigest()
    manifest = {
        "schema_version": 1, "pack_id": args.output.name, "pack_version": "0.1.0",
        "color_space": "sRGB", "alpha": "straight",
        "source_sha256": [source_sha256],
        "tools": {"ffmpeg": subprocess.check_output([command[0], "-version"], text=True).splitlines()[0],
                  "filter": command[command.index("-vf") + 1],
                  "zlib": f"{zlib.ZLIB_RUNTIME_VERSION} level 6"},
        "pages": [], "actions": [], "resources": [],
    }
    frames = []
    totals = {"png": 0}
    if not args.png_only:
        totals.update(rgba8=0, bc7=0)
        manifest["tools"]["ispc_texcomp"] = f"{importlib.metadata.version('ispc_texcomp')} {PROFILE}"
        settings = ispc_texcomp.BC7EncSettings.from_profile(PROFILE)
    stride = size * 4
    frame_bytes = stride * size
    has_alpha = False
    started = time.perf_counter()
    with subprocess.Popen(command, stdout=subprocess.PIPE) as decoder:
        try:
            while data := decoder.stdout.read(frame_bytes * batch):
                count, remainder = divmod(len(data), frame_bytes)
                if remainder:
                    raise ValueError("truncated decoded frame")
                has_alpha |= min(data[3::4]) < 255
                page_id = f"page-{len(manifest['pages']):03}"
                width = count * size
                rgba = b"".join(data[i + y:i + y + stride] for y in range(0, frame_bytes, stride)
                                for i in range(0, len(data), frame_bytes))
                payload = png(rgba, width, size)
                if len(payload) > 3 * 1024**2:
                    raise ValueError("PNG exceeds playback staging budget")
                page = {"id": page_id, "size": [width, size], "png": write(f"{page_id}.png", payload)}
                totals["png"] += len(payload)
                if not args.png_only:
                    surface = ispc_texcomp.RGBASurface(rgba, width, size, width * 4)
                    bc7 = ispc_texcomp.compress_blocks_bc7(surface, settings)
                    for variant, data in (("rgba8", rgba), ("bc7", bc7)):
                        page[variant] = write(f"{page_id}.{variant}", data)
                        totals[variant] += len(data)
                manifest["pages"].append(page)
                for slot in range(count):
                    i = len(frames)
                    frames.append({"page": page_id, "rect": [slot * size, 0, size, size],
                                   "duration_us": ((i + 1) * 1_000_000 // args.fps
                                                   - i * 1_000_000 // args.fps)})
        except BaseException:
            decoder.kill()
            raise
        if decoder.wait() != 0 or not frames or not has_alpha:
            raise ValueError("FFmpeg failed or decoded no transparent frames")
    manifest["actions"].append({"id": "reference", "logical_size": [size, size], "looping": True,
                                "anchor": [0.5, 1], "offset": [0, 0], "scale": 1,
                                "directions": [], "movement_us": None, "frames": frames})
    descriptor = write("manifest.json", (json.dumps(manifest, indent=2) + "\n").encode())
    print(json.dumps({"source_sha256": manifest["source_sha256"][0], "frames": len(frames),
                      "manifest_sha256": descriptor["sha256"], "page_bytes": totals,
                      "elapsed_s": round(time.perf_counter() - started, 3)}))


if __name__ == "__main__":
    main()
