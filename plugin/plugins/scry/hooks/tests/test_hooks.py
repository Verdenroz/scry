import json
import os
import subprocess
import sys
import tempfile
import time
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


    def test_work_lists_get_their_own_header(self):
        with mock.patch(
            "subprocess.run", return_value=brief_result("## ship (updated 1h ago)\n")
        ) as run:
            out = scry_watch.work_brief("/repo")
        self.assertEqual(run.call_args.args[0], ["scry", "work"])
        self.assertEqual(
            out, "\n\n" + scry_watch.WORK_HEADER + "## ship (updated 1h ago)"
        )


class GcTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.stamp = Path(tmp.name) / "cache" / "gc-stamp"
        for name, value in (
            ("GC_STAMP", self.stamp),
            ("DEBUG_LOG_FILE", Path(tmp.name) / "watch.log"),
        ):
            patcher = mock.patch.object(scry_watch, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def age_stamp(self, seconds):
        self.stamp.parent.mkdir(parents=True, exist_ok=True)
        self.stamp.touch()
        past = time.time() - seconds
        os.utime(self.stamp, (past, past))

    def test_due_when_stamp_is_missing_or_older_than_a_day(self):
        self.assertTrue(scry_watch.gc_due())
        self.age_stamp(25 * 3600)
        self.assertTrue(scry_watch.gc_due())

    def test_not_due_within_a_day(self):
        self.age_stamp(3600)
        self.assertFalse(scry_watch.gc_due())

    def test_spawn_is_detached_logged_and_stamps(self):
        with mock.patch("subprocess.Popen") as popen:
            scry_watch.spawn_gc("/repo")
        args, kwargs = popen.call_args
        self.assertEqual(args[0], ["scry", "memory", "gc"])
        self.assertTrue(kwargs["start_new_session"])
        self.assertEqual(kwargs["cwd"], "/repo")
        self.assertTrue(self.stamp.exists())
        self.assertFalse(scry_watch.gc_due())

    def test_spawn_failure_does_not_raise_or_stamp(self):
        with mock.patch("subprocess.Popen", side_effect=OSError("no scry")):
            scry_watch.spawn_gc("/repo")
        self.assertFalse(self.stamp.exists())


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
