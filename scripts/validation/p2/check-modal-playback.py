"""Verify rendered pet frames advance inside actual Windows modal loops."""

import argparse
import ctypes as c
from pathlib import Path
import re

from window_scenarios import Run
from windows_desktop import GuiInfo, U, displays, find_window, menu, message, require, wait


def advances(run, label):
    mark = len(run.text())

    def presented():
        stamps = [int(value) for value in re.findall(r"Playback frame \d+ \d+ (\d+)", run.text()[mark:])]
        return stamps if len(stamps) >= 10 and stamps[-1] - stamps[0] >= 750_000 else None

    stamps = wait(presented, label, timeout=3)
    max_gap = max(b - a for a, b in zip(stamps, stamps[1:]))
    require(max_gap <= 250_000, f"{label}: gap of {max_gap / 1000:.0f} ms")
    return {"frames": len(stamps), "max_gap_ms": max_gap / 1000}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("exe", "pack", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    with Run(args.output.resolve(), args.exe.resolve(), args.pack.resolve(), "modal", displays()[0]) as run:
        menu(run.hwnd, "S", during=lambda: print("Character menu:", advances(run, "character popup playback")))
        settings = wait(lambda: find_window(run.proc.pid, "桌宠 · 设置"), "settings window")
        thread = U.GetWindowThreadProcessId(settings, None)
        info = GuiInfo()
        info.size = c.sizeof(info)
        # SC_MOVE from the keyboard enters the native loop without holding a real mouse button.
        require(U.PostMessageW(settings, 0x0112, 0xF010, -1), "post SC_MOVE")
        try:
            wait(lambda: U.GetGUIThreadInfo(thread, c.byref(info)) and info.flags & 0x02
                 and info.move == settings, "native settings move loop")
            print("Settings move:", advances(run, "settings move playback"))
            require(U.GetGUIThreadInfo(thread, c.byref(info)) and info.move == settings,
                    "move loop still active during playback")
        finally:
            message(settings, 0x001F)
        wait(lambda: U.GetGUIThreadInfo(thread, c.byref(info)) and not info.flags & 0x02,
             "settings move loop exit")
        message(settings, 0x0010)
        menu(run.hwnd, "M", tray=True, during=lambda: print("Tray menu:", advances(run, "tray popup playback")))


if __name__ == "__main__":
    main()
