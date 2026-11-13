"""Assemble a local white-dragon v1 pack from the source manifest and WebM."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

IMPORT = Path(__file__).with_name("convert-vp9-action.py")
MANIFEST_LIMIT = 1024 * 1024
MAPPING = (
    ("interact", "01"),
    ("move-left", "move02left"),
    ("idle", "idle"),
    ("move-right", "move02"),
    ("drag", "drag"),
    ("land", "landing"),
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--legacy-manifest", type=Path, required=True)
    parser.add_argument("--ffmpeg", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pack-id", default="white-dragon")
    parser.add_argument("--pack-version", default="0.1.0")
    parser.add_argument("--size", type=int, default=320)
    parser.add_argument("--batch", type=int, default=4)
    parser.add_argument("--png-only", action="store_true")
    args = parser.parse_args()
    if not args.pack_version.strip():
        parser.error("pack version must not be blank")
    if args.output.exists():
        parser.error("output directory must not exist")
    legacy_bytes = args.legacy_manifest.read_bytes()
    legacy = json.loads(legacy_bytes.decode("utf-8-sig"))
    legacy_sha = hashlib.sha256(legacy_bytes).hexdigest()
    ffmpeg = args.ffmpeg.resolve(strict=True)
    with ffmpeg.open("rb") as stream:
        ffmpeg_sha = hashlib.file_digest(stream, "sha256").hexdigest()
    script_hashes = {
        script.name: hashlib.sha256(script.read_bytes()).hexdigest()
        for script in (Path(__file__), IMPORT, IMPORT.with_name("pack_png.py"))
    }
    items = {item["id"]: item for group in ("variants", "movementAnimations") for item in legacy[group]}
    items.update((legacy[key]["id"], legacy[key]) for key in ("idle", "dragAnimation", "landingAnimation"))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.output.parent, prefix=".assemble-") as workspace:
        staging = Path(workspace)
        pack = staging / "assembled"
        pack.mkdir()
        manifest = {"schema_version": 1, "pack_id": args.pack_id, "pack_version": args.pack_version,
                    "color_space": "sRGB", "alpha": "straight", "source_sha256": [legacy_sha],
                    "tools": {}, "pages": [], "actions": [], "resources": []}
        inputs = []
        for action_id, original_id in MAPPING:
            old = items[original_id]
            fps = old.get("fps", 24)
            if old.get("renderAnchor", "bottom-center") != "bottom-center":
                raise ValueError(f"unmapped render anchor: {original_id}")
            source = args.legacy_manifest.parent / old["playbackFile"]
            part = staging / action_id
            command = [sys.executable, str(IMPORT), "--ffmpeg", str(ffmpeg),
                       "--source", str(source), "--output", str(part),
                       "--fps", str(fps), "--batch", str(args.batch), "--size", str(args.size)]
            if args.png_only:
                command.append("--png-only")
            subprocess.run(command, check=True)
            data = json.loads((part / "manifest.json").read_text(encoding="utf-8"))
            action = data["actions"][0]
            total_us = sum(frame["duration_us"] for frame in action["frames"])
            if abs(total_us - old["durationMs"] * 1000) > 1_000_000 / fps:
                raise ValueError(f"unexpected source duration: {original_id}")
            direction = old.get("sourceFacing")
            movement = ([old["moveStartMs"] * 1000, old["moveStopMs"] * 1000] if direction else None)
            action.update(id=action_id, looping=action_id in ("idle", "drag"), anchor=[0.5, 1.0],
                          offset=old.get("renderOffset", [0.0, 0.0]), scale=old.get("renderScale", 1.0),
                          directions=[direction] if direction else [], movement_us=movement)
            for frame in action["frames"]:
                frame["page"] = f"{action_id}-{frame['page']}"
            (pack / action_id).mkdir()
            for page in data["pages"]:
                page["id"] = f"{action_id}-{page['id']}"
                for variant in ("png", "rgba8", "bc7"):
                    if variant not in page:
                        continue
                    payload = page[variant]
                    relative = f"{action_id}/{payload['path']}"
                    shutil.move(part / payload["path"], pack / relative)
                    payload["path"] = relative
            manifest["tools"].update(data["tools"])
            manifest["source_sha256"].extend(data["source_sha256"])
            manifest["pages"].extend(data["pages"])
            manifest["actions"].append(action)
            inputs.append({"action_id": action_id, "source_id": original_id,
                           "playback_file": old["playbackFile"], "sha256": data["source_sha256"][0],
                           "fps": fps, "frames": len(action["frames"]), "duration_us": total_us})
        manifest["tools"].update({name: "sha256:" + digest for name, digest in script_hashes.items()})
        manifest["tools"]["ffmpeg_binary"] = "sha256:" + ffmpeg_sha
        content = (json.dumps(manifest, separators=(",", ":")) + "\n").encode()
        if len(content) > MANIFEST_LIMIT:
            raise ValueError(f"manifest exceeds the v1 limit: {len(content)} > {MANIFEST_LIMIT} bytes")
        manifest_sha = hashlib.sha256(content).hexdigest()
        (pack / "manifest.json").write_bytes(content)
        provenance = {
            "schema_version": 1, "pack_manifest_sha256": manifest_sha,
            "source_manifest": {"name": args.legacy_manifest.name, "sha256": legacy_sha},
            "recipe": {"size": args.size, "batch": args.batch, "png_only": args.png_only},
            "tools": manifest["tools"], "actions": inputs,
        }
        (pack / "provenance.json").write_text(
            json.dumps(provenance, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        pack.rename(args.output.resolve())
        print(f"{args.output}  {len(inputs)} actions  {len(content)} manifest bytes  {manifest_sha}")


if __name__ == "__main__":
    main()
