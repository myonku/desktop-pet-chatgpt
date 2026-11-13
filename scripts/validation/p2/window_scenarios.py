"""Launch the isolated Windows behavior build and check its client size."""

import json
import math
import os
import re
import subprocess

from windows_desktop import U, bounds, find_window, get_monitor, message, require, wait


class Run:
    def __init__(self, output, exe, pack, name, monitor, mode="auto"):
        self.log = output / f"{name}.log"
        self.cwd = output / "startup-configs" / name
        self.cwd.mkdir(parents=True)
        config = self.cwd / "desktop-pet.dev.toml"
        config.write_text(f"initial_scale_percent = 70\npack_path = {json.dumps(pack.as_posix(), ensure_ascii=False)}\n",
                          encoding="utf-8")
        env = dict(os.environ, DESKTOP_PET_TRACE="1", DESKTOP_PET_FORMAT="png",
                   LOCALAPPDATA=str(output.resolve() / "profile"))
        b = monitor.work
        env["DESKTOP_PET_WINDOW_GEOMETRY"] = f"{mode},{b.left + 300},{b.top + 200}"
        self.proc = None
        self.hwnd = None
        self.exe, self.env = exe, env

    def __enter__(self):
        with self.log.open("wb") as log:
            self.proc = subprocess.Popen([self.exe], cwd=self.cwd, env=self.env,
                                         stdout=log, stderr=subprocess.STDOUT)
        try:
            self.hwnd = wait(lambda: find_window(self.proc.pid), f"window for {self.log.name}")
            self.until("First asset frame presented")
        except BaseException:
            self.close()
            raise
        return self

    def close(self):
        if self.proc and self.proc.poll() is None:
            if self.hwnd:
                message(self.hwnd, 0x10)
            try:
                self.proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.proc.terminate()
                self.proc.wait(timeout=3)

    def __exit__(self, exc_type, *_):
        self.close()
        if exc_type is None:
            require(self.proc.returncode == 0, f"pet exited with {self.proc.returncode}")

    def text(self):
        with self.log.open(encoding="utf-8", errors="replace") as log:
            return log.read()

    def until(self, marker, offset=0, timeout=5):
        return wait(lambda: marker in self.text()[offset:], f"{self.log.name}: {marker}", timeout)

    def action(self, name, offset=0, timeout=5):
        self.until(f"Playback action: {name} ", offset, timeout)


def check_size(run, monitor, forced_dip=False):
    match = re.search(r"EDID mm (Some\(\[(\d+), (\d+)\]\)|None), initial DIP ([0-9.]+)", run.text())
    require(match, "startup size trace")
    _, width_mm, height_mm, dip = match.groups()
    dpi = U.GetDpiForWindow(run.hwnd)
    screen = monitor.bounds
    work = monitor.work
    resolution = [screen.right - screen.left, screen.bottom - screen.top]
    if forced_dip:
        require(height_mm is None, "forced DIP mode must skip EDID")
    if height_mm:
        height = int(height_mm)
        base_pixels = max(36, min(90, 0.16 * height)) * resolution[1] / height
    else:
        base_pixels = 256 * dpi / 96
    expected_dip = base_pixels * 0.7 * 96 / dpi
    require(abs(float(dip) - expected_dip) <= 0.006, "startup size uses the expected DIP baseline")
    expected = min(math.floor(base_pixels * 0.7 + 0.5), work.right - work.left, work.bottom - work.top)

    def sized():
        left, top, right, bottom = bounds(run.hwnd)
        return (left, top, right, bottom) if right - left == bottom - top == expected else None

    left, _, right, _ = wait(sized, f"startup size settles at {expected}x{expected}")
    require(get_monitor(U.MonitorFromWindow(run.hwnd, 2)).device == monitor.device, "startup target monitor")
    return {"device": monitor.device, "edid_mm": [int(width_mm), int(height_mm)] if height_mm else None,
            "resolution_px": resolution, "work_area_px": [work.left, work.top, work.right, work.bottom],
            "dpi": dpi, "dip": expected_dip, "size_px": right - left}
