import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

HOOKS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HOOKS))

import scry_watch


def brief_result(stdout, code=0):
    return SimpleNamespace(returncode=code, stdout=stdout, stderr="")


class BriefTests(unittest.TestCase):
    def test_brief_is_appended_under_header(self):
        with mock.patch(
            "subprocess.run", return_value=brief_result("[lesson #12] (0.81) x\n")
        ):
            out = scry_watch.memory_brief("/repo")
        self.assertTrue(out.startswith("\n\n" + scry_watch.BRIEF_HEADER))
        self.assertTrue(out.endswith("[lesson #12] (0.81) x"))

    def test_empty_failed_or_timed_out_brief_adds_nothing(self):
        with mock.patch("subprocess.run", return_value=brief_result("")):
            self.assertEqual(scry_watch.memory_brief("/repo"), "")
        with mock.patch("subprocess.run", return_value=brief_result("boom", 1)):
            self.assertEqual(scry_watch.memory_brief("/repo"), "")
        timeout = subprocess.TimeoutExpired("scry", 5)
        with mock.patch("subprocess.run", side_effect=timeout):
            self.assertEqual(scry_watch.memory_brief("/repo"), "")

    def test_brief_runs_in_session_cwd_with_timeout(self):
        with mock.patch("subprocess.run", return_value=brief_result("")) as run:
            scry_watch.memory_brief("/repo")
        self.assertEqual(run.call_args.kwargs["cwd"], "/repo")
        self.assertEqual(run.call_args.kwargs["timeout"], 5)


class GuardTests(unittest.TestCase):
    def test_every_hook_exits_silently_under_capture_env(self):
        payload = json.dumps(
            {
                "prompt": "a long enough prompt to recall",
                "session_id": "guard",
                "cwd": "/",
            }
        )
        for name in ("scry_recall.py", "scry_watch.py", "scry_watch_kill.py"):
            result = subprocess.run(
                [sys.executable, str(HOOKS / name)],
                input=payload,
                text=True,
                capture_output=True,
                env={**os.environ, "SCRY_CAPTURE": "1"},
                timeout=10,
            )
            self.assertEqual((result.returncode, result.stdout), (0, ""), name)


class HookConfigTests(unittest.TestCase):
    def setUp(self):
        self.hooks = json.loads((HOOKS / "hook.json").read_text())["hooks"]

    def commands(self, event):
        return [h["command"] for group in self.hooks[event] for h in group["hooks"]]

    def test_capture_runs_at_precompact_and_session_end(self):
        for event in ("PreCompact", "SessionEnd"):
            self.assertTrue(
                any("scry_capture.py" in c for c in self.commands(event)), event
            )
        self.assertTrue(
            any("scry_watch_kill.py" in c for c in self.commands("SessionEnd"))
        )

    def test_session_start_matches_all_sources(self):
        self.assertEqual(
            self.hooks["SessionStart"][0]["matcher"], "startup|resume|clear|compact"
        )


if __name__ == "__main__":
    unittest.main()
