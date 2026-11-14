"""Build a v1 PNG candidate from the white-dragon source manifest."""

import argparse
import hashlib
import importlib.metadata
from itertools import batched
import json
from pathlib import Path
import re
import subprocess
import struct
import tempfile
import time
import wave
import zlib

import cv2
from PIL import Image, PngImagePlugin

from .__main__ import file_hash, upscale
from .frames import clear_hidden_rgb, optimize_frame, suppress_alpha_noise, suppress_contour_matte


BOUND_ACTIONS = {"01": "interact", "landing": "land", "move02left": "move-left", "move02": "move-right"}
PROFILES = ("resize", "alpha", "despeckle", "contour", "denoise", "gpu")
MIB = 1024**2


def source_actions(manifest, root):
    actions = {}
    files = set()
    for group, value in manifest.items():
        for item in value if isinstance(value, list) else [value]:
            if not isinstance(item, dict) or not item.get("playbackFile"):
                continue
            source_id = item["id"]
            action_id = BOUND_ACTIONS.get(source_id, f"variant-{source_id}" if group == "variants" else source_id)
            source = (root / item["playbackFile"]).resolve(strict=True)
            if (not re.fullmatch(r"[a-z0-9-]+", action_id) or action_id in actions
                    or not source.is_relative_to(root) or source.suffix.lower() != ".webm"
                    or item.get("renderAnchor", "bottom-center") != "bottom-center"):
                raise ValueError(f"invalid source action: {source_id}")
            files.add(source)
            actions[action_id] = (group, item, source)
    if not {"idle", "drag", *BOUND_ACTIONS.values()} <= actions.keys():
        raise ValueError("missing a core white-dragon action")
    if files != {path.resolve() for path in root.rglob("*.webm")}:
        raise ValueError("source manifest and on-disk WebM inventory differ")
    return actions


def decoded_frames(ffmpeg, source, size):
    command = [str(ffmpeg), "-hide_banner", "-loglevel", "error", "-nostdin", "-threads", "2",
               "-c:v", "libvpx-vp9", "-i", str(source), "-map", "0:v:0", "-an", "-sn", "-dn",
               "-filter_threads", "1", "-vf", f"scale={size}:{size}:flags=bicubic,format=rgba",
               "-fps_mode", "passthrough", "-f", "rawvideo", "-threads", "1", "pipe:1"]
    with subprocess.Popen(command, stdout=subprocess.PIPE) as decoder:
        try:
            while data := decoder.stdout.read(size * size * 4):
                yield Image.frombytes("RGBA", (size, size), data)
            if decoder.wait():
                raise ValueError(f"FFmpeg failed decoding {source.name}")
        finally:
            if decoder.poll() is None:
                decoder.kill()


def payload(root, path):
    return {"path": path.relative_to(root).as_posix(), "sha256": file_hash(path)}


def save_png(image, path):
    info = PngImagePlugin.PngInfo()
    info.add(b"sRGB", b"\0")
    image.save(path, pnginfo=info, compress_level=6)
    if path.stat().st_size > 3 * MIB:
        raise ValueError(f"PNG exceeds playback staging budget: {path.name}")


def clean_sequence(frames, count, looping, protected, report, contour):
    """Compare adjacent original masks, even after writing a cleaned frame."""
    def read(index):
        with Image.open(frames / f"{index:06}.png") as image:
            return image.copy()

    first = read(0)
    previous = read(count - 1) if looping and count > 1 else None
    current = first
    for index in range(count):
        following = read(index + 1) if index + 1 < count else first if looping and count > 1 else None
        neighbors = [frame for frame in (previous, following) if frame is not None]
        cleaned, removed = suppress_alpha_noise(current, neighbors, protected)
        report[index]["transient_alpha_pixels_cleared"] = removed
        if contour:
            cleaned, removed = suppress_contour_matte(cleaned, protected)
            report[index]["contour_alpha_pixels_cleared"] = removed
        cleaned.save(frames / f"{index:06}.png", compress_level=1)
        previous, current = current, following


