import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

HOOKS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HOOKS))

import scry_capture

REAL_RUN = subprocess.run
QUOTE = "linker `cc` not found on this machine"


class Fake:
    def __init__(self, output, fail_anchored=False):
        self.output = output
        self.fail_anchored = fail_anchored
        self.calls = []

    def __call__(self, argv, **kwargs):
        if argv[0] == "git":
            return REAL_RUN(argv, **kwargs)
        self.calls.append((argv, kwargs))
        out, code = "", 0
        if argv[0] == "claude":
            out = json.dumps({"structured_output": self.output})
        elif argv[:2] == ["scry", "remember"]:
            code = 1 if self.fail_anchored and "--anchor" in argv else 0
            out = "remembered #50 (salience 0.5, surprise 0.2)"
        elif argv[:2] == ["scry", "recall"]:
            out = "[decision #9] (0.70) use rrf"
        return SimpleNamespace(returncode=code, stdout=out, stderr="")

    def scry(self, *prefix):
        return [a for a, _ in self.calls if a[: len(prefix)] == list(prefix)]


def transcript(root: Path, edited: str) -> str:
    lines = [
        {"type": "user", "message": {"content": "why does the build fail?"}},
        {
            "type": "assistant",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "a",
                        "name": "Bash",
                        "input": {"command": "cargo build"},
                    },
                    {
                        "type": "tool_use",
                        "id": "b",
                        "name": "Edit",
                        "input": {"file_path": edited},
                    },
                    {
                        "type": "tool_use",
                        "id": "c",
                        "name": "Read",
                        "input": {"file_path": edited},
                    },
                ]
            },
        },
        {
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "a",
                        "content": f"error: {QUOTE}",
                    }
                ]
            },
        },
    ]
    path = root / "t.jsonl"
    path.write_text("\n".join(json.dumps(line) for line in lines) + "\n")
    return str(path)


def lesson(**extra):
    return {
        "kind": "lesson",
        "content": "cc is needed",
        "pain": 4,
        "supersedes": None,
        "anchors": ["src/lib.rs"],
        "evidence": {"entry": 5, "quote": QUOTE},
        **extra,
    }


class CaptureTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name).resolve()
        REAL_RUN(["git", "init", "-q", str(self.root)])
        patcher = mock.patch.object(scry_capture, "STATE_DIR", self.root / "state")
        patcher.start()
        self.addCleanup(patcher.stop)
        self.payload = {
            "session_id": "s1",
            "cwd": str(self.root),
            "hook_event_name": "PreCompact",
            "transcript_path": transcript(self.root, str(self.root / "src/lib.rs")),
        }

    def run_capture(self, fake):
        with mock.patch("subprocess.run", fake):
            return scry_capture.capture(self.payload)

    def test_repo_relative_converts_and_drops_outside_paths(self):
        paths = [str(self.root / "src/lib.rs"), "/elsewhere/x.rs"]
        self.assertEqual(
            scry_capture.repo_relative(paths, str(self.root)), {"src/lib.rs"}
        )

    def test_repo_relative_outside_a_repo_is_empty(self):
        with tempfile.TemporaryDirectory() as bare:
            self.assertEqual(scry_capture.repo_relative(["/x/a.rs"], bare), set())

    def test_proven_lesson_is_written_anchored_after_index(self):
        fake = Fake({"memories": [lesson()], "reinforce": [], "contradict": []})
        result = self.run_capture(fake)
        self.assertEqual(result["written"], 1)
        self.assertEqual(len(fake.scry("scry", "index")), 1)
        (remember,) = fake.scry("scry", "remember")
        self.assertIn("lesson", remember)
        self.assertEqual(remember[remember.index("--anchor") + 1], "src/lib.rs")
        self.assertEqual(remember[remember.index("--evidence") + 1], f"[#5] {QUOTE}")

    def test_model_runs_neutral_with_guard_env(self):
        fake = Fake({"memories": [], "reinforce": [], "contradict": []})
        self.run_capture(fake)
        ((argv, kwargs),) = [c for c in fake.calls if c[0][0] == "claude"]
        self.assertEqual(kwargs["cwd"], str(self.root / "state"))
        self.assertEqual(kwargs["env"]["SCRY_CAPTURE"], "1")
        self.assertEqual(kwargs["env"]["CLAUDE_MEM_INTERNAL"], "1")
        self.assertIn("[decision #9] use rrf", kwargs["input"])

    def test_anchor_failure_retries_without_anchors(self):
        fake = Fake(
            {"memories": [lesson()], "reinforce": [], "contradict": []},
            fail_anchored=True,
        )
        self.assertEqual(self.run_capture(fake)["written"], 1)
        first, second = fake.scry("scry", "remember")
        self.assertIn("--anchor", first)
        self.assertNotIn("--anchor", second)

    def test_supersede_runs_only_when_kind_survives(self):
        bad = lesson(
            evidence={"entry": 1, "quote": "why does the build fail?"}, supersedes=9
        )
        fake = Fake({"memories": [bad], "reinforce": [], "contradict": []})
        self.run_capture(fake)
        self.assertEqual(fake.scry("scry", "memory", "supersede"), [])
        self.assertIn("episode", fake.scry("scry", "remember")[0])

    def test_second_pass_has_nothing_new_and_never_remarks(self):
        out = {
            "memories": [],
            "reinforce": [{"id": 9, "evidence": {"entry": 5, "quote": QUOTE}}],
            "contradict": [],
        }
        first = Fake(out)
        self.run_capture(first)
        self.assertEqual(
            first.scry("scry", "memory", "helpful"),
            [["scry", "memory", "helpful", "9"]],
        )
        second = Fake(out)
        self.assertEqual(self.run_capture(second)["skipped"], "few tool calls")
        self.assertEqual(second.calls, [])

    def test_later_pass_sees_only_entries_after_the_saved_offset(self):
        out = {"memories": [], "reinforce": [], "contradict": []}
        self.run_capture(Fake(out))
        more = [
            {
                "type": "assistant",
                "message": {
                    "content": [
                        {"type": "tool_use", "id": f"n{i}", "name": "Read", "input": {}}
                        for i in range(3)
                    ]
                },
            },
        ]
        with open(self.payload["transcript_path"], "a") as handle:
            handle.write(json.dumps(more[0]) + "\n")
        fake = Fake(out)
        self.run_capture(fake)
        ((_, kwargs),) = [c for c in fake.calls if c[0][0] == "claude"]
        self.assertNotIn("cargo build", kwargs["input"])
        self.assertIn("[#1] tool_use: Read", kwargs["input"])
        self.assertTrue((self.root / "state" / "s1.lock").exists())

    def test_recall_does_not_touch_access_stats(self):
        fake = Fake({"memories": [], "reinforce": [], "contradict": []})
        self.run_capture(fake)
        self.assertIn("--no-touch", fake.scry("scry", "recall")[0])

    def test_few_tool_calls_skip_without_advancing(self):
        Path(self.payload["transcript_path"]).write_text(
            json.dumps({"type": "user", "message": {"content": "hi there"}}) + "\n"
        )
        fake = Fake({})
        self.assertEqual(self.run_capture(fake)["skipped"], "few tool calls")
        self.assertFalse((self.root / "state" / "s1.json").exists())


class EntryTests(unittest.TestCase):
    def run_entry(self, env, stdin="{}"):
        return REAL_RUN(
            [sys.executable, str(HOOKS / "scry_capture.py")],
            input=stdin,
            text=True,
            capture_output=True,
            env={**os.environ, **env},
            timeout=10,
        )

    def test_guard_env_exits_immediately_without_spawning(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = self.run_entry({"SCRY_CAPTURE": "1", "SCRY_CAPTURE_DIR": tmp})
            self.assertEqual(result.returncode, 0)
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_hook_returns_zero_and_logs_worker_error_for_bad_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "log"
            env = {
                "SCRY_CAPTURE": "",
                "SCRY_CAPTURE_DIR": tmp,
                "SCRY_CAPTURE_LOG": str(log),
            }
            payload = json.dumps({"session_id": "s", "transcript_path": "/nonexistent"})
            self.assertEqual(self.run_entry(env, payload).returncode, 0)
            for _ in range(50):
                if log.exists():
                    break
                REAL_RUN(["sleep", "0.1"])
            self.assertIn("error", json.loads(log.read_text().splitlines()[0]))


if __name__ == "__main__":
    unittest.main()
