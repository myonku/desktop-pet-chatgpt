"""Stage and exercise the default Windows build with an isolated profile and local assets."""

import argparse
from contextlib import contextmanager
import csv
import hashlib
from itertools import batched
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import time
import tomllib
from zipfile import ZIP_DEFLATED, ZipFile

from windows_desktop import U, bounds, drag, find_window, fullscreen, get_monitor, hit, menu, message, panel, quick, require, wait


HERE = Path(__file__).resolve().parent
CORE_BINDINGS = {
    "idle": "idle", "interact": "interact", "drag": "drag", "land": "land",
    "move-left": "move.left", "move-right": "move.right",
    "edge-left": "dock.left", "edge-right": "dock.right",
    "peek-left": "peek.left", "peek-right": "peek.right",
}
POWERSHELL = shutil.which("pwsh") or shutil.which("powershell")
SYSTEM = Path(os.environ["SystemRoot"]) / "System32"


class Profile:
    def __init__(self, root):
        self.root = root
        self.config = root / "DesktopPetChatGPT/config.toml"
        self.config.parent.mkdir(parents=True)

    def disable_auto_move(self):
        self.config.write_text("[preferences]\nauto_move = false\n", encoding="utf-8")

    def read(self):
        return tomllib.loads(self.config.read_text(encoding="utf-8"))



def powershell(command):
    return subprocess.check_output([POWERSHELL, "-NoProfile", "-Command", command],
                                   timeout=30, text=True, encoding="utf-8").strip()


def network_state():
    return json.loads(powershell(
        "[Console]::OutputEncoding = [Text.UTF8Encoding]::new(); "
        "$connected = @(Get-NetIPInterface | Where-Object ConnectionState -eq Connected | "
        "Select-Object -ExpandProperty InterfaceIndex); "
        "@{PhysicalUp = @(Get-NetAdapter -Physical | Where-Object Status -eq Up | "
        "Select-Object -ExpandProperty Name); "
        "DefaultRoutes = @(Get-NetRoute -PolicyStore ActiveStore | "
        "Where-Object { $_.InterfaceIndex -in $connected -and "
        "$_.DestinationPrefix -in @('0.0.0.0/0','::/0') } | "
        "Select-Object InterfaceAlias,DestinationPrefix)} | ConvertTo-Json -Compress"
    ))


