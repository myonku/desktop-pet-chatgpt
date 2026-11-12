"""Exercise P2-01 in an interactive Windows session with self-owned test windows."""

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

U = c.WinDLL("user32")
G = c.WinDLL("gdi32")
K = c.WinDLL("kernel32")
U.SetThreadDpiAwarenessContext.argtypes = [w.HANDLE]
U.SetThreadDpiAwarenessContext.restype = w.HANDLE
U.SetThreadDpiAwarenessContext(-4)  # Per-monitor awareness v2.
U.GetForegroundWindow.restype = w.HWND
U.GetDpiForWindow.argtypes = [w.HWND]
U.MonitorFromWindow.argtypes = [w.HWND, w.DWORD]
U.MonitorFromWindow.restype = w.HMONITOR
U.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
U.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
U.CreateWindowExW.argtypes = [w.DWORD, w.LPCWSTR, w.LPCWSTR, w.DWORD, c.c_int, c.c_int,
                             c.c_int, c.c_int, w.HWND, w.HMENU, w.HINSTANCE, w.LPVOID]
U.CreateWindowExW.restype = w.HWND
G.CreateRectRgn.restype = w.HRGN
G.PtInRegion.argtypes = [w.HRGN, c.c_int, c.c_int]
G.DeleteObject.argtypes = [w.HGDIOBJ]
U.GetWindowRgn.argtypes = [w.HWND, w.HRGN]
U.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
U.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
U.GetMonitorInfoW.argtypes = [w.HMONITOR, w.LPVOID]
U.GetGUIThreadInfo.argtypes = [w.DWORD, w.LPVOID]
for api in (U.IsWindowVisible, U.IsWindow, U.BringWindowToTop, U.SetForegroundWindow, U.DestroyWindow):
    api.argtypes = [w.HWND]
ENUM_WINDOW = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
ENUM_MONITOR = c.WINFUNCTYPE(w.BOOL, w.HMONITOR, w.HDC, c.POINTER(w.RECT), w.LPARAM)
TITLE = "Desktop Pet ChatGPT"


class MonitorInfo(c.Structure):
    _fields_ = [("size", w.DWORD), ("bounds", w.RECT), ("work", w.RECT), ("flags", w.DWORD),
                ("device", w.WCHAR * 32)]


