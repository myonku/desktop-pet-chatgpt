"""Validate the Windows ChatGPT read-only flow on an interactive desktop."""

import argparse
import ctypes as c
from ctypes import wintypes as w
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import tomllib
from _ctypes import COMError

import psutil
import uiautomation as ui

from windows_desktop import (
    SETTINGS, U, activate, bounds, click, find_window, get_monitor, invoke, menu, message, open_quick,
    panel, require, wait,
)

OVERVIEW = "信息速览"
HUB = "聊天窗口"
READY = "已连接，尚未读取账户信息"
NO_SOURCE = "等待桌面客户端"
NOT_RUNNING = "已安装 · 当前会话中未运行"
DESKTOP_VERSION = r"^(?:正在运行 · |客户端版本：)([0-9.]+)$"
QUOTA_ROW = r"^(?:主窗口|次窗口|[1-9][0-9]* (?:分钟|小时|天))：已用 [0-9]+%"
QUOTA_RESET = r" · (?:重置 [0-9-]+ [0-9:]+ UTC|重置时间未提供)$"
REFRESH_TIMEOUT = 16  # Account and usage requests have their own deadlines.
U.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]


def text(hwnd, name=None, regex=None):
    try:
        control = ui.ControlFromHandle(hwnd).TextControl(Name=name, RegexName=regex)
        return control.Exists(0, 0)
    except COMError:
        return False


def quota_rows(hwnd):
    root = ui.ControlFromHandle(hwnd)
    labels = []
    for index in (1, 2):
        row = root.TextControl(RegexName=QUOTA_ROW + QUOTA_RESET,
                               foundIndex=index)
        if not row.Exists(0, 0):
            break
        labels.append(row.Name)
    return labels


def check_quota_overview(overview, details):
    root = ui.ControlFromHandle(overview)
    for label in details:
        period, usage = label.split("：", 1)
        percent, reset = usage.removeprefix("已用 ").split("% · ", 1)
        remaining = max(0, 100 - int(percent))
        if reset.startswith("重置 "):
            utc = datetime.strptime(reset, "重置 %Y-%m-%d %H:%M:%S UTC")
            local = utc.replace(tzinfo=timezone.utc).astimezone()
            reset = f"{local:%m-%d %H:%M} 重置"
        else:
            reset = "重置时间未知"
        bar = root.ProgressBarControl(Name=f"{period}剩余额度")
        require(bar.Exists(1) and bar.GetRangeValuePattern().Value == remaining,
                "progress bar matches the remaining percentage")
        require(text(overview, name=f"剩余 {remaining}% · {reset}"),
                "quota reset matches the system timezone")


def open_overview(pet):
    click(pet)
    click(pet)
    return wait(lambda: panel(pet, OVERVIEW), "double-click opens the overview", timeout=10)


def connection_toggle(settings):
    checkbox = ui.ControlFromHandle(settings).CheckBoxControl(AutomationId="chatgpt-connection")
    require(checkbox.Exists(5) and checkbox.Name == "连接到ChatGPT", "connection checkbox")
    return checkbox.GetTogglePattern()


def open_chat(hwnd, index=1):
    link = ui.ControlFromHandle(hwnd).HyperlinkControl(Name="打开聊天", foundIndex=index)
    require(link.Exists(5), f"open chat link #{index}")
    link.GetInvokePattern().Invoke()