def sha(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def core_action_coverage(manifest):
    bindings = manifest["bindings"]
    present = {action for action, key in CORE_BINDINGS.items() if bindings.get(key) is not None}
    return sorted(present), sorted(CORE_BINDINGS.keys() - present)


def stage_checkpoint(args, output):
    pack = args.pack
    pack_zip = output / "default-pack.zip"
    subprocess.run([sys.executable, HERE.parents[1] / "release/assemble-asset-pack.py",
                    "--validator", args.validator, "--pack", pack, "--output", pack_zip],
                   check=True, timeout=60)
    install = output / "install"
    default = install / "resources/default"
    default.mkdir(parents=True)
    with ZipFile(pack_zip) as archive:
        archive.extractall(default)
    exe = install / "desktop-pet-chatgpt.exe"
    shutil.copy2(args.exe, exe)
    (install / "README.txt").write_text(
        "Windows x64 validation attachment. Extract the entire ZIP and run "
        "desktop-pet-chatgpt.exe. "
        "Install Windows App Runtime 2.4 or newer 2.x (x64) before opening the panels. "
        "Keep the executable and resources directory together.\n", encoding="utf-8",
    )
    payload = sorted(path for path in install.rglob("*") if path.is_file())
    archive_path = output / "windows-validation.zip"
    with ZipFile(archive_path, "x", ZIP_DEFLATED) as archive:
        for path in payload:
            archive.write(path, path.relative_to(install).as_posix())
    hashes = {path.relative_to(install).as_posix(): sha(path) for path in payload}
    with ZipFile(archive_path) as archive:
        require(archive.testzip() is None and set(archive.namelist()) == hashes.keys(),
                "verify checkpoint ZIP members")
        for name, expected in hashes.items():
            require(hashlib.sha256(archive.read(name)).hexdigest() == expected, f"checkpoint member {name}")
    return exe, hashes, {"sha256": sha(archive_path), "bytes": archive_path.stat().st_size,
                         "exe_sha256": sha(exe), "manifest_sha256": sha(default / "manifest.json")}


@contextmanager
def read_only_install(install):
    sid = powershell("[Security.Principal.WindowsIdentity]::GetCurrent().User.Value")
    target = "*" + sid
    subprocess.run([SYSTEM / "icacls.exe", install, "/deny", target + ":(OI)(CI)(WD,AD,WEA,WA,DE,DC)"],
                   check=True, capture_output=True, timeout=10)
    try:
        try:
            with (install / "write-probe").open("xb"):
                pass
        except PermissionError:
            pass
        else:
            raise AssertionError("test install unexpectedly permits creating files")
        yield
    finally:
        subprocess.run([SYSTEM / "icacls.exe", install, "/remove:d", target],
                       check=True, capture_output=True, timeout=10)


def environment(profile):
    env = {name: os.environ[name] for name in ("SystemRoot", "WINDIR") if name in os.environ}
    for folder in ("appdata", "temp", "home"):
        (profile.root / folder).mkdir(exist_ok=True)
    env.update(LOCALAPPDATA=str(profile.root), APPDATA=str(profile.root / "appdata"),
               USERPROFILE=str(profile.root / "home"), TEMP=str(profile.root / "temp"),
               TMP=str(profile.root / "temp"), PATH=str(SYSTEM))
    return env


@contextmanager
def product(exe, env, log):
    hwnd = None
    with log.open("wb") as stream:
        started = time.perf_counter()
        proc = subprocess.Popen([exe], cwd=exe.parent, env=env,
                                stdout=stream, stderr=subprocess.STDOUT)
        try:
            hwnd = wait(lambda: find_window(proc.pid), "product window", timeout=10)
            wait(lambda: "First asset frame presented" in log.read_text(encoding="utf-8", errors="replace"),
                 "first local asset frame", timeout=10)
            wait(lambda: U.IsWindowVisible(hwnd), "visible product window")
            yield proc, hwnd, round((time.perf_counter() - started) * 1000, 2)
        finally:
            if proc.poll() is None and hwnd and U.IsWindow(hwnd):
                message(hwnd, 0x10)
            try:
                proc.wait(timeout=8)
            except subprocess.TimeoutExpired:
                proc.terminate()
                proc.wait(timeout=5)
    require(proc.returncode == 0, f"product exit {proc.returncode}; see {log.name}")


def check_behavior(exe, profile, env, output):
    profile.disable_auto_move()
    with product(exe, env, output / "behavior.log") as (_, hwnd, _):
        monitor = get_monitor(U.MonitorFromWindow(hwnd, 2))
        require(hit(hwnd, .5, .5) and not hit(hwnd, .02, .02), "opaque/transparent hit region")
        _, top, _, _ = bounds(hwnd)
        drag(hwnd, monitor.work.left + 3, top)
        wait(lambda: bounds(hwnd)[0] == monitor.work.left, "snap to left work area")
        quick(hwnd, "展开桌宠")
        wait(lambda: bounds(hwnd)[0] > monitor.work.left, "expand from edge")
        drag_ms = []
        for step in range(10):
            left, top, _, _ = bounds(hwnd)
            started = time.perf_counter()
            drag(hwnd, left + (60 if step % 2 == 0 else -60), top)
            drag_ms.append(round((time.perf_counter() - started) * 1000, 2))
        menu(hwnd, "H", tray=True)
        wait(lambda: not U.IsWindowVisible(hwnd), "tray hide")
        menu(hwnd, "H", tray=True)
        wait(lambda: U.IsWindowVisible(hwnd), "tray restore")
        with fullscreen(monitor):
            wait(lambda: not U.IsWindowVisible(hwnd), "automatic fullscreen hide")
        wait(lambda: U.IsWindowVisible(hwnd), "fullscreen exit restores pet")
        menu(hwnd, "U", tray=True)
        wait(lambda: profile.read()["preferences"].get("sound") is False, "menu preference saved")
        menu(hwnd, "M", tray=True)
        wait(lambda: profile.read()["preferences"].get("auto_move") is True, "auto move enabled")
        position = bounds(hwnd)[:2]
        manifest = json.loads((exe.parent / "resources/default/manifest.json").read_bytes())
        move_ids = {clip for side in ("left", "right")
                    for clip in manifest["bindings"].get(f"move.{side}", {}).get("variants", {}).values()}
        require(move_ids, "offline behavior test needs an autonomous movement clip")
        move_delay = max(clip["movement_us"][0] for clip in manifest["clips"] if clip["id"] in move_ids)
        wait(lambda: bounds(hwnd)[:2] != position, "autonomous movement in product build",
             timeout=18 + move_delay / 1_000_000)
    with product(exe, env, output / "restart.log") as (_, _, _):
        require(profile.read()["preferences"]["sound"] is False, "preference preserved on restart")
    return drag_ms


def process_connections(pid):
    return json.loads(powershell(
        f"@{{Children = @(Get-CimInstance Win32_Process -Filter 'ParentProcessId = {pid}' | "
        "Select-Object -ExpandProperty Name); "
        f"TCP = @(Get-NetTCPConnection -OwningProcess {pid} -ErrorAction SilentlyContinue).Count; "
        f"UDP = @(Get-NetUDPEndpoint -OwningProcess {pid} -ErrorAction SilentlyContinue).Count}} "
        "| ConvertTo-Json -Compress"
    ))


def sample(pid, output, duration):
    def quote(path):
        return "'" + str(path).replace("'", "''") + "'"

    ids = ",".join(map(str, pid if isinstance(pid, list) else [pid]))
    command = (f"& {quote(HERE.parent / 'measure-process.ps1')} -ProcessIds @({ids}) "
               f"-DurationSeconds {duration} -OutputCsv {quote(output)}")
    subprocess.run([POWERSHELL, "-NoProfile", "-Command", command],
                   timeout=duration + 15, check=True)
    with output.open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.DictReader(stream))
    width = len(pid) if isinstance(pid, list) else 1
    totals = []
    for batch in batched(rows, width, strict=True):
        totals.append({key: sum(float(row[key]) for row in batch)
                       for key in ("CpuSeconds", "PrivateBytes", "WorkingSetBytes", "PeakWorkingSetBytes")}
                      | {"ElapsedSeconds": float(batch[0]["ElapsedSeconds"])})
    first, last = totals[0], totals[-1]
    elapsed = last["ElapsedSeconds"] - first["ElapsedSeconds"]
    return {
        "seconds": round(elapsed, 2),
        "cpu_percent_one_core": round(100 * (float(last["CpuSeconds"]) - float(first["CpuSeconds"]))
                                      / elapsed, 2),
        "private_bytes_max": max(int(row["PrivateBytes"]) for row in totals),
        "working_set_bytes_max": max(int(row["WorkingSetBytes"]) for row in totals),
        "peak_working_set_bytes": max(int(row["PeakWorkingSetBytes"]) for row in totals),
    }


