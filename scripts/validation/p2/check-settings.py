"""Validate P2-02 on the interactive Windows desktop using isolated product profiles."""

import argparse
from contextlib import contextmanager
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import tomllib

from windows_desktop import (
    K, U, activate, bounds, click, displays, find_window, fullscreen, menu, message, require, wait,
)

U.GetClientRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
U.ClientToScreen.argtypes = [w.HWND, c.POINTER(w.POINT)]
U.GetCursorPos.argtypes = [c.POINTER(w.POINT)]
U.WindowFromPoint.argtypes = [w.POINT]
U.WindowFromPoint.restype = w.HWND
U.SendMessageTimeoutW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM, w.UINT,
                                 w.UINT, c.POINTER(c.c_size_t)]
U.SendMessageTimeoutW.restype = w.LPARAM
U.GetWindowLongW.argtypes = [w.HWND, c.c_int]
U.GetDlgItem.argtypes = [w.HWND, c.c_int]
U.GetDlgItem.restype = w.HWND
K.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, w.LPVOID, w.DWORD, w.DWORD, w.HANDLE]
K.CreateFileW.restype = w.HANDLE
K.CloseHandle.argtypes = [w.HANDLE]
SETTINGS = "桌宠 · 设置"
RESOURCE_TRACE = re.compile(r'^Asset resources: Some\("([0-9a-f]+)"\) Some\("([0-9a-f]+)"\) Some\("([0-9a-f]+)"\)', re.M)


class Profile:
    def __init__(self, root):
        self.root = root
        self.config = root / "DesktopPetChatGPT/config.toml"
        self.config.parent.mkdir(parents=True)

    def write(self, value):
        lines = [f"{key} = {json.dumps(field, ensure_ascii=False)}"
                 for key, field in value.items() if not isinstance(field, dict)]
        for table, fields in value.items():
            if isinstance(fields, dict):
                lines += [f"[{table}]", *(f"{key} = {json.dumps(field, ensure_ascii=False)}"
                                          for key, field in fields.items())]
        self.config.write_text("\n".join(lines) + "\n", encoding="utf-8")

    def read(self):
        return tomllib.loads(self.config.read_text(encoding="utf-8"))


class Pet:
    def __init__(self, proc, log, hwnd, exe, profile):
        self.proc, self.log, self.hwnd = proc, log, hwnd
        self.exe, self.profile = exe, profile

    def text(self):
        return self.log.read_text(encoding="utf-8", errors="replace")

    def until(self, marker, offset=0):
        wait(lambda: marker in self.text()[offset:], marker, timeout=12)

    def settings(self):
        return wait(lambda: find_window(self.proc.pid, SETTINGS), "settings HWND", timeout=10)

    def open_settings(self, via_menu=False):
        mark = len(self.text())
        if via_menu:
            menu(self.hwnd, "S")
        else:
            duplicate = subprocess.run([self.exe], cwd=self.profile.root, env=launch_env(self.profile),
                                       capture_output=True, timeout=10)
            require(duplicate.returncode == 0, "second launch exits without creating a pet")
        hwnd = self.settings()
        self.until("Settings control: scale", mark)
        time.sleep(0.12)
        return hwnd

    def control(self, hwnd, name, proportion=0.5):
        activate(hwnd)
        pattern = rf"^Settings control: {name} ([\d.-]+) ([\d.-]+) ([\d.-]+) ([\d.-]+)"
        coordinates = wait(lambda: re.findall(pattern, self.text(), re.M), f"layout of {name}")
        left, top, right, bottom = map(float, coordinates[-1])
        dpi = U.GetDpiForWindow(hwnd) / 96
        x, y = round((left + (right - left) * proportion) * dpi), round((top + bottom) * dpi / 2)
        rect = w.RECT()
        U.GetClientRect(hwnd, c.byref(rect))
        require(0 <= x < rect.right and 0 <= y < rect.bottom, f"{name} outside client area")
        original = w.POINT()
        require(U.GetCursorPos(c.byref(original)), "read test cursor position")
        point = w.POINT(x, y)
        require(U.ClientToScreen(hwnd, c.byref(point)), "locate settings control")
        require(U.WindowFromPoint(point) == hwnd, f"{name} is covered by another window")
        require(U.SetCursorPos(point.x, point.y), "move pointer inside the test window")
        mark = len(self.text())
        try:
            # Keep the real pointer over the test HWND while sending targeted messages.
            # Otherwise Win32 can send a cursor-leave between the two button messages.
            packed = (x & 65535) | ((y & 65535) << 16)
            result = c.c_size_t()
            def send(msg, flags=0):
                require(U.SendMessageTimeoutW(hwnd, msg, flags, packed, 2, 1000, c.byref(result)),
                        f"send mouse message {msg:#x} to settings window")
            action = {"save": "Save", "reset": "Reset", "exit": "Exit",
                      "test_sound": "PlaySound"}.get(name, "Preview")
            marker = f"Settings action: {action}"
            for attempt in range(3):
                time.sleep(0.07)
                send(0x200)
                send(0x201, 1)
                time.sleep(0.08)
                send(0x202)
                try:
                    wait(lambda: marker in self.text()[mark:], marker, timeout=2)
                    time.sleep(0.14)
                    break
                except AssertionError:
                    if attempt == 2:
                        raise
        finally:
            actual = w.POINT()
            if U.GetCursorPos(c.byref(actual)) and (actual.x, actual.y) == (point.x, point.y):
                U.SetCursorPos(original.x, original.y)

    def close_settings(self, hwnd):
        message(hwnd, 0x10)
        wait(lambda: not U.IsWindow(hwnd), "settings closed")

    def side(self):
        rect = w.RECT()
        require(U.GetClientRect(self.hwnd, c.byref(rect)), "read pet client size")
        return rect.right

    def passthrough(self):
        return bool(U.GetWindowLongW(self.hwnd, -20) & 0x20)

    def resources(self):
        match = wait(lambda: RESOURCE_TRACE.search(self.text()), "all three character resources loaded")
        return match.groups()


