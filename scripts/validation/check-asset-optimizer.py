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
    print("Asset optimizer CPU checks passed" + ("; Vulkan checks passed" if gpu_args else ""))


if __name__ == "__main__":
    main()
