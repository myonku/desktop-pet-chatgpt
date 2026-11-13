"""Verify cross-monitor behavior on an extended Windows desktop."""

import argparse
import json
from pathlib import Path
import shutil
import time

from window_scenarios import Run, check_size
from windows_desktop import U, bounds, displays, drag, fullscreen, get_monitor, require, wait


def check_transition(run, primary, secondary, initial_dip):
    hwnd = run.hwnd
    work = secondary.work
    mark = len(run.text())
    drag(hwnd, work.left + 280, work.top + 180)
    wait(lambda: get_monitor(U.MonitorFromWindow(hwnd, 2)).device == secondary.device, "cross-monitor drag")
    run.action("land", mark)
    dpi = U.GetDpiForWindow(hwnd)
    expected = min(round(initial_dip * dpi / 96), work.right - work.left, work.bottom - work.top)

    def settled():
        left, top, right, bottom = bounds(hwnd)
        return (abs(right - left - expected) <= 1 and right - left == bottom - top
                and work.left <= left < right <= work.right
                and work.top <= top < bottom <= work.bottom)

    wait(settled, "retained DIP size and destination work area")
    with fullscreen(primary):
        time.sleep(0.5)
        require(U.IsWindowVisible(hwnd), "fullscreen on another monitor hid the pet")
    with fullscreen(secondary):
        wait(lambda: not U.IsWindowVisible(hwnd), "fullscreen on the current monitor hides the pet")
    wait(lambda: U.IsWindowVisible(hwnd), "restore on destination after fullscreen")
    left, _, right, _ = bounds(hwnd)
    return {"destination_dpi": dpi, "size_px": right - left}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="Windows Release with window-diagnostics")
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    monitors = displays()
    report = {"scenario": "multi-display", "available_displays": len(monitors)}
    if len(monitors) < 2:
        report.update(status="skipped", reason="An extended desktop with two active displays is required")
    else:
        primary, secondary = monitors[:2]
        pack = args.fixtures.resolve() / "p2-behavior"
        exe = output / args.exe.name
        shutil.copy2(args.exe, exe)
        with Run(output, exe, pack, "primary", primary) as run:
            first = check_size(run, primary)
            transition = check_transition(run, primary, secondary, first["dip"])
        with Run(output, exe, pack, "secondary", secondary) as run:
            second = check_size(run, secondary)
        report.update(status="passed", display_measurements=[first, second], transition=transition,
                      mixed_dpi="passed" if first["dpi"] != second["dpi"] else "skipped (same DPI)")
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
