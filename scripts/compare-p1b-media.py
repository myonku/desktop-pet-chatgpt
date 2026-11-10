"""Compare the generated pack's RGBA, PNG, BC7 and VP9 alpha reference."""

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
    for i in range(0, len(image), 4):
        x, y = (i // 4) % width, (i // 4) // width
        bg = (224 if (x // 8 + y // 8) % 2 else 64) if background == "checker" else background
        alpha = image[i + 3]
        for c in range(3):
            out[i + c] = (image[i + c] * alpha + bg * (255 - alpha) + 127) // 255
        out[i + 3] = 255
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--ffmpeg", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = (args.pack / "manifest.json").read_bytes()
    pack = json.loads(manifest)
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
