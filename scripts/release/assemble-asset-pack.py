"""Stage and zip only the payloads declared by a validated v1 asset pack."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from zipfile import ZIP_DEFLATED, ZipFile


def payload_paths(manifest):
    yield "manifest.json"
    for page in manifest["pages"]:
        for variant in ("png", "rgba8", "bc7"):
            if page.get(variant) is not None:
                yield page[variant]["path"]
    for resource in manifest["resources"]:
        content = resource["content"]
        if content["kind"] in ("audio", "icon", "text"):
            yield content["file"]["path"]


def validate(validator, pack):
    subprocess.run([validator, "--distribution", pack], check=True)


def assemble(source, validator, output):
    validate(validator, source)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".pack-", dir=output.parent) as temp:
        staging = Path(temp) / "pack"
        staging.mkdir()
        # Retain the exact manifest bytes. Validate the snapshot again before publishing it.
        manifest_path = source / "manifest.json"
        if not manifest_path.resolve(strict=True).is_relative_to(source):
            raise ValueError("manifest.json resolves outside the pack")
        with manifest_path.open("rb") as stream:
            manifest_bytes = stream.read(1024 * 1024 + 1)
        if len(manifest_bytes) > 1024 * 1024:
            raise ValueError("manifest.json exceeds the v1 size limit")
        (staging / "manifest.json").write_bytes(manifest_bytes)
        paths = sorted(set(payload_paths(json.loads(manifest_bytes))))
        for relative in paths:
            if relative == "manifest.json":
                continue
            if any(
                part in (".", "..") or not re.fullmatch(r"[A-Za-z0-9._-]+", part)
                for part in relative.split("/")
            ):
                raise ValueError(f"invalid pack-relative path: {relative}")
            file = (source / relative).resolve(strict=True)
            destination = (staging / relative).resolve()
            if not file.is_relative_to(source) or not destination.is_relative_to(staging):
                raise ValueError(f"payload escapes its pack: {relative}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(file, destination)
        validate(validator, staging)

        # Build and hash the complete archive before opening the requested output.
        archive = staging.parent / "asset-pack.zip"
        with ZipFile(archive, "x", ZIP_DEFLATED) as zipped:
            for relative in paths:
                zipped.write(staging / relative, relative)
        with archive.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        # Publish atomically without replacing an existing release artifact.
        if os.name == "nt":
            archive.rename(output)
        else:
            os.link(archive, output)
    return digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--validator", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        output = args.output.absolute()
        digest = assemble(args.pack.resolve(strict=True), args.validator.resolve(strict=True), output)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Could not assemble asset pack: {error}\n")
    print(f"{digest}  {output}")


if __name__ == "__main__":
    main()