def check_fixture(pet, overview, source, field):
    require(text(overview, name="已连接至ChatGPT") and text(overview, name="已连接至合成客户端"),
            "overview lists both clients")
    require(text(overview, regex=field), "fixture refresh keeps ChatGPT snapshot")
    open_chat(overview, 2)
    hub = wait(lambda: panel(pet, HUB), "open fixture details")
    wait(lambda: text(hub, name="订阅：fixture"), "direct navigation to fixture")
    require(not text(hub, regex=field), "details display only the selected source")
    require(not text(hub, regex=r"^Codex 额度"), "fixture details do not inherit Codex usage")
    invoke(hub, "ChatGPT")
    wait(lambda: text(hub, regex=field), "switch to ChatGPT details")
    overview = open_overview(pet)
    open_chat(overview, 2)
    wait(lambda: text(hub, name="订阅：fixture"), "existing hub navigates to requested source")
    invoke(hub, "刷新")
    wait(lambda: text(hub, name="正在刷新"), "fixture refresh in flight")
    invoke(hub, "清除缓存")
    wait(lambda: ui.ControlFromHandle(hub).ButtonControl(Name="刷新").IsEnabled,
         "cancelled fixture worker reaped")
    require(not text(hub, name="订阅：fixture"), "late fixture response discarded")
    invoke(hub, "刷新")
    wait(lambda: text(hub, name="正在刷新"), "fixture refresh during disconnect")
    source.unlink()
    wait(lambda: text(hub, name=NO_SOURCE), "fixture disconnect detected")
    require(not text(hub, name="订阅：fixture"), "disconnect clears old value")
    source.write_text("2", encoding="utf-8")
    wait(lambda: text(hub, name=READY), "fixture auto reconnects")
    invoke(hub, "刷新")
    wait(lambda: text(hub, name="订阅：fixture"), "fixture refresh after reconnect")
    invoke(hub, "ChatGPT")
    wait(lambda: text(hub, regex=field), "ChatGPT survived fixture clear and disconnect")
    return hub


def desktop_version(hwnd):
    try:
        control = ui.ControlFromHandle(hwnd).TextControl(RegexName=DESKTOP_VERSION)
        return re.fullmatch(DESKTOP_VERSION, control.Name)[1] if control.Exists(0, 0) else None
    except COMError:
        return None


def check_lifecycle(hub, field):
    input("Quit ChatGPT normally in this test session, then press Enter here. ")
    wait(lambda: text(hub, name=NOT_RUNNING) and text(hub, name=NO_SOURCE),
         "desktop exit detected and source disconnected", timeout=20)
    require(not text(hub, regex=field) and not text(hub, regex=r"^本地获取时间："),
            "desktop exit clears both value and acquisition time")
    require(text(hub, name="Codex 额度：—"), "desktop exit clears Codex usage")

    input("Launch ChatGPT in the same test session, then press Enter here. ")
    restarted_version = wait(
        lambda: desktop_version(hub) if text(hub, name=READY) else None,
        "desktop restarted and source reconnected", timeout=20,
    )
    require(not text(hub, regex=field) and not text(hub, regex=r"^本地获取时间："),
            "reconnection does not restore the old snapshot or query automatically")
    started = time.perf_counter()
    invoke(hub, "刷新")
    wait(lambda: text(hub, regex=field), "real account query after desktop restart",
         timeout=REFRESH_TIMEOUT)
    require(quota_rows(hub), "real Codex usage after desktop restart")
    restarted_refresh_ms = round((time.perf_counter() - started) * 1000)
    require(text(hub, regex=r"^本地获取时间：.* UTC$"), "new query has an acquisition time")
    return dict(exit_detected=True, snapshot_cleared=True, reconnect="automatic",
                refresh_after_restart=True, restarted_desktop_version=restarted_version,
                refresh_after_restart_ms=restarted_refresh_ms)