def measure_installed_pack(args, exe, profile, env, output):
    profile.disable_auto_move()
    with product(exe, env, output / "representative.log") as (proc, hwnd, _):
        time.sleep(2)
        idle = sample(proc.pid, output / "idle.csv", args.seconds)
        menu(hwnd, "S", tray=True)
        settings = wait(lambda: panel(hwnd), "product settings window")
        time.sleep(2)
        import psutil
        child, = psutil.Process(proc.pid).children()
        opened = sample([proc.pid, child.pid], output / "settings.csv", args.seconds)
        observations = {name: process_connections(pid) for name, pid in
                        (("pet", proc.pid), ("settings", child.pid))}
        require(len(observations["pet"]["Children"]) == 1 and not observations["settings"]["Children"]
                and all(not value["TCP"] and not value["UDP"] for value in observations.values()),
                "only the expected local settings process and no app sockets")
        message(settings, 0x10)
        wait(lambda: not U.IsWindow(settings), "close product settings")
        menu(hwnd, "H", tray=True)
        wait(lambda: not U.IsWindowVisible(hwnd), "hide representative pet")
        hidden = sample(proc.pid, output / "hidden.csv", args.seconds)
    return {"idle": idle, "settings": opened, "hidden": hidden, "process_observation": observations}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("exe", "validator", "pack", "output"):
        parser.add_argument(f"--{flag}", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=15)
    parser.add_argument("--require-disconnected", action="store_true",
                        help="require zero active physical adapters and default routes")
    args = parser.parse_args()
    for flag in ("exe", "validator", "pack", "output"):
        if path := getattr(args, flag):
            setattr(args, flag, path.resolve())
    if not 1 <= args.seconds <= 3600:
        parser.error("--seconds must be 1–3600")
    output = args.output
    output.mkdir(parents=True, exist_ok=False)
    network = network_state()
    (output / "network-before.json").write_text(json.dumps(network, indent=2), encoding="utf-8")
    if args.require_disconnected:
        require(not network["PhysicalUp"] and not network["DefaultRoutes"], "disconnect adapters before the test")
    exe, hashes, checkpoint = stage_checkpoint(args, output)
    profile = Profile(output / "profile")
    env = environment(profile)
    with read_only_install(exe.parent):
        startup = []
        for round_number in range(1, 11):
            with product(exe, env, output / f"startup-{round_number:02}.log") as (_, _, elapsed):
                startup.append(elapsed)
        drag_ms = check_behavior(exe, profile, env, output)
        metrics = measure_installed_pack(args, exe, profile, env, output)
        for relative, expected in hashes.items():
            require(sha(exe.parent / relative) == expected, f"installation file unchanged: {relative}")
    after = network_state()
    if args.require_disconnected:
        require(not after["PhysicalUp"] and not after["DefaultRoutes"], "network state changed during test")
    sample_pack = exe.parent / "resources/default"
    manifest = json.loads((sample_pack / "manifest.json").read_bytes())
    actions_present, actions_missing = core_action_coverage(manifest)
    report = {
        "checkpoint": checkpoint,
        "network_before": network,
        "network_after": after,
        "disconnected_required": args.require_disconnected,
        "startup_ms": {"samples": startup, "median": statistics.median(startup), "range": [min(startup), max(startup)]},
        "drag_feedback_ms": drag_ms,
        "representative": {"id": manifest["pack_id"], "manifest_sha256": sha(sample_pack / "manifest.json"),
                           "actions_present": actions_present,
                           "actions_missing": actions_missing, "metrics": metrics},
        "checks": ["installed default pack", "transparent hit region", "drag", "edge snap and expand",
                   "autonomous movement", "tray and fullscreen visibility", "settings and preference restart",
                   "read-only install", "no observed client process or socket"],
    }
    (output / "result.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
