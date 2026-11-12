"""Validate the release migration CLI with isolated profiles and a legacy writer fixture."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tomllib
import uuid
import winreg


LEGACY = "852f4f4"
PRIVATE_TEXT = "合成会话正文与标记 fixture-confidential"


def legacy_history(output):
    modules = output / "legacy-modules"
    modules.mkdir()
    for name in ("chat_backend", "platform_support", "quota", "version"):
        source = subprocess.check_output(["git", "show", f"{LEGACY}:app/{name}.py"])
        (modules / f"{name}.py").write_bytes(source)
    folder = output / "legacy-history"
    subprocess.run(
        [sys.executable, "-c", """
from chat_backend import HistoryStore
import sys

store = HistoryStore(sys.argv[1])
first = store.new(sys.argv[1], "work")
first["threadId"] = "fixture-thread"
first["turns"].append(dict(status="running", prompt=sys.argv[2], output="已有输出"))
second = store.new(sys.argv[1], "chat")
second["turns"].append(dict(status="completed", prompt="合成输入", result="合成结果"))
store.save()
""", str(folder), PRIVATE_TEXT],
        cwd=modules, check=True, timeout=15,
    )
    return folder / "conversations.json"


def migration(exe, env, *args, fail=False):
    result = subprocess.run([exe, "migrate", *args], env=env, capture_output=True, text=True,
                            encoding="utf-8", timeout=15)
    assert (result.returncode != 0) == fail, (args[0], result.returncode, result.stderr)
    assert PRIVATE_TEXT not in result.stdout + result.stderr
    return result.stderr if fail else result.stdout.strip()


def backup(profile, receipt_id, expected):
    assert re.fullmatch(r"[0-9a-f]{64}", receipt_id)
    directory = profile.config.parent / "backups" / "migrations" / receipt_id
    assert (directory / "source.json").read_bytes() == expected
    return directory


def check_qsettings(runtime):
    sys.path.insert(0, str(runtime.resolve()))
    from PySide6.QtCore import QSettings, qVersion

    assert qVersion() == "6.9.2", "use the Qt version pinned by the legacy project"
    organization = f"DesktopPetChatGPT.Validation-{uuid.uuid4().hex}"
    key_name = rf"Software\{organization}\DesktopPet"
    settings = QSettings(QSettings.Format.NativeFormat, QSettings.Scope.UserScope,
                         organization, "DesktopPet")
    try:
        settings.setValue("sizePercent", 130)
        settings.setValue("soundDisabled", True)
        settings.setValue("autoHideFullscreen", False)
        settings.sync()
        assert settings.status() == QSettings.Status.NoError
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key_name) as key:
            actual = {name: winreg.QueryValueEx(key, name) for name in
                      ("sizePercent", "soundDisabled", "autoHideFullscreen")}
        assert actual["sizePercent"] == (130, winreg.REG_DWORD)
        assert actual["soundDisabled"] == ("true", winreg.REG_SZ)
        assert actual["autoHideFullscreen"] == ("false", winreg.REG_SZ)
        return {"qt": qVersion(), "registry_values": actual}
    finally:
        del settings
        for key_name in (key_name, rf"Software\{organization}"):
            try:
                winreg.DeleteKey(winreg.HKEY_CURRENT_USER, key_name)
            except FileNotFoundError:
                pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--qt-runtime", type=Path, help="optional isolated PySide6 Essentials 6.9.2")
    args = parser.parse_args()
    output, exe, pack = (path.resolve() for path in (args.output, args.exe, args.pack))
    output.mkdir(parents=True, exist_ok=False)
    native = runpy.run_path(str(Path(__file__).with_name("check-settings.py")))
    profile = native["Profile"](output / "profile")
    env = dict(os.environ, LOCALAPPDATA=str(profile.root))
    original_config = profile.config.with_name("config.json")
    config_bytes = json.dumps({"schema_version": 1, "pack_path": pack.as_posix(),
                               "saved_size": {"base_dip": 240, "percent": 110},
                               "preferences": {"auto_move": False, "sound": False}}).encode()
    original_config.write_bytes(config_bytes)
    config_import = migration(exe, env, "config-json")
    config_id = config_import.removeprefix("Imported ")
    config_backup = backup(profile, config_id, config_bytes)
    config = profile.config.read_bytes()
    values = tomllib.loads(config.decode("utf-8"))
    assert values["saved_size"]["percent"] == 110 and not values["preferences"]["sound"]
    assert migration(exe, env, "config-json") == f"Already imported {config_id}"
    assert profile.config.read_bytes() == config
    with native["pet_process"](exe, profile, output / "product-startup.log") as pet:
        assert "initial DIP 264.00" in pet.text()
        assert "请先退出" in migration(exe, env, "config-json", fail=True)

    original_history = legacy_history(output)
    history_bytes = original_history.read_bytes()
    env["BAILONG_DATA_DIR"] = str(original_history.parent)
    corrupt = history_bytes[:-1]
    original_history.write_bytes(corrupt)
    error = migration(exe, env, "bailong-history", fail=True)
    corrupt_id = re.search(r"原件备份 ID ([0-9a-f]{64})", error)[1]
    backup(profile, corrupt_id, corrupt)
    provider = profile.config.parent / "history" / "legacy-bailong-codex"
    assert not provider.exists() and original_history.read_bytes() == corrupt
    original_history.write_bytes(history_bytes)
    history_id = migration(exe, env, "bailong-history").removeprefix("Imported ")
    history_backup = backup(profile, history_id, history_bytes)
    receipt = json.loads((history_backup / "receipt.json").read_bytes())
    assert receipt["source_sha256"] == hashlib.sha256(history_bytes).hexdigest()
    assert migration(exe, env, "bailong-history") == f"Already imported {history_id}"
    snapshots = [json.loads(path.read_bytes()) for path in provider.glob("*.json")]
    turns = {item["legacy"]["mode"]: item["legacy"]["turns"][0] for item in snapshots}
    assert len(snapshots) == 2
    assert turns["work"]["status"] == "interrupted" and turns["work"]["legacy_status"] == "running"
    assert turns["work"]["prompt"] == PRIVATE_TEXT and turns["work"]["output"] == "已有输出"
    assert turns["chat"]["status"] == "completed" and turns["chat"]["result"] == "合成结果"
    for item in snapshots:
        assert item["session_id"] == hashlib.sha256(item["legacy"]["id"].encode()).hexdigest()

    profile.config.write_bytes(config + b"\n# external edit\n")
    migration(exe, env, "rollback", config_id, fail=True)
    assert profile.config.read_bytes().endswith(b"# external edit\n")
    profile.config.write_bytes(config)
    assert migration(exe, env, "rollback", config_id) == f"Rolled back {config_id}"
    assert (config_backup / "rolled-back").read_bytes() == config
    assert migration(exe, env, "rollback", config_id) == f"Already rolled back {config_id}"
    assert migration(exe, env, "rollback", history_id) == f"Rolled back {history_id}"
    assert (history_backup / "rolled-back").is_dir() and not provider.exists()
    assert not profile.config.exists() and original_config.read_bytes() == config_bytes
    assert original_history.read_bytes() == history_bytes

    with exe.open("rb") as stream:
        exe_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    report = {
        "exe_sha256": exe_hash,
        "legacy_fixture_source": LEGACY,
        "config_migration": config_id,
        "history_migration": history_id,
        "corrupt_backup": corrupt_id,
        "checks": ["explicit import", "production reload", "running-instance lock", "idempotent retry",
                   "corrupt history recovery", "interrupted turn", "edit-safe rollback", "source integrity"],
    }
    if args.qt_runtime:
        report["qsettings_fixture"] = check_qsettings(args.qt_runtime)
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