def run(exe, pack, output, needs_auth, include_fixture, lifecycle):
    profile = output / "profile/DesktopPetChatGPT"
    profile.mkdir(parents=True)
    config = profile / "config.toml"
    config.write_text(
        f"pack_path = {json.dumps(pack.as_posix(), ensure_ascii=False)}\n"
        "[preferences]\nauto_move = false\nfullscreen_hide = false\n",
        encoding="utf-8",
    )
    env = dict(os.environ, LOCALAPPDATA=str(profile.parent), DESKTOP_PET_TRACE="1")
    fixture = output / "fixture-source"
    if include_fixture:
        fixture.write_text("1", encoding="utf-8")
        env["DESKTOP_PET_TEST_CLIENT_SOURCE"] = str(fixture)
    if needs_auth:
        empty_home = output / "empty-codex-home"
        empty_home.mkdir()
        env["CODEX_HOME"] = str(empty_home)

    def persisted():
        return tomllib.loads(config.read_text(encoding="utf-8"))["preferences"].get("chatgpt_connection")

    with (output / "pet.log").open("wb") as log, subprocess.Popen(
        [exe], cwd=output, env=env, stderr=log
    ) as proc:
        parent = psutil.Process(proc.pid)
        try:
            pet = wait(lambda: find_window(proc.pid), "pet window", timeout=15)
            wait(lambda: U.IsWindowVisible(pet), "pet visible")
            started = time.perf_counter()
            overview = open_overview(pet)
            left, top, right, bottom = bounds(overview)
            work = get_monitor(U.MonitorFromWindow(pet, 2)).work
            require(work.left <= left < right <= work.right and work.top <= top < bottom <= work.bottom,
                    "overview is inside pet monitor work area")
            wait(lambda: text(overview, name="已连接至ChatGPT"), "ChatGPT connects by default",
                 timeout=20)
            initial_version = wait(lambda: desktop_version(overview), "ChatGPT desktop metadata")
            field = r"^订阅：—$" if needs_auth else r"^订阅：(?!fixture$)[a-z0-9_]+$"
            wait(lambda: text(overview, regex=r"^需认证" if needs_auth else field),
                 "account query runs automatically",
                 timeout=REFRESH_TIMEOUT)
            refresh_ms = round((time.perf_counter() - started) * 1000)
            require(not text(overview, regex=r"^来源："), "overview omits source metadata")
            require(not ui.ControlFromHandle(overview).ButtonControl(Name="连接").Exists(0, 0),
                    "overview has no separate enable button")
            if include_fixture:
                wait(lambda: text(overview, name="订阅：fixture"), "fixture refreshes automatically")

            if include_fixture:
                hub = check_fixture(pet, overview, fixture, field)
            else:
                open_chat(overview)
                hub = wait(lambda: panel(pet, HUB), "hub opened from overview")
            wait(lambda: not U.IsWindow(overview), "overview closed on expand")
            require(text(hub, regex=field), "hub shares current account result")
            acquisition = ui.ControlFromHandle(hub).TextControl(RegexName=r"^本地获取时间：.* UTC$")
            require(acquisition.Exists(1), "details show acquisition time")
            utc = datetime.strptime(acquisition.Name, "本地获取时间：%Y-%m-%d %H:%M:%S UTC")
            local = utc.replace(tzinfo=timezone.utc).astimezone()
            quota = quota_rows(hub)
            overview = open_overview(pet)
            require(text(overview, name=f"刷新于 {local:%H:%M:%S}"),
                    "overview shows the snapshot time in the system timezone")
            if needs_auth:
                require(not quota and text(overview, name="Codex 额度：—"),
                        "unauthenticated view has no Codex usage")
            else:
                require(quota and text(overview, name="Codex 额度"), "real Codex usage in overview")
                check_quota_overview(overview, quota)
            message(overview, 0x10)
            wait(lambda: not U.IsWindow(overview), "overview closed after checking the quota")
            lifecycle_result = check_lifecycle(hub, field) if lifecycle else None
            menu(pet, "h", tray=True)
            wait(lambda: not U.IsWindowVisible(pet), "tray hides pet")
            require(U.IsWindow(hub), "independent hub remains open")
            menu(pet, "c", tray=True)
            require(panel(pet, HUB) == hub, "tray reuses existing hub")
            monitor = get_monitor(U.MonitorFromWindow(hub, 2)).work
            position = monitor.left + 60, monitor.top + 60
            require(U.SetWindowPos(hub, None, *position, 0, 0, 0x15), "move hub")
            wait(lambda: bounds(hub)[:2] == position, "hub moved")
            invoke(hub, "清除缓存")
            wait(lambda: text(hub, name=READY), "clear resets the shared data")
            require(text(hub, name="Codex 额度：—") and not quota_rows(hub),
                    "clear removes all Codex usage")
            message(hub, 0x10)
            wait(lambda: not U.IsWindow(hub), "hub closed")
            menu(pet, "c", tray=True)
            hub = wait(lambda: panel(pet, HUB), "hub opens again from tray")
            wait(lambda: bounds(hub)[:2] == position, "hub restores position")
            message(hub, 0x10)
            wait(lambda: not U.IsWindow(hub), "reopened hub closed")

            menu(pet, "h", tray=True)
            wait(lambda: U.IsWindowVisible(pet), "tray recovers pet")
            quick = open_quick(pet)
            invoke(quick, HUB)
            hub = wait(lambda: panel(pet, HUB), "quick menu opens chat integration")
            wait(lambda: not U.IsWindow(quick), "opening chat integration closes the quick toolbar")
            message(hub, 0x10)
            wait(lambda: not U.IsWindow(hub), "chat integration closed")
            overview = open_overview(pet)
            activate(overview)
            ui.SendKeys("{Esc}")
            wait(lambda: not U.IsWindow(overview), "Esc closes the overview")
            menu(pet, "c", tray=True)
            hub = wait(lambda: panel(pet, HUB), "reopen details")
            invoke(hub, "连接设置")
            settings = wait(lambda: panel(pet, SETTINGS), "reopen connection settings")
            toggle = connection_toggle(settings)
            require(toggle.ToggleState == 1, "connection is checked by default")
            toggle.Toggle()
            require(persisted() is None, "editing the checkbox does not persist the draft")
            invoke(settings, "应用并保存")
            wait(lambda: persisted() is False, "disable persists")
            message(settings, 0x10)
            wait(lambda: not U.IsWindow(settings), "settings closed")
            overview = open_overview(pet)
            if include_fixture:
                wait(lambda: text(overview, name="订阅：fixture"), "other cache survives disable")
                require(not text(overview, regex=r"^(?:已连接至)?ChatGPT$"),
                        "disabled client removed from overview")
            else:
                wait(lambda: text(overview, name="尚未启用客户端连接"), "disabled state restored")
            message(pet, 0x10)
            require(proc.wait(timeout=8) == 0, "pet exits cleanly")
            wait(lambda: not U.IsWindow(overview), "owned overview is reaped on exit")
            return dict(status="needs_auth" if needs_auth else "available", refresh_ms=refresh_ms,
                        quota_windows=len(quota),
                        clear_cache=True, shared_view=True, tray_reuse=True, hub_position=True,
                        escape=True, enabled_by_default=True, auto_refresh=True,
                        synthetic_source=include_fixture,
                        desktop_version=initial_version, lifecycle=lifecycle_result)
        finally:
            if proc.poll() is None:
                for child in parent.children(recursive=True):
                    try:
                        child.kill()
                    except psutil.NoSuchProcess:
                        pass
                proc.kill()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("exe", "pack", "output"):
        parser.add_argument(f"--{option}", required=True, type=Path)
    parser.add_argument("--needs-auth", action="store_true")
    parser.add_argument("--fixture", action="store_true",
                        help="also validate a synthetic source (requires window-diagnostics)")
    parser.add_argument("--lifecycle", nargs="?", const="automatic", choices=["automatic"],
                        help="interactively quit and restart the real client in a dedicated test session")
    args = parser.parse_args()
    if args.lifecycle and (args.needs_auth or args.fixture or not sys.stdin.isatty()):
        parser.error("--lifecycle requires an interactive terminal and a real signed-in test client")
    output = args.output.resolve()
    result = run(args.exe.resolve(strict=True), args.pack.resolve(strict=True), output,
                 args.needs_auth, args.fixture, args.lifecycle)
    report = json.dumps(result, indent=2)
    (output / "result.json").write_text(report + "\n", encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