class GuiInfo(c.Structure):
    _fields_ = [("size", w.DWORD), ("flags", w.DWORD), *[(name, w.HWND) for name in
                ("active", "focus", "capture", "menu", "move", "caret")], ("rect", w.RECT)]


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def wait(check, label, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if result := check():
            return result
        time.sleep(0.025)
    raise AssertionError(f"timed out: {label}")


def bounds(hwnd):
    rect = w.RECT()
    require(U.GetWindowRect(hwnd, c.byref(rect)), "GetWindowRect")
    return rect.left, rect.top, rect.right, rect.bottom


def get_monitor(handle):
    info = MonitorInfo()
    info.size = c.sizeof(info)
    require(U.GetMonitorInfoW(handle, c.byref(info)), "GetMonitorInfoW")
    return info


def displays():
    result = []

    @ENUM_MONITOR
    def collect(handle, _dc, _rect, _arg):
        result.append(get_monitor(handle))
        return True

    require(U.EnumDisplayMonitors(None, None, collect, 0), "EnumDisplayMonitors")
    return sorted(result, key=lambda monitor: not monitor.flags & 1)


def find_window(pid):
    handles = []

    @ENUM_WINDOW
    def collect(hwnd, _arg):
        owner = w.DWORD()
        U.GetWindowThreadProcessId(hwnd, c.byref(owner))
        if owner.value == pid:
            title = c.create_unicode_buffer(256)
            U.GetWindowTextW(hwnd, title, len(title))
            if title.value == TITLE:
                handles.append(hwnd)
        return True

    U.EnumWindows(collect, 0)
    return next(iter(handles), None)


def message(hwnd, msg, x=0, y=0, flags=0):
    packed = (x & 65535) | ((y & 65535) << 16)
    require(U.PostMessageW(hwnd, msg, flags, packed), f"PostMessage {msg:#x}")


def click(hwnd):
    left, top, right, bottom = bounds(hwnd)
    x, y = (right - left) // 2, (bottom - top) // 2
    message(hwnd, 0x201, x, y, 1)
    message(hwnd, 0x202, x, y)


def drag(hwnd, x, y, release=True):
    left, top, right, bottom = bounds(hwnd)
    grab_x, grab_y = (right - left) // 2, (bottom - top) // 2
    message(hwnd, 0x201, grab_x, grab_y, 1)
    message(hwnd, 0x200, x + grab_x - left, y + grab_y - top, 1)
    wait(lambda: bounds(hwnd)[:2] != (left, top), "native drag movement")
    if release:
        message(hwnd, 0x202, grab_x, grab_y)


def hit(hwnd, x, y):
    left, top, right, bottom = bounds(hwnd)
    region = G.CreateRectRgn(0, 0, 0, 0)
    try:
        require(U.GetWindowRgn(hwnd, region), "window alpha region")
        return bool(G.PtInRegion(region, round((right - left) * x), round((bottom - top) * y)))
    finally:
        G.DeleteObject(region)


def menu(hwnd, key, tray=False, subkey=None):
    if not tray:
        activate(hwnd)
    thread = U.GetWindowThreadProcessId(hwnd, None)
    if tray:
        message(hwnd, 0x8001, 0x007B, 1)  # Tray WM_CONTEXTMENU, ID 1.
    else:
        message(hwnd, 0x205)  # Right button release.
    info = GuiInfo()
    info.size = c.sizeof(info)
    wait(lambda: U.GetGUIThreadInfo(thread, c.byref(info)) and info.flags & 0x10
         and info.menu == hwnd, "test pet owns popup menu")

    for mnemonic in (key, subkey):
        if mnemonic:
            require(U.PostMessageW(hwnd, 0x102, ord(mnemonic.lower()), 1), "post mnemonic to the test menu owner")
            time.sleep(0.12)
    wait(lambda: U.GetGUIThreadInfo(thread, c.byref(info)) and not info.flags & 0x10,
         f"menu command {key} closed the popup")


def activate(hwnd):
    active_thread = U.GetWindowThreadProcessId(U.GetForegroundWindow(), None)
    thread = K.GetCurrentThreadId()
    attached = bool(active_thread and thread != active_thread
                    and U.AttachThreadInput(thread, active_thread, True))
    try:
        U.BringWindowToTop(hwnd)
        U.SetForegroundWindow(hwnd)
    finally:
        if attached:
            U.AttachThreadInput(thread, active_thread, False)
    wait(lambda: U.GetForegroundWindow() == hwnd, "activate test foreground window")


@contextmanager
def fullscreen(monitor):
    b = monitor.bounds
    previous = U.GetForegroundWindow()
    hwnd = U.CreateWindowExW(8, "STATIC", "P2 fullscreen test", 0x90000000, b.left, b.top,
                             b.right - b.left, b.bottom - b.top, None, None, None, None)
    require(hwnd, "create borderless test window")
    try:
        activate(hwnd)
        yield
    finally:
        U.DestroyWindow(hwnd)
        if previous and U.IsWindow(previous):
            U.SetForegroundWindow(previous)


class Run:
    def __init__(self, output, exe, pack, name, monitor, mode="auto", expect_asset=True):
        self.log = output / f"{name}.log"
        config = output / f"{name}.toml"
        config.write_text(f"initial_scale_percent = 70\npack_path = {json.dumps(pack.as_posix(), ensure_ascii=False)}\n",
                          encoding="utf-8")
        env = dict(os.environ, DESKTOP_PET_TRACE="1", DESKTOP_PET_FORMAT="png")
        b = monitor.work
        env["DESKTOP_PET_REFERENCE_CLIENT"] = f"{mode},{b.left + 300},{b.top + 200}"
        self.expect_asset = expect_asset
        self.proc = None
        self.hwnd = None
        self.exe, self.config, self.env = exe, config, env

    def __enter__(self):
        with self.log.open("wb") as log:
            self.proc = subprocess.Popen([self.exe, "--dev-config", self.config],
                                         env=self.env, stdout=log, stderr=subprocess.STDOUT)
        try:
            self.hwnd = wait(lambda: find_window(self.proc.pid), f"window for {self.log.name}")
            self.until("First asset frame presented" if self.expect_asset else "First frame presented")
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
        self.until(f"P2 action: {name} ", offset, timeout)


def check_size(run, monitor, forced_dip=False):
    match = re.search(r"EDID mm (Some\(\[(\d+), (\d+)\]\)|None), initial DIP ([0-9.]+)", run.text())
    require(match, "startup size trace")
    _, width_mm, height_mm, dip = match.groups()
    dpi = U.GetDpiForWindow(run.hwnd)
    left, top, right, bottom = bounds(run.hwnd)
    expected = round(float(dip) * dpi / 96)
    require(right - left == bottom - top == expected, f"startup size {bounds(run.hwnd)} expected {expected}")
    require(get_monitor(U.MonitorFromWindow(run.hwnd, 2)).device == monitor.device, "startup target monitor")
    if forced_dip:
        require(height_mm is None and float(dip) == 179.2, "DIP fallback must use 256 * 70%")
    elif height_mm:
        height = int(height_mm)
        expected = round(max(36, min(90, 0.16 * height)) * (monitor.bounds.bottom - monitor.bounds.top)
                         / height * 0.7)
        require(right - left == expected, "EDID-based size")
    return {"device": monitor.device, "edid_mm": [int(width_mm), int(height_mm)] if height_mm else None,
            "dpi": dpi, "dip": float(dip), "size_px": right - left}


def exercise(run, monitor, second):
    hwnd = run.hwnd
    work = monitor.work
    run.action("idle")
    mark = len(run.text())
    click(hwnd)
    time.sleep(0.12)
    require("P2 action: interact " not in run.text()[mark:], "interaction must wait for double-click window")
    run.action("interact", mark)
    require(hit(hwnd, 0.625, 0.37) and not hit(hwnd, 0.50, 0.70), "transformed alpha hit region")
    run.action("idle", mark)

    mark = len(run.text())
    click(hwnd)
    time.sleep(0.075)
    click(hwnd)
    run.until("P2 quota: connection required", mark)
    time.sleep(0.3)
    require("P2 action: interact " not in run.text()[mark:], "double-click must suppress interaction")

    # A manual movement preview plays in place even after autonomous movement is disabled.
    menu(hwnd, "M")
    mark = len(run.text())
    initial = bounds(hwnd)
    menu(hwnd, "A", subkey="L")
    run.action("move-left", mark)
    run.action("idle", mark)
    require(bounds(hwnd) == initial, "manual movement preview moved the window")

    for side, x in (("left", work.left + 4), ("right", work.right - (initial[2] - initial[0]) - 4)):
        mark = len(run.text())
        drag(hwnd, x, work.top + 160)
        run.action(f"edge-{side}", mark)
        edge_x = work.left if side == "left" else work.right - (initial[2] - initial[0])
        wait(lambda: bounds(hwnd)[0] == edge_x, f"{side} snap position")
        click(hwnd)
        run.action(f"peek-{side}", mark)
        mark = len(run.text())
        click(hwnd)
        time.sleep(0.07)
        click(hwnd)
        run.until("P2 quota: connection required", mark)
        require(bounds(hwnd)[0] == edge_x, "double-click detached pet")
        menu(hwnd, "E")
        wait(lambda: bounds(hwnd)[0] != edge_x, "expand from edge")

    # Release of a cancelled drag must not snap or trigger a click.
    mark = len(run.text())
    drag(hwnd, work.left + 4, work.top + 130, release=False)
    message(hwnd, 0x1F)
    message(hwnd, 0x202)
    time.sleep(0.35)
    require("P2 drop:" not in run.text()[mark:] and "P2 action: interact" not in run.text()[mark:],
            "cancelled drag committed a drop or click")

    if second:
        b = second.work
        drag(hwnd, b.left + 280, b.top + 180)
        wait(lambda: get_monitor(U.MonitorFromWindow(hwnd, 2)).device == second.device, "cross-monitor drag")
        run.action("land", mark)
        current = bounds(hwnd)
        require(b.left <= current[0] < current[2] <= b.right
                and b.top <= current[1] < current[3] <= b.bottom, "second monitor work area")
        initial_dip = float(re.search(r"initial DIP ([0-9.]+)", run.text())[1])
        require(abs((current[2] - current[0]) - initial_dip * U.GetDpiForWindow(hwnd) / 96) <= 1,
                "DIP size not preserved after cross-monitor drag")
        drag(hwnd, work.left + 350, work.top + 200)

    mark = len(run.text())
    menu(hwnd, "H")
    wait(lambda: not U.IsWindowVisible(hwnd), "manual hide")
    with fullscreen(monitor):
        time.sleep(0.4)
    require(not U.IsWindowVisible(hwnd), "leaving fullscreen cleared manual hide")
    menu(hwnd, "H", tray=True)
    wait(lambda: U.IsWindowVisible(hwnd), "tray restore")
    point = bounds(hwnd)
    if second:
        with fullscreen(second):
            time.sleep(0.4)
            require(U.IsWindowVisible(hwnd), "fullscreen on another monitor hid the pet")
    with fullscreen(monitor):
        wait(lambda: not U.IsWindowVisible(hwnd), "automatic fullscreen hide")
        before = len(run.text())
        time.sleep(0.45)
        require("P1b frame" not in run.text()[before:], "hidden animation submitted frames")
        require(bounds(hwnd) == point, "pet moved while hidden")
    wait(lambda: U.IsWindowVisible(hwnd), "restore after fullscreen")
    require(bounds(hwnd) == point, "resuming caused a position jump")
    require("P1b visibility: false" in run.text()[mark:], "visibility trace")

    # Observe a complete route, then interrupt the next one after the 10s cooldown.
    menu(hwnd, "M")
    mark = len(run.text())
    point = bounds(hwnd)
    wait(lambda: bounds(hwnd)[0] != point[0], "autonomous movement", timeout=17)
    action = re.search(r"P2 action: move-(left|right)", run.text()[mark:])
    require(action, "directional move asset")
    run.action("idle", mark)
    completed = bounds(hwnd)
    distance = round(144 * U.GetDpiForWindow(hwnd) / 96)
    delta = -distance if action[1] == "left" else distance
    require(completed[0] == point[0] + delta and completed[1] == point[1], "route distance/direction")
    finished = time.monotonic()
    mark = len(run.text())
    wait(lambda: bounds(hwnd)[0] != completed[0], "second autonomous movement", timeout=12)
    require(time.monotonic() - finished >= 9.5, "autonomous cooldown was bypassed")
    current = bounds(hwnd)
    drag(hwnd, current[0] + 28, current[1] + 25)
    run.action("land", mark)
    settled = bounds(hwnd)
    time.sleep(1.5)
    require(bounds(hwnd) == settled, "interrupted route resumed after drag")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="Windows Release with p1b-reference")
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    primary, *others = displays()
    secondary = next(iter(others), None)
    pack = args.fixtures.resolve() / "p2-behavior"
    kwargs = (args.output, args.exe.resolve())
    report = {"display_measurements": [], "checks": []}
    with Run(*kwargs, pack, "behavior", primary) as run:
        report["display_measurements"].append(check_size(run, primary))
        exercise(run, primary, secondary)
        report["checks"].append("behavior")
    if secondary:
        with Run(*kwargs, pack, "secondary", secondary) as run:
            report["display_measurements"].append(check_size(run, secondary))
    with Run(*kwargs, args.fixtures.resolve() / "p2-idle-only", "sparse-dip", primary, "dip") as run:
        report["display_measurements"].append(check_size(run, primary, forced_dip=True))
        mark = len(run.text())
        click(run.hwnd)
        time.sleep(0.45)
        require("P2 action: interact" not in run.text()[mark:], "missing optional action")
        require("P1b frame" in run.text()[mark:], "idle continued after unavailable interaction")
        report["checks"].append("missing optional action")
    broken = args.output.resolve() / "broken-page"
    shutil.copytree(pack, broken)
    (broken / "interact-1.png").write_bytes(b"intentionally invalid")
    with Run(*kwargs, broken, "broken-page", primary) as run:
        mark = len(run.text())
        click(run.hwnd)
        run.action("interact", mark)
        run.until("Action interact unavailable", mark)
        run.action("idle", mark)
        report["checks"].append("corrupt later page falls back to idle")
    (broken / "idle-0.png").write_bytes(b"intentionally invalid")
    with Run(*kwargs, broken, "broken-idle", primary, expect_asset=False) as run:
        run.until("Asset playback stopped; using placeholder")
        require("First asset frame presented" not in run.text(), "corrupt idle was presented")
        report["checks"].append("corrupt required idle falls back to placeholder")
    (args.output / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
