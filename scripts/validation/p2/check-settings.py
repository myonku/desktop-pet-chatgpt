"""Validate P2-02 on the interactive Windows desktop using isolated product profiles."""

import argparse
from contextlib import contextmanager
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import time

native = runpy.run_path(str(Path(__file__).with_name("check-windows.py")))
U, K = native["U"], native["K"]
bounds, click, displays = (native[key] for key in ("bounds", "click", "displays"))
find_window, menu, message, require, wait = (
    native[key] for key in ("find_window", "menu", "message", "require", "wait")
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
K.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, w.LPVOID, w.DWORD, w.DWORD, w.HANDLE]
K.CreateFileW.restype = w.HANDLE
K.CloseHandle.argtypes = [w.HANDLE]
SETTINGS = "桌宠 · 设置"
RESOURCE_TRACE = re.compile(r'^P2 resources: Some\("([0-9a-f]+)"\) Some\("([0-9a-f]+)"\) Some\("([0-9a-f]+)"\)', re.M)


class Profile:
    def __init__(self, root):
        self.root = root
        self.config = root / "DesktopPetChatGPT/config.json"
        self.config.parent.mkdir(parents=True)

    def write(self, value):
        self.config.write_text(json.dumps(value, ensure_ascii=False) + "\n", encoding="utf-8")

    def read(self):
        return json.loads(self.config.read_text(encoding="utf-8"))


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
            duplicate = subprocess.run([self.exe], env=launch_env(self.profile),
                                       capture_output=True, timeout=10)
            require(duplicate.returncode == 0, "second launch exits without creating a pet")
        hwnd = self.settings()
        self.until("P2 settings control: scale", mark)
        time.sleep(0.12)
        return hwnd

    def control(self, hwnd, name, proportion=0.5):
        pattern = rf"^P2 settings control: {name} ([\d.-]+) ([\d.-]+) ([\d.-]+) ([\d.-]+)"
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
            time.sleep(0.07)
            send(0x200)
            send(0x201, 1)
            time.sleep(0.08)
            send(0x202)
            time.sleep(0.14)
            action = {"save": "Save", "reset": "Reset", "exit": "Exit",
                      "test_sound": "PlaySound"}.get(name, "Preview")
            self.until(f"P2 settings action: {action}", mark)
        finally:
            actual = w.POINT()
            if U.GetCursorPos(c.byref(actual)) and (actual.x, actual.y) == (point.x, point.y):
                U.SetCursorPos(original.x, original.y)

    def close_settings(self, hwnd):
        message(hwnd, 0x10)
        wait(lambda: not U.IsWindow(hwnd), "settings closed")

    def side(self):
        left, _, right, _ = bounds(self.hwnd)
        return right - left

    def passthrough(self):
        return bool(U.GetWindowLongW(self.hwnd, -20) & 0x20)

    def resources(self):
        match = wait(lambda: RESOURCE_TRACE.search(self.text()), "all three character resources loaded")
        return match.groups()


def launch_env(profile):
    b = displays()[0].work
    return dict(os.environ, LOCALAPPDATA=str(profile.root), DESKTOP_PET_TRACE="1",
                DESKTOP_PET_FORMAT="png", DESKTOP_PET_REFERENCE_CLIENT=f"auto,{b.left + 210},{b.top + 160}")


