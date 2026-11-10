"""Compare RGBA, PNG, BC7 and VP9 alpha, or inspect an imported reference pack."""

import argparse
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import time

import texture2ddecoder


def timed_ffmpeg(exe, *args, data=None):
    start = time.perf_counter()
    result = subprocess.run([str(exe), "-hide_banner", "-loglevel", "error",
                             *map(str, args)], input=data, stdout=subprocess.PIPE, check=True)
    return result.stdout, round(time.perf_counter() - start, 3)


def crop(image, width, rect):
    x, y, w, h = rect
    return b"".join(image[(row * width + x) * 4:(row * width + x + w) * 4]
                    for row in range(y, y + h))


def composite(image, width, background):
    out = bytearray(image)
    solid = (background,) * 3 if isinstance(background, int) else background
    for i in range(0, len(image), 4):
        bg = solid
        if background == "checker":
            x, y = (i // 4) % width, (i // 4) // width
            bg = (224 if (x // 8 + y // 8) % 2 else 64,) * 3
        alpha = image[i + 3]
        for c in range(3):
            out[i + c] = (image[i + c] * alpha + bg[c] * (255 - alpha) + 127) // 255
        out[i + 3] = 255
    return out


def compare_imported(pack_dir, pack, output):
    """Compare the source-decoded RGBA and BC7 pages without transcoding VP9 again."""
    action = pack["actions"][0]
    width, height = action["logical_size"]
    backgrounds = {"black": 0, "white": 255, "checker": "checker", "blue": (20, 70, 180)}
    png = runpy.run_path(str(Path(__file__).with_name("generate-p1b-fixtures.py")))["png"]
    totals = {name: {"max": 0, "sum": 0} for name in backgrounds}
    alpha_max = alpha_added = alpha_lost = pixels = 0
    page_sizes = dict.fromkeys(("png", "rgba8", "bc7"), 0)
    pages = {page["id"]: page for page in pack["pages"]}
    frames = action["frames"]
    sample_frames = sorted({0, len(frames) // 2, len(frames) - 1})
    for page_id, page in pages.items():
        w, h = page["size"]
        rgba = (pack_dir / page["rgba8"]["path"]).read_bytes()
        encoded = (pack_dir / page["bc7"]["path"]).read_bytes()
        bgra = texture2ddecoder.decode_bc7(encoded, w, h)
        a, b = rgba[3::4], bgra[3::4]
        alpha_max = max(alpha_max, max(abs(x - y) for x, y in zip(a, b)))
        alpha_added += sum(x == 0 and y != 0 for x, y in zip(a, b))
        alpha_lost += sum(x != 0 and y == 0 for x, y in zip(a, b))
        pixels += len(a)
        for fmt in page_sizes:
            page_sizes[fmt] += (pack_dir / page[fmt]["path"]).stat().st_size
        for index in sample_frames:
            frame = frames[index]
            if frame["page"] != page_id:
                continue
            if frame["rect"][2:] != [width, height]:
                raise ValueError("expected uncropped, full-size imported frames")
            raw = crop(rgba, w, frame["rect"])
            test = bytearray(crop(bgra, w, frame["rect"]))
            test[0::4], test[2::4] = test[2::4], test[0::4]
            for name, bg in backgrounds.items():
                ref, bc = composite(raw, width, bg), composite(test, width, bg)
                for channel in range(3):
                    diffs = [abs(x - y) for x, y in zip(ref[channel::4], bc[channel::4])]
                    totals[name]["sum"] += sum(diffs)
                    totals[name]["max"] = max(totals[name]["max"], max(diffs))
                stride = width * 4
                sheet = b"".join(image[i:i + stride] for i in range(0, len(ref), stride)
                                 for image in (ref, bc))
                (output / f"frame-{index}-{name}.png").write_bytes(png(sheet, width * 2, height))
    channels = len(sample_frames) * width * height * 3
    return {"frames": len(frames), "pixels": pixels, "sample_frames": sample_frames,
            "page_payload_bytes": page_sizes,
            "bc7_alpha_max": alpha_max, "bc7_alpha_added_pixels": alpha_added,
            "bc7_alpha_lost_pixels": alpha_lost,
            "composite_errors": {name: {"max": item["max"], "mae": round(item["sum"] / channels, 4)}
                                 for name, item in totals.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--ffmpeg", type=Path)
    parser.add_argument("--imported", action="store_true",
                        help="compare the source-decoded RGBA and BC7 pages from a local import")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.imported and args.ffmpeg is None:
        parser.error("--ffmpeg is required for a generated fixture")
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = (args.pack / "manifest.json").read_bytes()
    pack = json.loads(manifest)
    if args.imported:
        result = compare_imported(args.pack, pack, args.output)
        result["manifest_sha256"] = hashlib.sha256(manifest).hexdigest()
        (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(result))
        return
    action = pack["actions"][0]
    width, height = action["logical_size"]
    durations = [f["duration_us"] for f in action["frames"]]
    if (not durations or any(abs(us - 1_000_000 / 30) > 1 for us in durations)
            or any(f["rect"][2:] != [width, height] for f in action["frames"])):
        raise ValueError("expected a fixed-size 30 fps fixture")
    frames = {fmt: [] for fmt in ("rgba8", "png", "bc7")}
    sizes = dict.fromkeys(frames, 0)
    pages = {}
    for page in pack["pages"]:
        raw, png, bc7 = [args.pack / page[fmt]["path"] for fmt in frames]
        for fmt, path in zip(frames, (raw, png, bc7)):
            sizes[fmt] += path.stat().st_size
        w, h = page["size"]
        rgba = raw.read_bytes()
        lossless, _ = timed_ffmpeg(args.ffmpeg, "-i", png, "-f", "rawvideo", "-pix_fmt", "rgba", "-")
        if rgba != lossless:
            raise ValueError(f"PNG differs from RGBA8: {png}")
        bgra = texture2ddecoder.decode_bc7(bc7.read_bytes(), w, h)
        bc_rgba = bytearray(bgra)
        bc_rgba[0::4], bc_rgba[2::4] = bgra[2::4], bgra[0::4]
        pages[page["id"]] = (w, (rgba, lossless, bc_rgba))
    for frame in action["frames"]:
        w, images = pages[frame["page"]]
        for fmt, image in zip(frames, images):
            frames[fmt].append(crop(image, w, frame["rect"]))
    source = b"".join(frames["rgba8"])
    webm = args.output / "reference.webm"
    _, encode_s = timed_ffmpeg(args.ffmpeg, "-f", "rawvideo", "-pixel_format", "rgba",
                               "-video_size", f"{width}x{height}", "-framerate", 30,
                               "-i", "-", "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p",
                               "-lossless", 1, "-auto-alt-ref", 0, webm, data=source)
    vp9, decode_s = timed_ffmpeg(args.ffmpeg, "-c:v", "libvpx-vp9", "-i", webm,
                                 "-f", "rawvideo", "-pix_fmt", "rgba", "-")
    if len(vp9) != len(source) or min(vp9[3::4]) == 255:
        raise ValueError("VP9 reference frame count or alpha mismatch")
    stride = width * height * 4
    frames["vp9"] = [vp9[i:i + stride] for i in range(0, len(vp9), stride)]
    png = runpy.run_path(str(Path(__file__).with_name("generate-p1b-fixtures.py")))["png"]
    errors = {}
    for fmt in ("png", "bc7", "vp9"):
        errors[fmt] = {"alpha_max": max(abs(a - b) for a, b in
                                      zip(source[3::4], b"".join(frames[fmt])[3::4]))}
    for bg in (0, 255, "checker"):
        variants = {fmt: [composite(img, width, bg) for img in images]
                    for fmt, images in frames.items()}
        ref = b"".join(variants["rgba8"])
        for fmt in errors:
            diffs = [abs(a - b) for i, (a, b) in enumerate(zip(ref, b"".join(variants[fmt])))
                     if i % 4 != 3]
            errors[fmt][str(bg)] = {"max": max(diffs), "mae": round(sum(diffs) / len(diffs), 4)}
        for index in (0, len(durations) // 2):
            images = [images[index] for images in variants.values()]
            sheet = b"".join(img[y * width * 4:(y + 1) * width * 4]
                             for y in range(height) for img in images)
            (args.output / f"frame-{index}-{bg}.png").write_bytes(png(sheet, 4 * width, height))
    result = {
        "manifest_sha256": hashlib.sha256(manifest).hexdigest(),
        "page_payload_bytes": sizes,
        "manifest_and_aux_bytes": len(manifest) + sum(
            (args.pack / item["content"]["file"]["path"]).stat().st_size
            for item in pack["resources"] if "file" in item["content"]),
        "vp9_action_bytes": webm.stat().st_size,
        "ffmpeg_encode_s": encode_s,
        "ffmpeg_decode_s": decode_s,
        "composite_errors": errors,
    }
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
