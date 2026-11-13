"""Exercise single-display pet behavior in an interactive Windows session."""

import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import time

from window_scenarios import Run, check_size
from windows_desktop import U, bounds, click, displays, drag, fullscreen, hit, menu, message, require, wait


def exercise(run, monitor):
    hwnd = run.hwnd
    work = monitor.work
    run.action("idle")
    mark = len(run.text())
    click(hwnd)
    time.sleep(0.12)
    require("Playback action: interact " not in run.text()[mark:], "interaction must wait for double-click window")
    run.action("interact", mark)
    require(hit(hwnd, 0.625, 0.37) and not hit(hwnd, 0.50, 0.70), "transformed alpha hit region")
    run.action("idle", mark)

    mark = len(run.text())
    click(hwnd)
    time.sleep(0.075)
    click(hwnd)
    run.until("Quota: connection required", mark)
    time.sleep(0.3)
    require("Playback action: interact " not in run.text()[mark:], "double-click must suppress interaction")

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
        run.until("Quota: connection required", mark)
        require(bounds(hwnd)[0] == edge_x, "double-click detached pet")
        menu(hwnd, "E")
        wait(lambda: bounds(hwnd)[0] != edge_x, "expand from edge")

    # Release of a cancelled drag must not snap or trigger a click.
    mark = len(run.text())
    drag(hwnd, work.left + 4, work.top + 130, release=False)
    message(hwnd, 0x1F)
    message(hwnd, 0x202)
    time.sleep(0.35)
    require("Drop:" not in run.text()[mark:] and "Playback action: interact" not in run.text()[mark:],
            "cancelled drag committed a drop or click")

    mark = len(run.text())
    menu(hwnd, "H")
    wait(lambda: not U.IsWindowVisible(hwnd), "manual hide")
    with fullscreen(monitor):
        time.sleep(0.4)
    require(not U.IsWindowVisible(hwnd), "leaving fullscreen cleared manual hide")
    menu(hwnd, "H", tray=True)
    wait(lambda: U.IsWindowVisible(hwnd), "tray restore")
    point = bounds(hwnd)
    with fullscreen(monitor):
        wait(lambda: not U.IsWindowVisible(hwnd), "automatic fullscreen hide")
        before = len(run.text())
        time.sleep(0.45)
        require("Playback frame" not in run.text()[before:], "hidden animation submitted frames")
        require(bounds(hwnd) == point, "pet moved while hidden")
    wait(lambda: U.IsWindowVisible(hwnd), "restore after fullscreen")
    require(bounds(hwnd) == point, "resuming caused a position jump")
    require("Window visibility: false" in run.text()[mark:], "visibility trace")

    # Observe a complete route, then interrupt the next one after the 10s cooldown.
    menu(hwnd, "M")
    mark = len(run.text())
    point = bounds(hwnd)
    wait(lambda: bounds(hwnd)[0] != point[0], "autonomous movement", timeout=17)
    action = re.search(r"Playback action: move-(left|right)", run.text()[mark:])
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
    parser.add_argument("--exe", type=Path, required=True, help="Windows Release with window-diagnostics")
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    primary = displays()[0]
    pack = args.fixtures.resolve() / "p2-behavior"
    exe = args.output.resolve() / args.exe.name
    shutil.copy2(args.exe, exe)
    kwargs = (args.output, exe)
    report = {"scenario": "single-display", "display_measurements": [], "checks": []}
    with Run(*kwargs, pack, "behavior", primary) as run:
        report["display_measurements"].append(check_size(run, primary))
        exercise(run, primary)
        report["checks"].append("behavior")
    with Run(*kwargs, args.fixtures.resolve() / "p2-idle-only", "sparse-dip", primary, "dip") as run:
        report["display_measurements"].append(check_size(run, primary, forced_dip=True))
        mark = len(run.text())
        click(run.hwnd)
        time.sleep(0.45)
        require("Playback action: interact" not in run.text()[mark:], "missing optional action")
        require("Playback frame" in run.text()[mark:], "idle continued after unavailable interaction")
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
    run = Run(*kwargs, broken, "broken-idle", primary)
    with run.log.open("wb") as log:
        result = subprocess.run([run.exe], cwd=run.cwd, env=run.env,
                                stdout=log, stderr=subprocess.STDOUT, timeout=12)
    require(result.returncode != 0 and "没有可用素材包" in run.text()
            and "First frame presented" not in run.text(), "invalid startup packs must stop before presentation")
    report["checks"].append("corrupt required first page with no default pack stops startup")
    (args.output / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
