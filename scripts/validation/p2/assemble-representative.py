"""Assemble the local P2 representative pack from the legacy source manifest and WebM."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

IMPORT = Path(__file__).resolve().parents[1] / "p1/import-p1b-reference.py"
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
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output directory must not exist")
    legacy = json.loads(args.legacy_manifest.read_text(encoding="utf-8-sig"))
    items = {item["id"]: item for group in ("variants", "movementAnimations") for item in legacy[group]}
    items.update((legacy[key]["id"], legacy[key]) for key in ("idle", "dragAnimation", "landingAnimation"))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.output.parent, prefix="p2-assemble-") as workspace:
        staging = Path(workspace)
        pack = staging / "assembled"
        pack.mkdir()
        manifest = {"schema_version": 1, "pack_id": "white-dragon-p2-local", "pack_version": "0.1.0",
                    "color_space": "sRGB", "alpha": "straight", "source_sha256": [],
                    "tools": {}, "pages": [], "actions": [], "resources": []}
        for action_id, original_id in MAPPING:
            old = items[original_id]
            fps = old.get("fps", 24)
            if old.get("renderAnchor", "bottom-center") != "bottom-center":
                raise ValueError(f"unmapped render anchor: {original_id}")
            source = args.legacy_manifest.parent / old["playbackFile"]
            part = staging / action_id
            subprocess.run([sys.executable, str(IMPORT), "--ffmpeg", str(args.ffmpeg),
                            "--source", str(source), "--output", str(part),
                            "--fps", str(fps), "--batch", "4", "--size", "320"], check=True)
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
                    payload = page[variant]
                    relative = f"{action_id}/{payload['path']}"
                    shutil.move(part / payload["path"], pack / relative)
                    payload["path"] = relative
            manifest["tools"].update(data["tools"])
            manifest["source_sha256"].extend(data["source_sha256"])
            manifest["pages"].extend(data["pages"])
            manifest["actions"].append(action)
        manifest["tools"]["assembler"] = "sha256:" + hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
        content = (json.dumps(manifest, indent=2) + "\n").encode()
        (pack / "manifest.json").write_bytes(content)
        pack.rename(args.output.resolve())
        print(f"{args.output}  {len(manifest['actions'])} actions  {hashlib.sha256(content).hexdigest()}")


if __name__ == "__main__":
    main()
