"""Optimize a single animation's lexically ordered RGBA PNG sequence."""

import argparse
import hashlib
import importlib.metadata
import io
import json
from pathlib import Path
import subprocess
import tempfile

from PIL import Image

from .frames import clean_alpha, denoise_rgb, rgb_for_upscale


MODEL = "realesr-animevideov3"
SCALE = 2


def file_hash(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def rectangle(value):
    try:
        x, y, width, height = map(int, value.split(","))
        if min(x, y) >= 0 and min(width, height) > 0:
            return x, y, width, height
    except ValueError:
        pass
    raise argparse.ArgumentTypeError("expected x,y,width,height in input pixels")


def upscale(frames, exe, models, workspace, tile, gpu_id):
    exe, models = exe.resolve(strict=True), models.resolve(strict=True)
    model_files = [models / f"{MODEL}-x{SCALE}.{extension}" for extension in ("param", "bin")]
    provenance = {"model": MODEL, "scale": SCALE, "tile": tile, "gpu_id": gpu_id,
                  "executable_sha256": file_hash(exe),
                  "weights": {path.name: file_hash(path) for path in model_files}}
    rgb_input, rgb_output = workspace / "rgb-input", workspace / "rgb-output"
    rgb_input.mkdir()
    rgb_output.mkdir()
    paths = sorted(frames.glob("*.png"))
    for index, path in enumerate(paths):
        with Image.open(path) as frame:
            rgb_for_upscale(frame).save(rgb_input / f"{index:06}.png")
    command = [str(exe), "-i", str(rgb_input), "-o", str(rgb_output), "-m", str(models),
               "-n", MODEL, "-s", str(SCALE), "-t", str(tile), "-f", "png"]
    if gpu_id is not None:
        command.extend(("-g", str(gpu_id)))
    subprocess.run(command, check=True)
    for index, path in enumerate(paths):
        with Image.open(path) as frame, Image.open(rgb_output / f"{index:06}.png") as result:
            expected = (frame.width * SCALE, frame.height * SCALE)
            if result.size != expected:
                raise ValueError(f"GPU output size mismatch for {path.name}")
            alpha = frame.getchannel("A").resize(expected, Image.Resampling.BILINEAR)
            rgba = result.convert("RGB")
            rgba.putalpha(alpha)
        rgba.save(path)
    return provenance


def optimize(args):
    source, output = args.source.resolve(strict=True), args.output.resolve()
    if not source.is_dir() or output.is_relative_to(source) or output.exists():
        raise ValueError("use a PNG input directory and a new output directory outside it")
    inputs = sorted(path for path in source.iterdir() if path.is_file() and path.suffix.lower() == ".png")
    if not inputs:
        raise ValueError("no PNG frames found")
    if len({path.stem.casefold() for path in inputs}) != len(inputs):
        raise ValueError("frame names must be unique regardless of extension case")
    report = {
        "profile": "1", "alpha_cleanup": not args.keep_alpha, "protected": args.protect,
        "rgb_denoise": args.denoise, "upscale": None, "frames": [],
        "tool_sha256": {
            path.name: file_hash(path) for path in (Path(__file__), Path(__file__).with_name("frames.py"))
        },
        "dependencies": {
            name: importlib.metadata.version(name)
            for name in ("pillow", "numpy", "opencv-python-headless")
        },
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".optimize-", dir=output.parent) as temporary:
        workspace = Path(temporary)
        result = workspace / "result"
        frames = result / "frames"
        frames.mkdir(parents=True)
        size = None
        for path in inputs:
            data = path.read_bytes()
            with Image.open(io.BytesIO(data)) as frame:
                if frame.format != "PNG" or frame.mode != "RGBA" or frame.n_frames != 1:
                    raise ValueError(f"expected a single straight RGBA PNG frame: {path.name}")
                if size is None:
                    size = frame.size
                    if any(x + w > frame.width or y + h > frame.height for x, y, w, h in args.protect):
                        raise ValueError("protected rectangle lies outside the input canvas")
                if frame.size != size:
                    raise ValueError(f"inconsistent frame dimensions: {path.name}")
                cleaned, removed = (frame.copy(), 0) if args.keep_alpha else clean_alpha(frame, args.protect)
                if args.denoise:
                    cleaned = denoise_rgb(cleaned)
                name = path.stem + ".png"
                cleaned.save(frames / name)
            report["frames"].append({"input": path.name, "input_sha256": hashlib.sha256(data).hexdigest(),
                                     "output": name, "alpha_pixels_cleared": removed})
        report["input_size"] = size
        report["output_size"] = size
        if args.gpu_exe:
            report["upscale"] = upscale(frames, args.gpu_exe, args.models, workspace, args.tile, args.gpu_id)
            report["output_size"] = [dimension * SCALE for dimension in size]
        for entry in report["frames"]:
            entry["output_sha256"] = file_hash(frames / entry["output"])
        (result / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        result.rename(output)
    print(f"{output}  {len(inputs)} frames")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True, help="directory of RGBA PNG frames for one action")
    parser.add_argument("--output", type=Path, required=True, help="new directory for frames and report")
    parser.add_argument("--protect", type=rectangle, action="append", default=[],
                        metavar="X,Y,W,H", help="exclude a rectangle from alpha cleanup")
    parser.add_argument("--keep-alpha", action="store_true", help="skip alpha cleanup")
    parser.add_argument("--denoise", action="store_true", help="apply mild RGB denoising to opaque interiors")
    parser.add_argument("--gpu-exe", type=Path, help="path to realesrgan-ncnn-vulkan")
    parser.add_argument("--models", type=Path, help="directory containing the x2 anime video model")
    parser.add_argument("--gpu-id", type=int, help="select a Vulkan device by its Real-ESRGAN index")
    parser.add_argument("--tile", type=int, default=0, help="GPU tile size, 0 for automatic")
    args = parser.parse_args()
    if bool(args.gpu_exe) != bool(args.models):
        parser.error("--gpu-exe and --models must be provided together")
    if args.tile and (not args.gpu_exe or args.tile < 32):
        parser.error("--tile requires a GPU executable and must be 0 or at least 32")
    if args.gpu_id is not None and (not args.gpu_exe or args.gpu_id < 0):
        parser.error("--gpu-id requires a GPU executable and a nonnegative device index")
    if args.keep_alpha and args.protect:
        parser.error("--protect requires alpha cleanup")
    try:
        optimize(args)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Asset optimization failed: {error}\n")


if __name__ == "__main__":
    main()
