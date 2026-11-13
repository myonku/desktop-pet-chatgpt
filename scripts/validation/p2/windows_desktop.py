"""Win32 helpers for tests that operate on their own windows."""

from contextlib import contextmanager
import ctypes as c
from ctypes import wintypes as w
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


def find_window(pid, caption=TITLE):
    handles = []

    @ENUM_WINDOW
    def collect(hwnd, _arg):
        owner = w.DWORD()
        U.GetWindowThreadProcessId(hwnd, c.byref(owner))
        if owner.value == pid:
            title = c.create_unicode_buffer(256)
            U.GetWindowTextW(hwnd, title, len(title))
            if title.value == caption:
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
    hwnd = U.CreateWindowExW(8, "STATIC", "Desktop pet fullscreen test", 0x90000000, b.left, b.top,
                             b.right - b.left, b.bottom - b.top, None, None, None, None)
    require(hwnd, "create borderless test window")
    try:
        activate(hwnd)
        yield
    finally:
        U.DestroyWindow(hwnd)
        if previous and U.IsWindow(previous):
            U.SetForegroundWindow(previous)