def build_action(args, root, action_id, item, source):
    started = time.perf_counter()
    # Source v1 omits fps for its two number-03 flight clips.
    fps = item.get("fps", 30 if item.get("number") == "03" and item.get("movementAngleRange") else 24)
    direction = item.get("sourceFacing")
    action = {"id": action_id, "logical_size": [args.size, args.size],
              "looping": bool(item.get("loop") or action_id in ("idle", "drag")),
              "anchor": [0.5, 1.0], "offset": item.get("renderOffset", [0, 0]),
              "scale": item.get("renderScale", 1), "directions": [direction] if direction else [],
              "movement_us": [item["moveStartMs"] * 1000, item["moveStopMs"] * 1000] if direction else None,
              "frames": []}
    folder = root / action_id
    folder.mkdir()
    protected = args.protect.get(action_id, [])
    if any(x + w > args.size or y + h > args.size for x, y, w, h in protected):
        raise ValueError(f"protected rectangle lies outside {action_id}'s input canvas")
    report = {"action_id": action_id, "source_id": item["id"], "name": item.get("name", action_id),
              "playback_file": item["playbackFile"], "sha256": file_hash(source), "fps": fps,
              "protected": protected, "frames": [], "upscale": None}
    bounds = []
    has_alpha = False
    pages = []
    with tempfile.TemporaryDirectory(prefix=".frames-", dir=root) as temporary:
        workspace = Path(temporary)
        frames = workspace / "frames"
        frames.mkdir()
        for index, frame in enumerate(decoded_frames(args.ffmpeg, source, args.size)):
            has_alpha |= frame.getchannel("A").getextrema()[0] < 255
            result, removed = optimize_frame(frame, keep_alpha=args.profile == "resize",
                                             protected=protected, denoise=args.profile == "denoise")
            result.save(frames / f"{index:06}.png", compress_level=1)
            report["frames"].append({"input_rgba_sha256": hashlib.sha256(frame.tobytes()).hexdigest(),
                                     "alpha_pixels_cleared": removed})
        count = len(report["frames"])
        duration = count * 1_000_000 // fps
        if not has_alpha or not count or abs(duration - item["durationMs"] * 1000) > 1_000_000 / fps:
            raise ValueError(f"invalid transparency or source timing for {action_id}: {count} frames at {fps} fps")
        if args.profile in ("despeckle", "contour"):
            clean_sequence(frames, count, action["looping"], protected, report["frames"],
                           contour=args.profile == "contour")
        if args.profile == "gpu":
            report["upscale"] = upscale(frames, args.gpu_exe, args.models, workspace, args.tile,
                                        args.gpu_id, resize_back=True)
        for page_index, paths in enumerate(batched(sorted(frames.glob("*.png")), args.batch)):
            page_id = f"{action_id}-{page_index:03}"
            strip = Image.new("RGBA", (len(paths) * args.size, args.size))
            for slot, path in enumerate(paths):
                index = len(action["frames"])
                with Image.open(path) as image:
                    result = clear_hidden_rgb(image)
                report["frames"][index]["output_rgba_sha256"] = hashlib.sha256(result.tobytes()).hexdigest()
                if action_id == "idle":
                    box = result.getchannel("A").point(lambda a: 255 if a >= 128 else 0).getbbox()
                    if box:
                        bounds.append(box)
                    if index == 0:
                        save_png(result, root / "poster.png")
                x = slot * args.size
                strip.paste(result, (x, 0))
                action["frames"].append({"page": page_id, "rect": [x, 0, args.size, args.size],
                                         "duration_us": (index + 1) * 1_000_000 // fps - index * 1_000_000 // fps})
            path = folder / f"{page_id}.png"
            save_png(strip, path)
            pages.append({"id": page_id, "size": list(strip.size), "png": payload(root, path)})
    report.update(frame_count=count, duration_us=duration, elapsed_s=round(time.perf_counter() - started, 2))
    print(f"{args.profile}: {action_id} ({count} frames at {fps} fps, {report['elapsed_s']} s)", flush=True)
    return action, pages, bounds, report


def derive_poses(root, idle, bounds, batch):
    """Bake the rotated sprite once per side and share it between exposure poses."""
    if not bounds:
        raise ValueError("no visible idle bounds for edge poses")
    size = idle["logical_size"][0]
    from math import cos, radians, sin

    x0, y0 = (min(box[axis] for box in bounds) for axis in (0, 1))
    x1, y1 = (max(box[axis] for box in bounds) for axis in (2, 3))
    center = size / 2
    for side, angle in (("left", 45), ("right", -45)):
        c, s = cos(radians(angle)), sin(radians(angle))
        xs = [center + c * (x - center) - s * (y - center) for x in (x0, x1) for y in (y0, y1)]
        left, right = min(xs), max(xs)
        def shift_for(fraction):
            exposed = fraction * (right - left)
            return exposed - right if side == "left" else size - exposed - left

        # Bake the wider crop. The narrow pose moves that same artwork farther
        # outside the window, so intermediate positions reveal real sprite pixels.
        edge_fraction, peek_fraction = 0.55, 0.665
        baked_shift = shift_for(peek_fraction)
        x = -center - baked_shift
        matrix = (c, s, center + c * x - s * center,
                  -s, c, center - s * x - c * center)
        folder = root / f"tilt-{side}"
        folder.mkdir()
        pages, timeline = [], []
        for index, frames in enumerate(batched(idle["frames"], batch)):
            strip = Image.new("RGBA", (size * len(frames), size))
            page_id = f"tilt-{side}-{index:03}"
            source = root / "idle" / f"{frames[0]['page']}.png"
            with Image.open(source) as atlas:
                for slot, frame in enumerate(frames):
                    x, y, w, h = frame["rect"]
                    sprite = atlas.crop((x, y, x + w, y + h)).convert("RGBa")
                    sprite = sprite.transform((size, size), Image.Transform.AFFINE, matrix,
                                              Image.Resampling.BICUBIC).convert("RGBA")
                    strip.paste(sprite, (slot * size, 0))
                    timeline.append({**frame, "page": page_id})
            path = folder / f"{page_id}.png"
            save_png(strip, path)
            pages.append({"id": page_id, "size": list(strip.size), "png": payload(root, path)})
        for pose, fraction in (("edge", edge_fraction), ("peek", peek_fraction)):
            action_id = f"{pose}-{side}"
            shift = shift_for(fraction)
            offset = (shift - baked_shift) / size
            action = {**idle, "id": action_id, "offset": [offset, 0], "scale": 1, "frames": timeline}
            report = {"action_id": action_id, "parent": "idle", "bounds_alpha_threshold": 128,
                      "clockwise_degrees": angle, "stable_bounds": [x0, y0, x1, y1],
                      "target_visible_width_fraction": fraction, "horizontal_shift_px": shift,
                      "baked_shift_px": baked_shift, "canvas_offset_x": offset,
                      "frame_count": len(action["frames"])}
            yield action, pages, report
            pages = []


def add_resources(root, manifest, name, click, ffmpeg):
    poster = root / "poster.png"
    if poster.exists():
        with Image.open(poster) as image:
            manifest["pages"].append({"id": "character-poster", "size": list(image.size), "png": payload(root, poster)})
        manifest["resources"].append({"key": "character.poster", "content": {"kind": "image", "page": "character-poster"}})
    if name:
        path = root / "character-name.txt"
        path.write_text(name + "\n", encoding="utf-8")
        manifest["resources"].append({"key": "character.name", "content": {"kind": "text", "file": payload(root, path)}})
    if click:
        samples = subprocess.check_output([ffmpeg, "-v", "error", "-nostdin", "-i", click,
                                           "-f", "s16le", "-ac", "1", "-ar", "44100", "pipe:1"])
        path = root / "click.wav"
        with wave.open(str(path), "wb") as stream:
            stream.setparams((1, 2, 44100, 0, "NONE", "not compressed"))
            stream.writeframes(samples)
        manifest["resources"].append({"key": "interaction.click", "content": {"kind": "audio", "file": payload(root, path)}})
        manifest["source_sha256"].append(file_hash(click))


def add_icon(root, manifest, provenance, key, source, sizes):
    import io

    with Image.open(source) as image:
        if image.format != "PNG" or image.n_frames != 1 or image.width != image.height:
            raise ValueError(f"expected a square static PNG icon: {source.name}")
        rgba = image.convert("RGBA")
    header = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    pixels = bytearray()
    for size in sizes:
        buffer = io.BytesIO()
        rgba.resize((size, size), Image.Resampling.LANCZOS).save(buffer, format="PNG")
        data = buffer.getvalue()
        offset = 6 + 16 * len(sizes) + len(pixels)
        header.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        pixels.extend(data)
    path = root / f"{key}.ico"
    path.write_bytes(header + pixels)
    manifest["resources"].append({"key": key, "content": {"kind": "icon", "file": payload(root, path)}})
    digest = file_hash(source)
    manifest["source_sha256"].append(digest)
    provenance["resources"][key] = {"source": source.name, "sha256": digest, "sizes": sizes,
                                     "recipe": "Lanczos RGBA8 PNG in ICO"}


def build(args):
    source_data = args.source.read_bytes()
    source_manifest = json.loads(source_data.decode("utf-8-sig"))
    inventory = source_actions(source_manifest, args.source.parent)
    chosen = set(args.action) | {"idle"} if args.action else set(inventory)
    unknown = (chosen | args.protect.keys()) - inventory.keys()
    if unknown or args.protect.keys() - chosen:
        raise ValueError(f"invalid action or protection selection: {sorted(unknown | (args.protect.keys() - chosen))}")
    source_hash = hashlib.sha256(source_data).hexdigest()
    tools = {name: importlib.metadata.version(name) for name in ("pillow", "numpy", "opencv-python-headless")}
    tools.update({path.name: f"sha256:{file_hash(path)}" for path in Path(__file__).parent.glob("*.py")})
    tools.update(ffmpeg=subprocess.check_output([args.ffmpeg, "-version"], text=True).splitlines()[0],
                 ffmpeg_sha256=file_hash(args.ffmpeg), zlib=zlib.ZLIB_RUNTIME_VERSION)
    manifest = {"schema_version": 1, "pack_id": "white-dragon", "pack_version": args.pack_version,
                "color_space": "sRGB", "alpha": "straight", "source_sha256": [source_hash],
                "tools": tools, "pages": [], "actions": [], "resources": []}
    provenance = {"schema_version": 1, "upstream_release": args.upstream_release,
                  "source_manifest": {"name": args.source.name, "sha256": source_hash},
                  "recipe": {"size": args.size, "batch": args.batch, "profile": args.profile,
                             "decode_filter": "bicubic", "hidden_rgb": "zero", "format": "PNG"},
                  "source_actions": [], "derived_actions": [], "resources": {}}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    cv2.setNumThreads(1)
    with tempfile.TemporaryDirectory(prefix=".candidate-", dir=args.output.parent) as temporary:
        root = Path(temporary) / "pack"
        root.mkdir()
        for action_id, (group, item, source) in inventory.items():
            if action_id not in chosen:
                continue
            action, pages, bounds, report = build_action(args, root, action_id, item, source)
            report.update(source_group=group, scale=action["scale"], offset=action["offset"],
                          directions=action["directions"], movement_us=action["movement_us"])
            manifest["pages"].extend(pages)
            manifest["actions"].append(action)
            manifest["source_sha256"].append(report["sha256"])
            provenance["source_actions"].append(report)
            if action_id == "idle" and not args.action:
                for pose, pose_pages, details in derive_poses(root, action, bounds, args.batch):
                    manifest["actions"].append(pose)
                    manifest["pages"].extend(pose_pages)
                    provenance["derived_actions"].append(details)
        add_resources(root, manifest, source_manifest.get("character"), args.click, args.ffmpeg)
        provenance["resources"] = {"character.poster": {"action": "idle", "frame": 0},
                                   "character.name": {"source": "character"}}
        if args.click:
            provenance["resources"]["interaction.click"] = {
                "source": args.click.name, "sha256": manifest["source_sha256"][-1],
                "recipe": "mono PCM16 44100 Hz", "license": "see source sound license"}
        if args.app_icon:
            add_icon(root, manifest, provenance, "app.icon", args.app_icon, (16, 24, 32, 48, 64, 128, 256))
        if args.tray_icon:
            add_icon(root, manifest, provenance, "tray.icon", args.tray_icon, (16, 20, 24, 32, 48, 64))
        content = (json.dumps(manifest, separators=(",", ":")) + "\n").encode()
        if len(content) > MIB:
            raise ValueError(f"manifest exceeds the v1 limit: {len(content)} bytes")
        (root / "manifest.json").write_bytes(content)
        provenance.update(pack_manifest_sha256=hashlib.sha256(content).hexdigest(), tools=tools)
        (root / "provenance.json").write_text(json.dumps(provenance, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        subprocess.run([args.validator, "--distribution", root], check=True)
        if args.output.exists():
            raise FileExistsError(args.output)
        root.rename(args.output)
    print(f"{args.output}: {len(manifest['actions'])} actions, {len(manifest['pages'])} pages, {len(content)} manifest bytes")


def main():
    from .__main__ import rectangle

    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("source", "ffmpeg", "validator", "output"):
        parser.add_argument(f"--{option}", type=Path, required=True)
    parser.add_argument("--pack-version", required=True)
    parser.add_argument("--upstream-release", required=True)
    parser.add_argument("--click", type=Path)
    parser.add_argument("--app-icon", type=Path, help="square PNG for taskbar and desktop shortcuts")
    parser.add_argument("--tray-icon", type=Path, help="square PNG for the notification area")
    parser.add_argument("--profile", choices=PROFILES, default="despeckle")
    parser.add_argument("--size", type=int, default=480)
    parser.add_argument("--batch", type=int, default=4)
    parser.add_argument("--action", action="append", help="build selected sample actions plus idle; omit for the full set")
    parser.add_argument("--protect", action="append", default=[], metavar="ACTION:X,Y,W,H")
    parser.add_argument("--gpu-exe", type=Path)
    parser.add_argument("--models", type=Path)
    parser.add_argument("--gpu-id", type=int)
    parser.add_argument("--tile", type=int, default=0)
    args = parser.parse_args()
    if not args.pack_version.strip() or not args.upstream_release.strip():
        parser.error("pack version and upstream release must not be blank")
    if (args.size < 4 or args.size % 4 or args.batch < 1 or args.size * args.batch > 4096
            or args.size**2 * args.batch * 5 > 6 * MIB):
        parser.error("page layout exceeds alignment, dimensions or playback budget")
    if args.profile == "gpu":
        if (not args.gpu_exe or not args.models or (args.tile and args.tile < 32)
                or (args.gpu_id is not None and args.gpu_id < 0)):
            parser.error("gpu profile requires executable and models, tile 0 or >=32, and a nonnegative GPU ID")
    elif args.gpu_exe or args.models or args.tile or args.gpu_id is not None:
        parser.error("GPU options require the gpu profile")
    if args.profile == "resize" and args.protect:
        parser.error("--protect requires alpha cleanup")
    try:
        protected = {}
        for entry in args.protect:
            action_id, coords = entry.split(":", 1)
            protected.setdefault(action_id, []).append(rectangle(coords))
        args.protect = protected
        for name in ("source", "ffmpeg", "validator", "click", "app_icon", "tray_icon"):
            if path := getattr(args, name):
                setattr(args, name, path.resolve(strict=True))
        args.output = args.output.resolve()
        if args.output.exists() or args.output.is_relative_to(args.source.parent):
            raise ValueError("use a new output directory outside the source tree")
        build(args)
    except (OSError, ValueError, KeyError, argparse.ArgumentTypeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Candidate build failed: {error}\n")


if __name__ == "__main__":
    main()