def launch_env(profile):
    b = displays()[0].work
    return dict(os.environ, LOCALAPPDATA=str(profile.root), DESKTOP_PET_TRACE="1",
                DESKTOP_PET_FORMAT="png", DESKTOP_PET_WINDOW_GEOMETRY=f"auto,{b.left + 210},{b.top + 160}")


def launch(exe, profile, log, cwd=None):
    with log.open("wb") as stream:
        return subprocess.Popen([exe], cwd=cwd or profile.root, env=launch_env(profile),
                                stdout=stream, stderr=subprocess.STDOUT)


def answer_config_dialog(proc, choice):
    dialog = wait(lambda: find_window(proc.pid, "桌宠配置无法加载"), "startup configuration dialog")
    button = U.GetDlgItem(dialog, choice)
    require(button and U.PostMessageW(dialog, 0x111, choice, button), "answer startup dialog")


@contextmanager
def pet_process(exe, profile, log, cwd=None, answer=None, start_hidden=False):
    proc = launch(exe, profile, log, cwd)
    pet = None
    try:
        if answer is not None:
            answer_config_dialog(proc, answer)
        hwnd = wait(lambda: find_window(proc.pid), "pet window", timeout=12)
        pet = Pet(proc, log, hwnd, exe, profile)
        if start_hidden:
            pet.until("Window: DPI scale")
            require(not U.IsWindowVisible(hwnd), "start_hidden did not keep the pet hidden")
            require("First asset frame presented" not in pet.text(), "hidden startup presented a frame")
            menu(hwnd, "H", tray=True)
        pet.until("First asset frame presented")
        yield pet
    finally:
        if proc.poll() is None:
            if pet and U.IsWindow(pet.hwnd):
                message(pet.hwnd, 0x10)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.terminate()
                proc.wait(timeout=5)
    require(proc.returncode == 0, f"pet exit code {proc.returncode}: {log}")


def startup_visibility(exe, profile, output):
    profile.write({"preferences": {"start_hidden": True, "fullscreen_hide": False}})
    original = profile.config.read_bytes()
    for name in ("start-hidden", "restart-hidden"):
        with pet_process(exe, profile, output / f"{name}.log", start_hidden=True) as pet:
            require(U.IsWindowVisible(pet.hwnd), "tray did not restore the hidden pet")
            with fullscreen(displays()[0]):
                time.sleep(0.5)
                require(U.IsWindowVisible(pet.hwnd), "fullscreen_hide=false ignored")
        require(profile.config.read_bytes() == original, "manual show changed startup preference")


