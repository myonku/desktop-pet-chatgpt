"""Offline event-correlation checks for the M6 App Server probe."""

from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
import unittest


spec = spec_from_file_location("probe", Path(__file__).with_name("check-codex-conversation.py"))
probe = module_from_spec(spec)
spec.loader.exec_module(probe)


def event(method, **params):
    return dict(method=method, params=dict(threadId="thread", turnId="turn", **params))


def delta(text, item="reply"):
    return event("item/agentMessage/delta", itemId=item, delta=text)


def completed(item="reply", text=probe.MARKER):
    return event("item/completed", item=dict(id=item, type="agentMessage", text=text))


def turn_end(status="completed"):
    return event("turn/completed", turn=dict(id="turn", status=status))


class Server:
    def __init__(self, frames):
        self.frames = iter(frames)

    def request(self, method, params, on_event=None):
        if method == "account/read":
            return dict(account=dict(type="chatgpt"))
        if method == "thread/start":
            return dict(thread=dict(id="thread", ephemeral=True))
        if method == "turn/start":
            # Exercise notifications arriving before the turn acknowledgement.
            on_event(next(self.frames))
            return dict(turn=dict(id="turn"))
        raise AssertionError(method)

    def frame(self, _deadline):
        try:
            return next(self.frames)
        except StopIteration as error:
            raise probe.ProbeError("response_timeout") from error


class StreamingChecks(unittest.TestCase):
    def run_probe(self, frames):
        return probe.check_ephemeral_turn(Server(frames), Path.cwd())

    def test_matching_fragments_and_completion(self):
        result = self.run_probe([
            delta("DESKTOP_PET_"), delta("M6_OK"), completed(), turn_end(),
        ])
        self.assertEqual(result["streamed_deltas"], 2)
        self.assertTrue(result["reply_matches"])

    def test_wrong_delta_with_correct_final_text_is_rejected(self):
        with self.assertRaisesRegex(probe.ProbeError, "streamed_text_mismatch"):
            self.run_probe([delta("incorrect"), completed(), turn_end()])

    def test_deltas_from_another_item_cannot_verify_reply(self):
        with self.assertRaisesRegex(probe.ProbeError, "streamed_text_mismatch"):
            self.run_probe([delta(probe.MARKER, "other"), completed(), turn_end()])

    def test_foreign_thread_and_turn_events_are_ignored(self):
        foreign_thread = delta("incorrect")
        foreign_thread["params"]["threadId"] = "other-thread"
        foreign_turn = delta("incorrect")
        foreign_turn["params"]["turnId"] = "other-turn"
        result = self.run_probe([
            foreign_thread, foreign_turn, delta(probe.MARKER), completed(), turn_end(),
        ])
        self.assertEqual(result["streamed_deltas"], 1)

    def test_nontext_delta_is_rejected(self):
        with self.assertRaisesRegex(probe.ProbeError, "invalid_text_delta"):
            self.run_probe([delta(7), completed(), turn_end()])

    def test_oversized_stream_is_rejected(self):
        with self.assertRaisesRegex(probe.ProbeError, "streamed_text_too_large"):
            self.run_probe([delta("x" * (probe.MAX_LINE + 1))])

    def test_failed_turn_cannot_pass_with_correct_text(self):
        with self.assertRaisesRegex(probe.ProbeError, "streamed_reply_not_verified"):
            self.run_probe([delta(probe.MARKER), completed(), turn_end("failed")])

    def test_turn_without_completed_reply_is_rejected(self):
        with self.assertRaisesRegex(probe.ProbeError, "streamed_reply_not_verified"):
            self.run_probe([delta(probe.MARKER), turn_end()])


if __name__ == "__main__":
    unittest.main()
