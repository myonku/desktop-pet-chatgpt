"""Validate the Windows settings center and quick panel on an interactive desktop."""

import argparse
from collections import Counter
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time
import tomllib

import psutil
import uiautomation as ui

from windows_desktop import (
    U, QUICK, SETTINGS, activate, bounds, displayed, drag, find_window, get_monitor, invoke, menu,
    message, open_quick, open_settings_from_quick, panel, require, wait,
)

U.IsIconic.argtypes = [w.HWND]
U.ShowWindowAsync.argtypes = [w.HWND, c.c_int]


def sample(parent):
    rows = []
    for proc in [parent, *parent.children()]:
        try:
            with proc.oneshot():
                memory = proc.memory_info()
                ticks = proc.cpu_times()
                rows.append(dict(pid=proc.pid, private=memory.private, rss=memory.rss,
                                 cpu=ticks.user + ticks.system, handles=proc.num_handles(),
                                 threads=proc.num_threads(), switches=sum(proc.num_ctx_switches())))
        except psutil.NoSuchProcess:
            continue
    return {key: sum(row[key] for row in rows) for key in
            ("private", "rss", "cpu", "handles", "threads", "switches")} | {"processes": rows}


def measure_open(pet, parent, read_log, tray):
    mark = len(read_log())
    started = time.perf_counter()
    if tray:
        menu(pet, "s", tray=True)
    else:
        open_settings_from_quick(pet)
    marker = "WinUI settings ready"
    wait(lambda: marker in read_log()[mark:], marker, timeout=10)
    hwnd = wait(lambda: panel(pet), "settings becomes visible")
    owner = next(child for child in parent.children() if find_window(child.pid, SETTINGS) == hwnd)
    return owner, hwnd, 1000 * (time.perf_counter() - started)


def functional(pet, parent, config, read_log):
    _, hwnd, _ = measure_open(pet, parent, read_log, False)
    require(U.GetWindowLongW(hwnd, -16) & (0x00040000 | 0x00010000) == 0,
            "settings frame has no resize or maximize style")
    root = ui.ControlFromHandle(hwnd)
    original = bounds(pet)
    slider = root.SliderControl(AutomationId="pet-scale").GetPattern(ui.PatternId.RangeValuePattern)
    slider.SetValue(130)
    wait(lambda: bounds(pet)[2] - bounds(pet)[0] > original[2] - original[0], "preview resize")
    message(hwnd, 0x10)
    wait(lambda: not parent.children(), "preview panel closes")
    wait(lambda: bounds(pet)[2] - bounds(pet)[0] == original[2] - original[0],
         "closing reverts preview")

    owner, hwnd, _ = measure_open(pet, parent, read_log, True)
    root = ui.ControlFromHandle(hwnd)
    activate(hwnd)
    U.ShowWindowAsync(hwnd, 6)
    wait(lambda: U.IsIconic(hwnd), "panel minimizes")
    menu(pet, "s", tray=True)
    wait(lambda: not U.IsIconic(hwnd), "settings menu restores the same window")
    reuse_focused = U.GetForegroundWindow() == hwnd
    require([p.pid for p in parent.children()] == [owner.pid], "settings reuses its child")

    slider = root.SliderControl(AutomationId="pet-scale").GetPattern(ui.PatternId.RangeValuePattern)
    slider.SetValue(115)
    root.ButtonControl(Name="应用并保存").GetPattern(ui.PatternId.InvokePattern).Invoke()
    wait(lambda: tomllib.loads(config.read_text(encoding="utf-8")).get("saved_size", {}).get("percent") == 115,
         "shared settings save")
    message(hwnd, 0x10)
    wait(lambda: not parent.children(), "settings child exits")
    return {"fixed_frame": True, "preview_reverted": True, "saved_scale": 115, "same_child_restored": True,
            "reopen_foreground": reuse_focused}