@contextmanager
def pet_process(exe, profile, log, expected_asset=True, extra=()):
    with log.open("wb") as stream:
        proc = subprocess.Popen([exe, *extra], env=launch_env(profile), stdout=stream, stderr=subprocess.STDOUT)
    pet = None
    try:
        hwnd = wait(lambda: find_window(proc.pid), "pet window", timeout=12)
        pet = Pet(proc, log, hwnd, exe, profile)
        pet.until("First asset frame presented" if expected_asset else "First frame presented")
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
        pet.until("P2 sound: interaction.click (settings)", offset)
        original_settings_bounds = bounds(hwnd)
        pet.control(hwnd, "scale", 0.03)
        wait(lambda: pet.side() != original_side, "live pet resize")
        require(bounds(hwnd) == original_settings_bounds, "resizing moved settings window")
        require(profile.config.read_bytes() == initial, "preview wrote config before Apply")
        pet.close_settings(hwnd)
        wait(lambda: pet.side() == original_side, "close cancels preview")

        hwnd = pet.open_settings(via_menu=True)
        pet.control(hwnd, "scale", 0.03)
        pet.control(hwnd, "sound")
        pet.control(hwnd, "task_bubbles")
        preview_side = pet.side()
        pet.control(hwnd, "save")
        saved = wait(lambda: (value := profile.read())["preferences"].get("sound") is False and value, "saved settings")
        require(saved["saved_size"]["base_dip"] == 240 and saved["saved_size"]["percent"] != 110
                and saved["preferences"]["task_bubbles"] is False, "saved size and independent toggles")
        pet.close_settings(hwnd)
        require(pet.side() == preview_side, "Apply did not retain the previewed size")
        offset = len(pet.text())
        click(pet.hwnd)
        time.sleep(0.4)
        require("P2 sound: interaction.click" not in pet.text()[offset:], "muted click emitted sound")

        menu(pet.hwnd, "P", tray=True)
        wait(pet.passthrough, "mouse passthrough enabled")
        wait(lambda: profile.read()["preferences"]["passthrough"], "passthrough preference saved")

    saved = profile.read()
    saved["pack_path"] = os.path.relpath(dusk, profile.config.parent)
    profile.write(saved)
    with pet_process(exe, profile, output / "dusk.log") as pet:
        dusk_resources = pet.resources()
        require(all(left != right for left, right in zip(dawn_resources, dusk_resources)),
                "image, click sound and generic name must all come from the new pack")
        require(pet.side() == preview_side, "saved resize did not survive restart")
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

    with pet_process(exe, profile, output / "reset.log", expected_asset=False) as pet:
        require("initial DIP 264.00" not in pet.text(), "reset left the old saved size active")
    return {"dawn": dawn_resources, "dusk": dusk_resources,
            "saved_pixels": preview_side, "initial_saved_pixels": original_side}


def recovery(exe, profile, output):
    corrupt = b'{"schema_version": 99}\n'
    profile.config.write_bytes(corrupt)
    with pet_process(exe, profile, output / "bad-config.log", expected_asset=False) as pet:
        pet.until("Could not load configuration")
        hwnd = pet.open_settings()
        require(profile.config.read_bytes() == corrupt, "bad file overwritten at startup")
        pet.control(hwnd, "reset")
        backups = profile.config.parent / "backups"
        backed_up = wait(lambda: list(backups.glob("config-invalid-*.json")), "invalid original backup")
        wait(lambda: profile.config.read_bytes() != corrupt, "repaired product config")
        require(backed_up[0].read_bytes() == corrupt and profile.read()["schema_version"] == 1,
                "explicit repair must preserve exact original bytes")

    original = profile.config.read_bytes()
    with pet_process(exe, profile, output / "locked-config.log", expected_asset=False) as pet:
        hwnd = pet.open_settings()
        handle = K.CreateFileW(str(profile.config), 0x80000000, 3, None, 3, 0, None)
        require(handle not in (None, c.c_void_p(-1).value), "lock destination without delete sharing")
        try:
            pet.control(hwnd, "sound")
            mark = len(pet.text())
            pet.control(hwnd, "save")
            pet.until("P2 settings error:", mark)
            require(profile.config.read_bytes() == original, "failed atomic replacement modified original")
            require(not list(profile.config.parent.glob(".config-*.tmp")), "staged file leaked")
            pet.close_settings(hwnd)
        finally:
            K.CloseHandle(handle)


def dev_isolation(exe, profile, dawn, output):
    profile.write({"schema_version": 1, "saved_size": {"base_dip": 240, "percent": 110}})
    original = profile.config.read_bytes()
    dev = output / "isolated.toml"
    dev.write_text(f"initial_scale_percent = 100\npack_path = {json.dumps(dawn.as_posix(), ensure_ascii=False)}\n",
                   encoding="utf-8")
    with pet_process(exe, profile, output / "dev.log", extra=("--dev-config", dev)) as pet:
        require(pet.side() != round(264 * U.GetDpiForWindow(pet.hwnd) / 96), "TOML overrides saved size")
        hwnd = pet.open_settings()
        pet.control(hwnd, "sound")
        pet.control(hwnd, "save")
        require(profile.config.read_bytes() == original, "development Apply wrote product preferences")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="Windows Release with p1b-reference")
    parser.add_argument("--packs", type=Path, required=True, help="P1 fixture directory containing dawn and dusk")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    dawn, dusk = (args.packs.resolve() / name for name in ("dawn", "dusk"))
    require(all((pack / "manifest.json").is_file() for pack in (dawn, dusk)), "missing P1 asset fixtures")
    exe = args.exe.resolve()
    report = settings_roundtrip(exe, Profile(output / "profile"), dawn, dusk, output)
    recovery(exe, Profile(output / "recovery-profile"), output)
    dev_isolation(exe, Profile(output / "dev-profile"), dawn, output)
    report["checks"] = ["single instance", "preview/apply/restart", "sound independence",
                        "tray passthrough", "resource swap", "reset/recovery", "locked config", "development isolation"]
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
