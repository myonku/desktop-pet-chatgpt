"""Probe the public Codex App Server before enabling desktop conversation UI."""

import argparse
from collections import deque
import json
from pathlib import Path
from queue import Empty, Full, Queue
import re
import subprocess
from threading import Event, Thread
from time import monotonic


MAX_LINE = 1024 * 1024
MARKER = "DESKTOP_PET_M6_OK"
THREAD_STATUSES = {"notLoaded", "idle", "active", "systemError"}


class ProbeError(Exception):
    pass


class AppServer:
    def __init__(self, cli, cwd):
        self.command = [cli, "app-server", "--listen", "stdio://"]
        self.cwd = cwd
        self.frames = Queue(maxsize=64)
        self.overflow = Event()
        self.next_id = 0

    def __enter__(self):
        self.process = subprocess.Popen(
            self.command, cwd=self.cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        Thread(target=self.read_frames, daemon=True).start()
        return self

    def __exit__(self, *_):
        try:
            self.process.stdin.close()
            self.process.wait(timeout=2)
        except (OSError, subprocess.TimeoutExpired):
            self.process.kill()
            self.process.wait()

    def read_frames(self):
        try:
            with self.process.stdout as output:
                while line := output.readline(MAX_LINE + 1):
                    self.frames.put_nowait(line)
                    if len(line) > MAX_LINE or not line.endswith(b"\n"):
                        return
            self.frames.put_nowait(b"")
        except Full:
            self.overflow.set()
        except OSError:
            return

    def frame(self, deadline):
        if self.overflow.is_set():
            raise ProbeError("event_queue_overflow")
        remaining = deadline - monotonic()
        if remaining <= 0:
            raise ProbeError("response_timeout")
        try:
            line = self.frames.get(timeout=remaining)
        except Empty as error:
            raise ProbeError("response_timeout") from error
        if not line:
            raise ProbeError("server_disconnected")
        if len(line) > MAX_LINE or not line.endswith(b"\n"):
            raise ProbeError("invalid_frame")
        frame = json.loads(line)
        if not isinstance(frame, dict):
            raise ProbeError("invalid_frame")
        return frame

    def send(self, frame):
        self.process.stdin.write(json.dumps(frame).encode("utf-8") + b"\n")
        self.process.stdin.flush()

    def request(self, method, params, on_event=lambda _: None):
        self.next_id += 1
        self.send(dict(id=self.next_id, method=method, params=params))
        deadline = monotonic() + 12
        while True:
            frame = self.frame(deadline)
            if "id" not in frame and "method" in frame:
                on_event(frame)
                continue
            if frame.get("id") != self.next_id or "method" in frame:
                raise ProbeError("unexpected_server_message")
            if "error" in frame:
                code = frame["error"].get("code")
                raise ProbeError(f"{method}_rejected_{code if type(code) is int else 'unknown'}")
            return frame["result"]

    def initialize(self):
        self.request("initialize", dict(clientInfo=dict(
            name="desktop_pet_chatgpt_probe", title="Desktop Pet Conversation Probe",
            version="0.1.0",
        )))
        self.send(dict(method="initialized"))


def check_selected_thread(server, thread_id):
    thread = server.request("thread/read", dict(threadId=thread_id, includeTurns=False))["thread"]
    status = thread["status"]["type"]
    if thread["id"] != thread_id or status not in THREAD_STATUSES:
        raise ProbeError("unexpected_thread_identity_or_status")
    return dict(thread_id_matches=True, runtime_status=status)


def check_ephemeral_turn(server, cwd):
    account = server.request("account/read", dict(refreshToken=False))["account"]
    if not account or account["type"] != "chatgpt":
        raise ProbeError("chatgpt_account_required")
    thread = server.request("thread/start", dict(
        ephemeral=True, cwd=str(cwd), sandbox="read-only", approvalPolicy="never",
    ))["thread"]
    if thread.get("ephemeral") is not True or not thread.get("id"):
        raise ProbeError("ephemeral_thread_not_confirmed")

    pending = deque()

    def buffer(frame):
        if len(pending) >= 64:
            raise ProbeError("too_many_events_before_turn_ack")
        pending.append(frame)

    turn = server.request("turn/start", dict(threadId=thread["id"], input=[
        dict(type="text", text=f"Reply with exactly {MARKER}. Do not use any tools."),
    ]), on_event=buffer)["turn"]
    if not turn.get("id"):
        raise ProbeError("turn_identity_missing")

    deltas = {}
    streamed_reply = 0
    deadline = monotonic() + 40
    while True:
        frame = pending.popleft() if pending else server.frame(deadline)
        if "id" in frame:
            raise ProbeError("unexpected_server_message")
        params = frame.get("params", {})
        if params.get("threadId") != thread["id"]:
            continue
        method = frame.get("method")
        if method == "turn/completed" and params["turn"]["id"] == turn["id"]:
            if params["turn"]["status"] != "completed" or not streamed_reply:
                raise ProbeError("streamed_reply_not_verified")
            return dict(ephemeral=True, streamed_deltas=streamed_reply, reply_matches=True,
                        turn_completed=True)
        if params.get("turnId") != turn["id"]:
            continue
        if method == "item/agentMessage/delta":
            delta = params.get("delta")
            item_id = params.get("itemId")
            if not isinstance(delta, str) or not isinstance(item_id, str) or not item_id:
                raise ProbeError("invalid_text_delta")
            if not delta:
                continue
            text, count = deltas.get(item_id, ("", 0))
            if len(text) + len(delta) > MAX_LINE:
                raise ProbeError("streamed_text_too_large")
            deltas[item_id] = (text + delta, count + 1)
            if len(deltas) > 64:
                raise ProbeError("too_many_streaming_items")
        elif method == "item/completed" and params["item"]["type"] == "agentMessage":
            item = params["item"]
            text, count = deltas.pop(item["id"], ("", 0))
            if item["text"].strip() == MARKER:
                if text != item["text"]:
                    raise ProbeError("streamed_text_mismatch")
                streamed_reply = max(streamed_reply, count)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path, help="trusted Desktop companion codex.exe")
    parser.add_argument("--output", required=True, type=Path, help="new evidence directory")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--thread-id", help="ID of an authorized desktop test chat, metadata only")
    mode.add_argument("--ephemeral-turn", action="store_true", help="send one in-memory test turn")
    args = parser.parse_args()
    if args.thread_id is not None and not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", args.thread_id):
        parser.error("invalid thread ID")
    try:
        cli = args.cli.resolve(strict=True)
        args.output.mkdir(parents=True)
        cwd = args.output.resolve() / "workspace"
        cwd.mkdir()
    except OSError:
        parser.error("provide an existing CLI and a new evidence directory")
    try:
        with AppServer(cli, cwd) as server:
            server.initialize()
            if args.ephemeral_turn:
                result = check_ephemeral_turn(server, cwd)
            else:
                result = check_selected_thread(server, args.thread_id)
        report = dict(status="ok", **result)
    except ProbeError as error:
        report = dict(status="error", reason=str(error))
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        report = dict(status="error", reason="io_or_protocol_error")
    text = json.dumps(report, indent=2)
    (args.output / "result.json").write_text(text + "\n", encoding="utf-8")
    print(text)
    return int(report["status"] != "ok")


if __name__ == "__main__":
    raise SystemExit(main())