def check_quick(pet, parent, config, read_log):
    owner, hwnd, _ = measure_open(pet, parent, read_log, True)
    saved = lambda: tomllib.loads(config.read_text(encoding="utf-8"))
    root = ui.ControlFromHandle(hwnd)
    slider = root.SliderControl(AutomationId="pet-scale").GetPattern(ui.PatternId.RangeValuePattern)
    slider.SetValue(150)

    def ready_quick():
        mark = len(read_log())
        hwnd = open_quick(pet)
        wait(lambda: "WinUI quick panel ready" in read_log()[mark:], "quick hooks and placement dispatched")
        return hwnd

    quick = ready_quick()
    activate(quick)
    work = get_monitor(U.MonitorFromWindow(pet, 2)).work
    wait(lambda: (r := bounds(quick)) != bounds(pet)
         and work.left <= r[0] < r[2] <= work.right and work.top <= r[1] < r[3] <= work.bottom,
         "quick panel inside pet monitor work area")
    quick_root = ui.ControlFromHandle(quick)
    require(not quick_root.ButtonControl(Name="信息速览").Exists(0, 0), "overview is a separate gesture")
    require(not quick_root.CheckBoxControl(Name="自移动").Exists(0, 0), "settings are collapsed")
    scale_row = (quick_root.ButtonControl(Name="+"), quick_root.TextControl(Name="缩放"),
                 quick_root.ButtonControl(Name="-"))
    require(all(control.Exists(0, 0) for control in scale_row), "all scale controls are present")
    plus, label, minus = (control.BoundingRectangle for control in scale_row)
    require(plus.right <= label.left < label.right <= minus.left
            and max(plus.top, label.top, minus.top) < min(plus.bottom, label.bottom, minus.bottom),
            "scale controls share one row in +, label, - order")
    row_height = 34 * U.GetDpiForWindow(quick) // 96
    panel_bottom = bounds(quick)[3]
    for name in ("聊天窗口", "动作预览", "快捷设置"):
        row = quick_root.ButtonControl(Name=name).BoundingRectangle
        require(row.bottom - row.top >= row_height and row.bottom < panel_bottom,
                f"full-height {name} row fits inside the collapsed toolbar")
    invoke(quick, "+")
    wait(lambda: saved()["saved_size"]["percent"] == 125, "quick scale commits once")
    require(U.IsWindow(hwnd), "settings survives quick command")
    ql, qt, qr, qb = bounds(quick)
    ui.Click(ql + 4, (qt + qb) // 2)
    ui.SendKeys("{Esc}")
    require(U.IsWindow(quick), "blank and Escape keep the toolbar open")
    activate(hwnd)
    wait(lambda: not U.IsWindow(quick), "other window dismisses toolbar")

    invoke(hwnd, "应用并保存")
    require(root.ButtonControl(Name="覆盖冲突并保存").Exists(5), "size conflict is shown")
    require(saved()["saved_size"]["percent"] == 125, "conflict preserves the committed size")
    invoke(hwnd, "覆盖冲突并保存")
    wait(lambda: saved()["saved_size"]["percent"] == 150, "explicit conflict resolution")

    quick = ready_quick()
    activate(quick)
    quick_root = ui.ControlFromHandle(quick)
    base_height = bounds(quick)[3] - bounds(quick)[1]
    invoke(quick, "动作预览")
    require(quick_root.ButtonControl(Name="收起动作").Exists(5), "action submenu expanded")
    wait(lambda: bounds(quick)[3] - bounds(quick)[1] > base_height, "submenu fits inside the expanded toolbar")
    invoke(quick, "左侧探头")
    require(U.IsWindow(quick), "an action preview leaves the toolbar open")
    require(quick_root.ButtonControl(Name="展开桌宠").Exists(5), "dock adds the expand row")
    invoke(quick, "展开桌宠")
    wait(lambda: not quick_root.ButtonControl(Name="展开桌宠").Exists(0, 0), "expand clears dock row")
    invoke(quick, "收起动作")
    wait(lambda: bounds(quick)[3] - bounds(quick)[1] == base_height, "action menu collapses")
    invoke(quick, "快捷设置")
    require(quick_root.ButtonControl(Name="全部设置").Exists(5), "full settings is nested")
    invoke(quick, "音效", "CheckBox", "Toggle")
    wait(lambda: not saved()["preferences"]["sound"], "toolbar sound toggle persists")
    invoke(quick, "全屏隐藏", "CheckBox", "Toggle")
    wait(lambda: saved()["preferences"]["fullscreen_hide"], "toolbar fullscreen toggle persists")
    invoke(quick, "收起设置")
    message(pet, 0x205)
    wait(lambda: not U.IsWindow(quick), "second pet right-click closes toolbar")

    quick = ready_quick()
    activate(quick)
    before = bounds(pet)
    before_quick = bounds(quick)
    y = before[1] + (80 if before[3] + 80 < work.bottom else -80)
    x = before[0] + (90 if before[2] + 90 < work.right else -90)
    drag(pet, x, y)
    wait(lambda: bounds(quick) != before_quick, "toolbar follows pet drag")
    require(U.IsWindow(quick), "drag leaves toolbar open")
    message(quick, 0x205, 6, 35)
    wait(lambda: not U.IsWindow(quick), "right-click inside toolbar closes it")

    menu(pet, "m", tray=True)
    wait(lambda: saved()["preferences"]["auto_move"], "tray toggles auto move")
    require(U.IsWindow(hwnd), "tray command keeps settings window")
    invoke(hwnd, "应用并保存")
    require(saved()["preferences"]["auto_move"], "settings keeps untouched external fields")
    message(hwnd, 0x10)
    wait(lambda: not owner.is_running(), "settings shutdown reaps child")
    return {"quick_scale": True, "scale_single_row": True, "conflict": True,
            "blank_inside_persists": True, "focus_dismisses": True,
            "nested_menus": True, "right_click": True, "drag_follow": True, "shared_preferences": True}


def check_first_show(pet, parent, repeats):
    # Sample each dedicated quick-panel process from before its window is created.
    # Win32 visibility alone includes cloaked windows, so only record DWM-visible states.
    from threading import Event, Thread

    work = get_monitor(U.MonitorFromWindow(pet, 2)).work
    anchor = bounds(pet)
    dpi = U.GetDpiForWindow(pet)
    width = min(124 * dpi // 96, work.right - work.left)
    height = min(164 * dpi // 96, work.bottom - work.top)
    gap = 8 * dpi // 96
    left, top, right, bottom = anchor
    if right + gap + width <= work.right:
        x, y = right + gap, top
    elif left - gap - width >= work.left:
        x, y = left - gap - width, top
    elif top - gap - height >= work.top:
        x, y = left, top - gap - height
    else:
        x, y = left, bottom + gap
    x = min(max(x, work.left), work.right - width)
    y = min(max(y, work.top), work.bottom - height)
    expected = (x, y, x + width, y + height)

    evidence = []
    for _ in range(repeats):
        samples = set()
        stop = Event()

        def sample_window():
            child = None
            while not stop.is_set():
                if child is None:
                    child = next(iter(parent.children()), None)
                if child and (hwnd := find_window(child.pid, QUICK)) and displayed(hwnd):
                    try:
                        samples.add((bounds(hwnd), U.GetWindowLongW(hwnd, -16) & 0xc00000,
                                     U.GetWindowLongW(hwnd, -20) & 0x00040080))
                    except AssertionError:
                        if U.IsWindow(hwnd):
                            raise
                stop.wait(.001)

        watch = Thread(target=sample_window)
        watch.start()
        try:
            hwnd = open_quick(pet)
            time.sleep(.1)
        finally:
            stop.set()
            watch.join()
        require(samples == {(expected, 0, 0x80)},
                f"every visible frame must be anchored and borderless: {sorted(samples)} versus {expected}")
        evidence.append({"rect": expected, "caption": False, "tool_window": True})
        message(hwnd, 0x10)
        wait(lambda: not parent.children(), "close sampled quick panel")
    return evidence

def startup_timeout(pet, parent, read_log):
    # Start the child, then suspend it before it can answer the handshake.
    mark = len(read_log())
    held = []

    def suspend_on_launch():
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            if children := parent.children():
                child = children[0]
                child.suspend()
                held.append(child)
                return
            time.sleep(0.002)

    from threading import Thread
    thread = Thread(target=suspend_on_launch)
    thread.start()
    try:
        menu(pet, "s", tray=True)
        thread.join()
        require(held, "suspended the test panel process")
        wait(lambda: "WinUI settings startup timed out" in read_log()[mark:], "WinUI handshake deadline", 8)
        wait(lambda: "无法打开设置中心" in read_log()[mark:], "runtime failure feedback")
        require(not panel(pet), "no fallback settings panel")
        measure_open(pet, parent, read_log, True)
        require(not held[0].is_running(), "timed-out child reaped")
    finally:
        thread.join()
        if held and held[0].is_running():
            held[0].resume()


def run(args):
    output = args.output
    profile = output / "profile/DesktopPetChatGPT"
    profile.mkdir(parents=True)
    config = profile / "config.toml"
    config.write_text(f"pack_path = {json.dumps(args.pack.as_posix(), ensure_ascii=False)}\n"
                      "[preferences]\nauto_move = false\nfullscreen_hide = false\n", encoding="utf-8")
    env = dict(os.environ, LOCALAPPDATA=str(profile.parent), DESKTOP_PET_TRACE="1")
    log = output / "playback.log"

    def read_log():
        return log.read_text(encoding="utf-8", errors="replace")

    with log.open("wb") as out, subprocess.Popen([args.exe], cwd=output, env=env, stderr=out) as proc:
        parent = psutil.Process(proc.pid)
        try:
            pet = wait(lambda: find_window(proc.pid), "pet window")
            wait(lambda: "First asset frame presented" in read_log(), "same pack starts")
            time.sleep(0.5)
            baseline = sample(parent)
            cycles = []
            for index in range(args.cycles):
                mark = len(read_log())
                owner, hwnd, latency = measure_open(pet, parent, read_log, index % 2)
                time.sleep(0.3)
                opened = sample(parent)
                time.sleep(0.45)
                settled = sample(parent)
                rectangle, dpi = bounds(hwnd), U.GetDpiForWindow(hwnd)
                foreground = U.GetForegroundWindow() == hwnd
                message(hwnd, 0x10)
                wait(lambda: not find_window(owner.pid, SETTINGS), "settings closed")
                wait(lambda: not parent.children(), "WinUI child exits")
                time.sleep(0.2)
                cycles.append(dict(open_ms=round(latency, 2), opened=opened, settled=settled,
                                   closed=sample(parent), rect=rectangle, dpi=dpi, foreground=foreground,
                                   frames=read_log()[mark:].count("Playback frame ")))

            menu(pet, "h", tray=True)
            wait(lambda: not U.IsWindowVisible(pet), "hide via tray")
            mark = len(read_log())
            time.sleep(5)
            hidden = sample(parent)
            require("Playback frame " not in read_log()[mark:], "hidden pet remains idle")
            menu(pet, "h", tray=True)
            wait(lambda: U.IsWindowVisible(pet) and "Playback frame " in read_log()[mark:], "restore via tray")
            restored = sample(parent)
            result = dict(baseline=baseline, cycles=cycles, hidden=hidden, restored=restored)
            if args.functional:
                result["functional"] = functional(pet, parent, config, read_log)
                result["quick"] = check_quick(pet, parent, config, read_log)
                menu(pet, "m", tray=True)
                wait(lambda: not tomllib.loads(config.read_text(encoding="utf-8"))["preferences"]["auto_move"],
                     "pause movement for first-show measurements")
                result["quick_first_show"] = check_first_show(pet, parent, args.cycles)
            if args.timeout:
                startup_timeout(pet, parent, read_log)
                result["startup_timeout_reaped"] = True
            else:
                owner, _, _ = measure_open(pet, parent, read_log, False)
            message(pet, 0x10)
            require(proc.wait(timeout=8) == 0, "pet exits from main window")
            if not args.timeout:
                wait(lambda: not owner.is_running(), "no child survives parent exit")
                result["exit_with_open_settings"] = True
            return result
        finally:
            if proc.poll() is None:
                proc.kill()


def blocked_transport(args):
    output = args.output / "blocked-transport"
    profile = output / "profile/DesktopPetChatGPT"
    profile.mkdir(parents=True)
    (profile / "config.toml").write_text(
        f"pack_path = {json.dumps(args.pack.as_posix(), ensure_ascii=False)}\n"
        "[preferences]\nauto_move = false\nfullscreen_hide = false\n", encoding="utf-8")
    env = dict(os.environ, LOCALAPPDATA=str(profile.parent), DESKTOP_PET_TRACE="1",
               DESKTOP_PET_TEST_BLOCK_PANEL_INPUT="1")
    log = output / "playback.log"

    def read_log():
        return log.read_text(encoding="utf-8", errors="replace")

    with log.open("wb") as stream, subprocess.Popen([args.exe], cwd=output, env=env, stderr=stream) as proc:
        try:
            parent = psutil.Process(proc.pid)
            pet = wait(lambda: find_window(proc.pid), "pet window for blocked transport")
            wait(lambda: "First asset frame presented" in read_log(), "formal pack starts")
            owner, settings, _ = measure_open(pet, parent, read_log, True)
            # The diagnostic child handles its window but deliberately never reads the poster.
            menu(pet, "h", tray=True)
            require(not U.IsWindowVisible(pet), "tray hides pet while settings stops reading")
            require(owner.is_running() and U.IsWindow(settings), "tray remains available before timeout")
            menu(pet, "h", tray=True)
            require(U.IsWindowVisible(pet), "tray restores pet while settings stops reading")
            wait(lambda: "WinUI settings transport stalled" in read_log(), "write deadline", timeout=8)
            wait(lambda: not parent.children(), "blocked child reaped")
            quick_window = open_quick(pet)
            require(U.IsWindow(quick_window), "independent quick panel opens after transport failure")
            message(pet, 0x10)
            require(proc.wait(timeout=8) == 0, "pet exits after transport failure")
            return {"tray_responsive": True, "blocked_child_reaped": True, "quick_reopens": True}
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait(timeout=5)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("exe", "pack", "output"):
        parser.add_argument(f"--{option}", required=True, type=Path)
    parser.add_argument("--cycles", type=int, default=20)
    parser.add_argument("--functional", action="store_true")
    parser.add_argument("--blocked-transport", action="store_true", help="stall reads after the settings handshake")
    parser.add_argument("--timeout", action="store_true", help="suspend one child to test startup fallback")
    args = parser.parse_args()
    for name in ("exe", "pack", "output"):
        setattr(args, name, getattr(args, name).resolve())
    require(args.cycles > 0, "at least one cycle required")
    args.output.mkdir(parents=True, exist_ok=False)
    evidence = run(args)
    if args.blocked_transport:
        evidence["blocked_transport"] = blocked_transport(args)
    cycles = evidence["cycles"]
    report = dict(exe_sha256=digest(args.exe), pack_sha256=digest(args.pack / "manifest.json"),
                  open_first_ms=cycles[0]["open_ms"],
                  open_median_ms=statistics.median(cycle["open_ms"] for cycle in cycles),
                  focus_results=dict(Counter(str(c["foreground"]) for c in cycles)),
                  **evidence)
    (args.output / "result.json").write_text(
        json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")
    print(report["open_first_ms"], report["open_median_ms"], report["focus_results"], flush=True)


if __name__ == "__main__":
    main()