def settings_roundtrip(exe, profile, dawn, dusk, output):
    profile.write({"schema_version": 1, "initial_scale_percent": 50,
                   "saved_size": {"base_dip": 240, "percent": 110},
                   "preferences": {"auto_move": False},
                   "pack_path": os.path.relpath(dawn, profile.config.parent)})
    initial = profile.config.read_bytes()
    with pet_process(exe, profile, output / "dawn.log") as pet:
        dawn_resources = pet.resources()
        dpi = U.GetDpiForWindow(pet.hwnd)
        original_side = round(264 * dpi / 96)
        require(pet.side() == original_side, "saved logical size must override startup percentage")
        require("EDID mm None, initial DIP 264.00" in pet.text(), "saved size bypassed monitor probe")

        hwnd = pet.open_settings()
        pet.control(hwnd, "task_bubbles")
        offset = len(pet.text())
        pet.control(hwnd, "test_sound")
        pet.until("Sound: interaction.click (settings)", offset)
        original_settings_bounds = bounds(hwnd)
        pet.control(hwnd, "scale", 0.03)
        wait(lambda: pet.side() != original_side, "live pet resize")
        require(bounds(hwnd) == original_settings_bounds, "resizing moved settings window")
        require(profile.config.read_bytes() == initial, "preview wrote config before Apply")
        mark = len(pet.text())
        menu(pet.hwnd, "S")
        wait(lambda: not U.IsWindow(hwnd), "role menu closes the existing settings window")
        wait(lambda: pet.side() == original_side, "role menu cancels preview")
        require(profile.config.read_bytes() == initial, "opening the role menu saved a preview")

        hwnd = pet.settings()
        pet.until("Settings control: scale", mark)
        pet.control(hwnd, "scale", 0.03)
        pet.control(hwnd, "sound")
        pet.control(hwnd, "task_bubbles")
        pet.control(hwnd, "save")
        saved = wait(lambda: (value := profile.read())["preferences"].get("sound") is False and value, "saved settings")
        require(saved["saved_size"]["base_dip"] == 240 and saved["saved_size"]["percent"] != 110
                and saved["preferences"]["task_bubbles"] is False, "saved size and independent toggles")
        preview_side = round(saved["saved_size"]["base_dip"] * saved["saved_size"]["percent"] / 100 * dpi / 96)
        wait(lambda: pet.side() == preview_side, "saved size settles in the preview")
        pet.close_settings(hwnd)
        require(pet.side() == preview_side, "Apply did not retain the previewed size")
        offset = len(pet.text())
        click(pet.hwnd)
        time.sleep(0.4)
        require("Sound: interaction.click" not in pet.text()[offset:], "muted click emitted sound")

        hwnd = pet.open_settings()
        pet.control(hwnd, "scale", 0.30)
        wait(lambda: pet.side() != preview_side, "resize preview before tray menu")
        menu(pet.hwnd, "P", tray=True)
        wait(lambda: not U.IsWindow(hwnd), "tray menu closes settings")
        wait(lambda: pet.side() == preview_side, "tray menu cancels resize preview")
        wait(pet.passthrough, "mouse passthrough enabled")
        wait(lambda: profile.read()["preferences"]["passthrough"], "passthrough preference saved")
        require(profile.read()["saved_size"] == saved["saved_size"], "tray menu saved previewed size")

    saved = profile.read()
    saved["pack_path"] = os.path.relpath(dusk, profile.config.parent)
    profile.write(saved)
    with pet_process(exe, profile, output / "dusk.log") as pet:
        dusk_resources = pet.resources()
        require(all(left != right for left, right in zip(dawn_resources, dusk_resources)),
                "image, click sound and generic name must all come from the new pack")
        wait(lambda: pet.side() == preview_side, "saved resize survives restart")
        require(pet.passthrough(), "saved passthrough did not survive restart")
        menu(pet.hwnd, "P", tray=True)
        wait(lambda: not pet.passthrough(), "tray must restore mouse interaction")
        wait(lambda: not profile.read()["preferences"]["passthrough"], "tray recovery persisted")
        hwnd = pet.open_settings()
        pet.control(hwnd, "reset")
        reset = wait(lambda: (value := profile.read()).get("saved_size") is None and value, "reset persisted")
        require("pack_path" not in reset and "initial_scale_percent" not in reset
                and reset["preferences"]["sound"] and reset["preferences"]["task_bubbles"],
                "restored built-in defaults")
        pet.control(hwnd, "exit")
        wait(lambda: pet.proc.poll() is not None, "exit via settings")

    with pet_process(exe, profile, output / "reset.log") as pet:
        require("initial DIP 264.00" not in pet.text(), "reset left the old saved size active")
    return {"dawn": dawn_resources, "dusk": dusk_resources,
            "saved_pixels": preview_side, "initial_saved_pixels": original_side}


