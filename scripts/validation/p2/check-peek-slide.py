"""Exercise travel, edge pose slides, and drag-off ordering on Windows."""

import argparse
import ctypes as c
from ctypes import wintypes as w
from pathlib import Path
import re
import time

from window_scenarios import Run
from windows_desktop import G, U, bounds, displays, drag, menu, message, require, wait


G.GetRgnBox.argtypes = [w.HRGN, c.POINTER(w.RECT)]


def silhouette(hwnd):
    region = G.CreateRectRgn(0, 0, 0, 0)
    rect = w.RECT()
    try:
        if not U.GetWindowRgn(hwnd, region) or G.GetRgnBox(region, c.byref(rect)) < 2:
            return None
        for y in range(rect.top, rect.bottom, 2):
            for x in range(rect.left, rect.right, 2):
                if G.PtInRegion(region, x, y):
                    return rect.right - rect.left, x, y
    finally:
        G.DeleteObject(region)
    return None


def toggle(run, target, expanding):
    hwnd = run.hwnd
    window = bounds(hwnd)
    initial, x, y = wait(lambda: silhouette(hwnd), "visible hit region")
    mark = len(run.text())
    message(hwnd, 0x201, x, y, 1)
    message(hwnd, 0x202, x, y)
    run.action(target, mark)
    samples = []
    until = time.monotonic() + 0.35
    while time.monotonic() < until:
        require(bounds(hwnd) == window, "peek moved the snapped window")
        if sample := silhouette(hwnd):
            samples.append(sample[0])
        time.sleep(0.007)
    final = samples[-1]
    travel = (final - initial) * (1 if expanding else -1)
    require(travel > 15, f"visible width changed by only {travel} pixels")
    low, high = sorted((initial, final))
    middle = {width for width in samples if low + 7 <= width <= high - 7}
    require(len(middle) >= 3, f"missing intermediate widths: {sorted(set(samples))}")
    before, after = run.text()[mark:].split(f"Playback action: {target} ", 1)
    last = re.findall(r"Playback frame (\d+)", run.text()[:mark] + before)
    first = re.search(r"Playback frame (\d+)", after)
    require(last and first and 0 <= int(first[1]) - int(last[-1]) <= 2,
            f"switch to {target} lost the idle phase")
    print(target, f"{initial} -> {final} px, {len(middle)} intermediate widths, phase {last[-1]} -> {first[1]}")
    return travel


def preview_at_edge(run, side, peek, key, edge_x):
    hwnd = run.hwnd
    start = bounds(hwnd)
    mark = len(run.text())
    menu(hwnd, "A", subkey=key)
    target = f"{'peek' if peek else 'edge'}-{side}"
    low, high = sorted((start[0], edge_x))
    wait(lambda: low < bounds(hwnd)[0] < high, f"{target} in transit")
    require(f"Playback action: {target} " not in run.text()[mark:], "pose started in transit")
    wait(lambda: bounds(hwnd)[0] == edge_x, f"{target} reached edge")
    run.action(target, mark)
    require(bounds(hwnd)[:2] == (edge_x, start[1]), "preview changed vertical position")
    if peek:
        log = run.text()[mark:]
        require(0 <= log.find(f"Playback action: edge-{side} ") < log.find(f"Playback action: {target} "),
                "peek preview did not start with the snapped pose")
    print(target, f"menu preview travelled from x={start[0]} to x={edge_x}")


def snap(run, side, edge_x, y, inset):
    hwnd = run.hwnd
    mark = len(run.text())
    drag(hwnd, edge_x + inset, y)
    start_x = edge_x + inset
    low, high = sorted((start_x, edge_x))
    wait(lambda: low < bounds(hwnd)[0] < high, f"{side} snap in transit")
    require(f"Playback action: edge-{side} " not in run.text()[mark:], "edge appeared in transit")
    wait(lambda: bounds(hwnd)[0] == edge_x, f"{side} snap arrived")
    run.action(f"edge-{side}", mark)
    widths = []
    until = time.monotonic() + 0.4
    while time.monotonic() < until:
        if sample := silhouette(hwnd):
            widths.append(sample[0])
        time.sleep(0.003)
    require(len(widths) > 2 and widths[0] - widths[-1] > 15
            and len({w for w in widths if widths[-1] + 5 < w < widths[0] - 5}) >= 2,
            f"edge entry did not slide: {sorted(set(widths))}")
    print(f"edge-{side} entry {widths[0]} -> {widths[-1]} px")


def drag_off(run, x, y):
    hwnd = run.hwnd
    mark = len(run.text())
    _, grab_x, grab_y = wait(lambda: silhouette(hwnd), "visible edge hit region")
    start_x, start_y, _, _ = bounds(hwnd)
    message(hwnd, 0x201, grab_x, grab_y, 1)
    message(hwnd, 0x200, x + grab_x - start_x, y + grab_y - start_y, 1)
    # Release immediately to also exercise the pending drag's queued drop.
    message(hwnd, 0x202, grab_x, grab_y)
    wait(lambda: bounds(hwnd)[:2] != (start_x, start_y), "drag left the edge")
    _, _, after = run.text()[mark:].partition("Playback action: drag ")
    require(re.search(r"Playback frame \d+", after), "window moved before drag presentation")
    run.action("land", mark)
    require(bounds(hwnd)[:2] == (x, y), "queued drop lost the drag destination")
    print(f"drag presented before moving from x={start_x} to x={x}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("exe", "pack", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--half-travel", action="store_true", help="check the reduced 11.5% exposure difference")
    args = parser.parse_args()
    monitor = displays()[0]
    args.output.mkdir(parents=True, exist_ok=False)
    with Run(args.output.resolve(), args.exe.resolve(), args.pack.resolve(), "peek-slide", monitor) as run:
        hwnd = run.hwnd
        run.until("Playback frame 4 ")
        menu(hwnd, "M")
        for side in ("right", "left"):
            left, _, right, _ = bounds(hwnd)
            edge_x = monitor.work.left if side == "left" else monitor.work.right - (right - left)
            mark = len(run.text())
            y = monitor.work.top + 160
            inward = 1 if side == "left" else -1
            snap(run, side, edge_x, y, 12 * inward)
            run.until("Playback frame 20 ", mark)
            out = toggle(run, f"peek-{side}", expanding=True)
            back = toggle(run, f"edge-{side}", expanding=False)
            if args.half_travel:
                require(all(0.10 <= value / (right - left) <= 0.19 for value in (out, back)),
                        "slide distance differs from the half-travel candidate")
            if side == "left":
                toggle(run, f"peek-{side}", expanding=True)
            drag_off(run, edge_x + 100 * inward, y + 20)
        width = bounds(hwnd)[2] - bounds(hwnd)[0]
        for side, peek, key in (("right", False, "2"), ("left", True, "3"),
                                ("right", True, "4"), ("left", False, "1")):
            edge_x = monitor.work.left if side == "left" else monitor.work.right - width
            preview_at_edge(run, side, peek, key, edge_x)


if __name__ == "__main__":
    main()
