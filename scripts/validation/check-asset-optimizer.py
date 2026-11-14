"""Exercise the offline optimizer with generated RGBA frames from the repo root."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate(root, gpu_args):
    source = root / "input"
    source.mkdir()
    frame = Image.new("RGBA", (32, 32))
    ImageDraw.Draw(frame).rectangle((6, 6, 17, 17), fill=(210, 180, 150, 255))
    frame.putpixel((10, 10), (202, 172, 142, 255))
    frame.putpixel((27, 4), (50, 50, 50, 100))
    frame.putpixel((27, 27), (50, 50, 50, 100))
    for name in ("0002.png", "0001.png"):
        frame.save(source / name)

    def run(output, *options, success=True):
        result = subprocess.run(
            [sys.executable, "-m", "tools.asset_optimizer", "--source", str(source),
             "--output", str(output), *options],
            capture_output=True, text=True, timeout=120,
        )
        check((result.returncode == 0) == success, result.stdout + result.stderr)

    cpu = root / "cpu"
    run(cpu, "--protect", "27,27,1,1", "--denoise")
    report = json.loads((cpu / "report.json").read_bytes())
    check([entry["input"] for entry in report["frames"]] == ["0001.png", "0002.png"],
          "frame order")
    expected_alpha = frame.getchannel("A")
    expected_alpha.putpixel((27, 4), 0)
    for entry in report["frames"]:
        output = cpu / "frames" / entry["output"]
        check(entry["input_sha256"] == digest(source / entry["input"]), "input hash")
        check(entry["output_sha256"] == digest(output), "output hash")
        check(entry["alpha_pixels_cleared"] == 1, "removed one unprotected speck")
        with Image.open(output) as result:
            check(ImageChops.difference(result.getchannel("A"), expected_alpha).getbbox() is None,
                  "alpha cleanup preserves the protected speck and body")
            check(result.getpixel((6, 6)) == frame.getpixel((6, 6)), "RGB contour preserved")
            check(result.getpixel((10, 10)) != frame.getpixel((10, 10)), "opaque RGB denoised")

    original_report = digest(cpu / "report.json")
    run(cpu, success=False)
    check(digest(cpu / "report.json") == original_report, "existing result preserved")
    if gpu_args:
        gpu = root / "gpu"
        run(gpu, "--keep-alpha", *gpu_args)
        report = json.loads((gpu / "report.json").read_bytes())
        check(report["output_size"] == [64, 64], "GPU output dimensions")
        check(report["upscale"]["weights"], "GPU weight hashes")
        with Image.open(gpu / "frames/0001.png") as result:
            alpha = frame.getchannel("A").resize((64, 64), Image.Resampling.BILINEAR)
            check(ImageChops.difference(result.getchannel("A"), alpha).getbbox() is None,
                  "GPU RGB processing preserves the separately scaled alpha")
        failed_gpu = root / "failed-gpu"
        run(failed_gpu, *gpu_args, "--gpu-id", "999", success=False)
        check(not failed_gpu.exists(), "GPU failure left a published result")

    Image.new("RGB", (32, 32)).save(source / "0003.png")
    failed = root / "failed"
    run(failed, success=False)
    check(not failed.exists(), "invalid frame left a published result")
    check(not list(root.glob(".optimize-*")), "temporary output was left behind")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gpu-exe", type=Path)
    parser.add_argument("--models", type=Path)
    args = parser.parse_args()
    if bool(args.gpu_exe) != bool(args.models):
        parser.error("--gpu-exe and --models must be provided together")
    gpu_args = ["--gpu-exe", str(args.gpu_exe.resolve()), "--models", str(args.models.resolve())] if args.gpu_exe else []
    with tempfile.TemporaryDirectory(prefix="desktop-pet-optimizer-") as folder:
        validate(Path(folder), gpu_args)
    check_transient_alpha()
    check_contour_matte()
    print("Asset optimizer CPU checks passed" + ("; Vulkan checks passed" if gpu_args else ""))


def check_transient_alpha():
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    from tools.asset_optimizer.frames import suppress_alpha_noise

    frame = Image.new("RGBA", (320, 320))
    draw = ImageDraw.Draw(frame)
    draw.rectangle((100, 100, 220, 250), fill=(200, 160, 130, 255))
    draw.line((220, 175, 235, 175), fill=(200, 160, 130, 128))
    frame.putpixel((25, 25), (255, 0, 125, 255))
    frame.putpixel((25, 26), (255, 0, 125, 60))
    frame.putpixel((26, 26), (255, 0, 125, 10))
    frame.putpixel((40, 40), (250, 240, 100, 220))
    frame.putpixel((60, 60), (255, 255, 255, 12))
    draw.rectangle((250, 60, 255, 65), fill=(255, 250, 100, 220))
    neighbor = frame.copy()
    ImageDraw.Draw(neighbor).rectangle((24, 24, 27, 27), fill=(0, 0, 0, 0))
    neighbor.putpixel((40, 40), (0, 0, 0, 0))
    neighbor.putpixel((41, 40), (250, 240, 100, 220))
    cleaned, count = suppress_alpha_noise(frame, [neighbor], [(60, 60, 1, 1)])
    expected = frame.copy()
    for point in ((25, 25), (25, 26), (26, 26)):
        expected.putpixel(point, (*frame.getpixel(point)[:3], 0))
    check(count == 3 and cleaned.tobytes() == expected.tobytes(),
          "remove opaque and translucent transient specks while preserving a moving effect, connected detail and protection")


def check_contour_matte():
    from tools.asset_optimizer.frames import suppress_contour_matte

    frame = Image.new("RGBA", (320, 320))
    draw = ImageDraw.Draw(frame)
    draw.rectangle((80, 80, 240, 230), fill=(225, 215, 235, 255))
    olive = (127, 117, 89, 255)
    draw.rectangle((135, 135, 170, 170), fill=olive)
    draw.line((239, 120, 260, 120), fill=(220, 190, 230, 215))
    draw.line((160, 231, 160, 237), fill=olive)
    draw.rectangle((168, 234, 170, 236), fill=(115, 107, 85, 80))
    frame.putpixel((172, 236), (36, 31, 30, 110))
    frame.putpixel((190, 235), olive)
    frame.putpixel((210, 237), (245, 222, 120, 255))

    cleaned, count = suppress_contour_matte(frame, [(190, 235, 1, 1)])
    expected = frame.copy()
    alpha = expected.getchannel("A")
    draw = ImageDraw.Draw(alpha)
    draw.line((160, 232, 160, 237), fill=0)
    draw.rectangle((168, 234, 170, 236), fill=0)
    alpha.putpixel((172, 236), 0)
    expected.putalpha(alpha)
    check(count == 16 and cleaned.tobytes() == expected.tobytes(),
          "clear connected opaque matte and detached fringe while retaining the contour, enclosed color, bright effect and protection")


if __name__ == "__main__":
    main()