def recovery(exe, profile, output):
    corrupt = b"schema_version = 99\n"
    profile.config.write_bytes(corrupt)
    cancelled = launch(exe, profile, output / "cancel-config.log")
    try:
        answer_config_dialog(cancelled, 7)  # IDNO
        require(cancelled.wait(timeout=5) != 0, "declining config repair must stop startup")
    finally:
        if cancelled.poll() is None:
            cancelled.terminate()
            cancelled.wait(timeout=5)
    require(profile.config.read_bytes() == corrupt, "declining config repair modified the file")
    require(not (profile.config.parent / "backups").exists(), "declining config repair created a backup")
    with pet_process(exe, profile, output / "bad-config.log", answer=6) as pet:  # IDYES
        pet.until("Could not load configuration")
        backups = profile.config.parent / "backups"
        backed_up = list(backups.glob("config-invalid-*.toml"))
        require(len(backed_up) == 1, "one invalid original backup")
        require(backed_up[0].read_bytes() == corrupt and profile.read()["schema_version"] == 1,
                "explicit repair must preserve exact original bytes")

    profile.write({"pack_path": "missing-pack"})
    with pet_process(exe, profile, output / "default-pack.log") as pet:
        pet.until("指定素材包")
        pet.until("Asset playback: dawn")

    original = profile.config.read_bytes()
    with pet_process(exe, profile, output / "locked-config.log") as pet:
        hwnd = pet.open_settings()
        handle = K.CreateFileW(str(profile.config), 0x80000000, 3, None, 3, 0, None)
        require(handle not in (None, c.c_void_p(-1).value), "lock destination without delete sharing")
        try:
            pet.control(hwnd, "sound")
            mark = len(pet.text())
            pet.control(hwnd, "save")
            pet.until("Settings error:", mark)
            require(profile.config.read_bytes() == original, "failed atomic replacement modified original")
            require(not list(profile.config.parent.glob(".config-*.tmp")), "staged file leaked")
            pet.close_settings(hwnd)
        finally:
            K.CloseHandle(handle)


def dev_isolation(exe, product_exe, profile, dawn, dusk, output):
    profile.write({"schema_version": 1, "saved_size": {"base_dip": 240, "percent": 110},
                   "pack_path": dusk.as_posix()})
    original = profile.config.read_bytes()
    dev = profile.root / "desktop-pet.dev.toml"
    shutil.copytree(dawn, profile.root / "assets/packs/white-dragon/0.1.0")
    with pet_process(exe, profile, output / "dev-without-config.log") as pet:
        require("initial DIP 264.00" not in pet.text(), "missing dev TOML loaded the product size")
        pet.until("Asset playback: dawn")
        hwnd = pet.open_settings()
        pet.control(hwnd, "sound")
        pet.control(hwnd, "save")
        require(profile.config.read_bytes() == original, "missing dev TOML redirected save to product")
        require(not dev.exists(), "development Apply created a configuration file")

    dev.write_text(f"initial_scale_percent = 100\npack_path = {json.dumps(dawn.as_posix(), ensure_ascii=False)}\n",
                   encoding="utf-8")
    development = dev.read_bytes()
    with pet_process(exe, profile, output / "dev.log") as pet:
        require("initial DIP 264.00" not in pet.text(), "development launch used product size")
        hwnd = pet.open_settings()
        pet.control(hwnd, "sound")
        pet.control(hwnd, "save")
        require(profile.config.read_bytes() == original, "development Apply wrote product preferences")
        require(dev.read_bytes() == development, "development Apply wrote development preferences")

    with pet_process(product_exe, profile, output / "product-with-dev-config.log") as pet:
        pet.until("initial DIP 264.00")
        pet.until("Asset playback: dusk")
        require(dev.read_bytes() == development, "product launch modified development preferences")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="Windows Release with window-diagnostics")
    parser.add_argument("--dev-exe", type=Path, required=True, help="Windows Release with development,window-diagnostics")
    parser.add_argument("--packs", type=Path, required=True, help="P1 fixture directory containing dawn and dusk")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    dawn, dusk = (args.packs.resolve() / name for name in ("dawn", "dusk"))
    require(all((pack / "manifest.json").is_file() for pack in (dawn, dusk)), "missing P1 asset fixtures")
    exe = output / args.exe.name
    shutil.copy2(args.exe, exe)
    shutil.copytree(dawn, output / "resources/default")
    report = settings_roundtrip(exe, Profile(output / "profile"), dawn, dusk, output)
    recovery(exe, Profile(output / "recovery-profile"), output)
    startup_visibility(exe, Profile(output / "visibility-profile"), output)
    dev_exe = output / "development" / args.dev_exe.name
    dev_exe.parent.mkdir()
    shutil.copy2(args.dev_exe, dev_exe)
    dev_isolation(dev_exe, exe, Profile(output / "dev-profile"), dawn, dusk, output)
    report["checks"] = ["single instance", "preview/apply/restart", "menus close settings and discard previews", "sound independence",
                        "tray passthrough", "resource swap", "reset/recovery", "default pack fallback",
                        "locked config", "development isolation", "startup visibility and tray recovery"]
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
