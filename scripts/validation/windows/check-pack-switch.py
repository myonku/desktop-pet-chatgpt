"""Exercise the settings-to-restart flow and late idle-page recovery using real v2 packs."""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib

import uiautomation as ui

from windows_desktop import U, find_window, invoke, menu, message, open_quick, panel, require, wait

FALLBACK = "指定素材包的待机动作无法播放，已切换到默认素材包。"


def manifest(pack):
    return json.loads((pack / "manifest.json").read_bytes())


def notice(hwnd, prefix):
    text = ui.ControlFromHandle(hwnd).TextControl(RegexName=f"^{prefix}")
    return wait(lambda: text.Exists(0), prefix, timeout=30)


@contextmanager
def launch(exe, profile, log):
    with log.open("wb") as stream, subprocess.Popen(
        [exe], cwd=exe.parent, env=dict(os.environ, LOCALAPPDATA=str(profile), DESKTOP_PET_TRACE="1"),
        stdout=stream, stderr=subprocess.STDOUT,
    ) as proc:
        pet = None
        try:
            pet = wait(lambda: find_window(proc.pid), "pet window")
            wait(lambda: "First asset frame presented" in log.read_text(encoding="utf-8", errors="replace"),
                 "first package frame")
            yield pet
        finally:
            if proc.poll() is None:
                if pet and U.IsWindow(pet):
                    message(pet, 0x10)
                else:
                    proc.kill()
                try:
                    proc.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)
    require(proc.returncode == 0, f"pet did not exit normally; see {log.name}")


def write_config(path, pack=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    selected = f"pack_path = {json.dumps(pack.as_posix(), ensure_ascii=False)}\n" if pack else ""
    path.write_text(selected + "[preferences]\nauto_move = false\nfullscreen_hide = false\n", encoding="utf-8")


def check_switch(exe, profile, default, alternate, output):
    config = profile / "DesktopPetChatGPT/config.toml"
    write_config(config)
    original = config.read_bytes()
    with launch(exe, profile, output / "select.log") as pet:
        menu(pet, "s", tray=True)
        settings = wait(lambda: panel(pet), "settings window")
        root = ui.ControlFromHandle(settings)
        entry = root.EditControl(AutomationId="asset-pack-path")
        require(entry.Exists(5), "pack directory input")
        entry.GetPattern(ui.PatternId.ValuePattern).SetValue(str(alternate))
        invoke(settings, "检查素材包")
        notice(settings, "素材包通过检查")
        require(config.read_bytes() == original, "checking leaves the saved config unchanged")
        invoke(settings, "应用并保存")
        notice(settings, "已保存。重启后载入")
        saved = tomllib.loads(config.read_text(encoding="utf-8"))
        require(Path(saved["pack_path"]) == alternate, "validated path was saved")

    candidate = manifest(alternate)
    built_in = {item["id"] for item in default["clips"]}
    poses = {binding["variants"][binding["default"]] for key, binding in candidate["bindings"].items()
             if key.startswith(("dock.", "peek."))}
    alternate_only = next((item for item in candidate["clips"] if item["id"] not in built_in | poses), None)
    require(alternate_only is not None, "alternate pack needs an additional in-place preview clip")
    clip_id = alternate_only["id"]
    restart_log = output / "restart.log"
    with launch(exe, profile, restart_log) as pet:
        quick = open_quick(pet)
        invoke(quick, "动作预览")
        mark = restart_log.stat().st_size
        invoke(quick, alternate_only.get("preview_name", clip_id))

        def preview_presented():
            log = restart_log.read_bytes()[mark:].decode("utf-8", errors="replace").replace("\r\n", "\n")
            _, selected, playback = log.partition(f"Preview clip: {clip_id}\n")
            _, prepared, frames = playback.partition(f"Playback action: {clip_id} ")
            return selected and prepared and "\nPlayback frame " in frames

        wait(preview_presented, "explicit alternate-only preview presents a frame after restart")
    return clip_id


def check_recovery(exe, profile, default_path, output):
    damaged = output / "damaged-selected"
    shutil.copytree(default_path, damaged)
    data = manifest(damaged)
    idle_binding = data["bindings"]["idle"]
    idle = next(clip for clip in data["clips"] if clip["id"] == idle_binding["variants"][idle_binding["default"]])
    _, later, *_ = dict.fromkeys(frame["page"] for frame in idle["frames"])
    file = next(page["payload"]["path"] for page in data["pages"] if page["id"] == later)
    (damaged / file).write_bytes(b"damaged after selection")
    config = profile / "DesktopPetChatGPT/config.toml"
    write_config(config, damaged)
    original = config.read_bytes()
    for attempt in range(1, 3):
        log = output / f"recovery-{attempt}.log"
        with launch(exe, profile, log) as pet:
            wait(lambda: FALLBACK in log.read_text(encoding="utf-8", errors="replace"), "runtime default fallback")
            previous = len(log.read_bytes())
            wait(lambda: b"Playback frame " in log.read_bytes()[previous:], "default animation presents")
            menu(pet, "s", tray=True)
            settings = wait(lambda: panel(pet), "settings after runtime fallback")
            notice(settings, FALLBACK)
        require(config.read_bytes() == original, "runtime fallback leaves user selection intact")
    return file


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("exe", "default-pack", "alternate-pack", "output"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    default, alternate = args.default_pack.resolve(strict=True), args.alternate_pack.resolve(strict=True)
    require(default != alternate, "two distinct v2 packs required")
    exe = Path(shutil.copy2(args.exe.resolve(strict=True), output))
    shutil.copytree(default, output / "resources/default")
    action = check_switch(exe, output / "switch-profile", manifest(default), alternate, output)
    file = check_recovery(exe, output / "recovery-profile", default, output)
    report = {"default": manifest(default)["pack_version"], "alternate": manifest(alternate)["pack_version"],
              "checked_without_save": True, "saved_and_restarted": True, "played_alternate_action": action,
              "damaged_page": file, "runtime_recoveries": 2, "saved_config_unchanged_on_fallback": True}
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
