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

import capture_scry
import scry_capture

REAL_RUN = subprocess.run
QUOTE = "linker `cc` not found on this machine"


class Fake:
    def __init__(
        self,
        output,
        fail_anchored=False,
        verify=("not configured", 0),
        stale="",
        reaffirm_code=0,
        index_code=0,
        stale_code=0,
    ):
        self.output = output
        self.index_code = index_code
        self.stale_code = stale_code
        self.stale = stale
        self.reaffirm_code = reaffirm_code
        self.fail_anchored = fail_anchored
        self.verify = verify
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
        elif argv[:3] == ["scry", "memory", "verify"]:
            out, code = self.verify
            rest = argv[3:]
            claim = rest[rest.index("--") + 1] if "--" in rest else rest[0]
            if claim.startswith("-") and "--" not in rest:
                out, code = "", 2
        elif argv[:2] == ["scry", "index"]:
            if self.index_code is None:
                raise subprocess.TimeoutExpired(argv, kwargs["timeout"])
            code = self.index_code
        elif argv[:3] == ["scry", "memory", "stale"]:
            if self.stale_code is None:
                raise subprocess.TimeoutExpired(argv, kwargs["timeout"])
            out, code = self.stale, self.stale_code
        elif argv[:3] == ["scry", "memory", "reaffirm"]:
            out, code = f"reaffirmed #{argv[3]}", self.reaffirm_code
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


def memory_episode():
    return lesson(kind="episode", anchors=[])


class CaptureCase(unittest.TestCase):
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



class CaptureTests(CaptureCase):
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

    def test_leading_dash_text_goes_after_the_separator(self):
        out = {
            "memories": [lesson(content="--force pushes need a lease")],
            "reinforce": [],
            "contradict": [],
        }
        fake = Fake(out)
        self.run_capture(fake)
        (remember,) = fake.scry("scry", "remember")
        self.assertEqual(remember[-2:], ["--", "--force pushes need a lease"])
        recall = fake.scry("scry", "recall")[0]
        self.assertEqual(recall[-2], "--")

    def test_recall_does_not_touch_access_stats(self):
        fake = Fake({"memories": [], "reinforce": [], "contradict": []})
        self.run_capture(fake)
        self.assertIn("--no-touch", fake.scry("scry", "recall")[0])

    def verified(self, verify):
        out = {"memories": [lesson()], "reinforce": [], "contradict": []}
        fake = Fake(out, verify=verify)
        return fake, self.run_capture(fake)

    def test_supported_memory_keeps_kind_and_anchors(self):
        fake, result = self.verified(("supported 0.83", 0))
        self.assertEqual(result["verified"], [["lesson", 0.83]])
        self.assertEqual(result["downgraded"], 0)
        (verify,) = fake.scry("scry", "memory", "verify")
        self.assertEqual(verify[-1], "cc is needed")
        self.assertIn(f"error: {QUOTE}", verify[4])
        self.assertIn("lesson", fake.scry("scry", "remember")[0])
        self.assertIn("--anchor", fake.scry("scry", "remember")[0])

    def test_unsupported_memory_becomes_episode_without_anchors(self):
        fake, result = self.verified(("unsupported 0.04", 0))
        self.assertEqual(result["downgraded"], 1)
        (remember,) = fake.scry("scry", "remember")
        self.assertIn("episode", remember)
        self.assertNotIn("--anchor", remember)

    def test_not_configured_keeps_quote_only_result(self):
        fake, result = self.verified(("not configured", 0))
        self.assertEqual(result["verify"], "not configured")
        self.assertEqual(result["downgraded"], 0)
        self.assertIn("lesson", fake.scry("scry", "remember")[0])

    def test_verify_failure_fails_closed(self):
        fake, result = self.verified(("", 2))
        self.assertEqual(result["downgraded"], 1)
        self.assertEqual(len(result["verify_errors"]), 1)
        self.assertIn("episode", fake.scry("scry", "remember")[0])

    def test_dash_leading_claim_is_verified_after_the_separator(self):
        out = {
            "memories": [lesson(content="--force pushes need a lease")],
            "reinforce": [],
            "contradict": [],
        }
        fake = Fake(out, verify=("supported 0.9", 0))
        result = self.run_capture(fake)
        self.assertEqual(result["verified"], [["lesson", 0.9]])
        self.assertNotIn("verify_errors", result)
        (verify,) = fake.scry("scry", "memory", "verify")
        self.assertEqual(verify[-2:], ["--", "--force pushes need a lease"])

    def test_unsupported_drops_supersedes(self):
        out = {
            "memories": [lesson(supersedes=9)],
            "reinforce": [],
            "contradict": [],
        }
        fake = Fake(out, verify=("unsupported 0.1", 0))
        self.run_capture(fake)
        self.assertEqual(fake.scry("scry", "memory", "supersede"), [])

    def test_episodes_are_not_verified(self):
        fake = Fake({"memories": [memory_episode()], "reinforce": [], "contradict": []})
        self.run_capture(fake)
        self.assertEqual(fake.scry("scry", "memory", "verify"), [])

    def test_few_tool_calls_skip_without_advancing(self):
        Path(self.payload["transcript_path"]).write_text(
            json.dumps({"type": "user", "message": {"content": "hi there"}}) + "\n"
        )
        fake = Fake({})
        self.assertEqual(self.run_capture(fake)["skipped"], "few tool calls")
        self.assertFalse((self.root / "state" / "s1.json").exists())


STALE_LINE = "[fact #31] (stale)\tsrc/lib.rs\tcc must be on PATH\n"


def reaffirm_output(**extra):
    claim = {"id": 31, "evidence": {"entry": 5, "quote": QUOTE}, **extra}
    return {"memories": [], "reinforce": [], "contradict": [], "reaffirm": [claim]}


class ReaffirmTests(CaptureCase):
    def test_stale_lookup_uses_edited_repo_paths_and_fills_its_own_block(self):
        fake = Fake(reaffirm_output(), stale=STALE_LINE)
        self.run_capture(fake)
        self.assertEqual(
            fake.scry("scry", "memory", "stale"),
            [["scry", "memory", "stale", "--file", "src/lib.rs"]],
        )
        ((_, kwargs),) = [c for c in fake.calls if c[0][0] == "claude"]
        block = kwargs["input"].split("Stale memories anchored")[1]
        self.assertIn("[fact #31] cc must be on PATH", block.split("Transcript:")[0])

    def test_no_stale_lookup_when_nothing_was_edited(self):
        reads = {
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "tool_use", "id": str(i), "name": "Read", "input": {}}
                    for i in range(3)
                ]
            },
        }
        Path(self.payload["transcript_path"]).write_text(json.dumps(reads) + "\n")
        fake = Fake(reaffirm_output(), stale=STALE_LINE)
        self.run_capture(fake)
        self.assertEqual(fake.scry("scry", "memory", "stale"), [])

    def test_proven_reaffirm_is_verified_then_applied_with_citation(self):
        fake = Fake(reaffirm_output(), stale=STALE_LINE, verify=("supported 0.9", 0))
        result = self.run_capture(fake)
        self.assertEqual(result["reaffirmed"], [31])
        (verify,) = fake.scry("scry", "memory", "verify")
        self.assertEqual(verify[-1], "cc must be on PATH")
        self.assertIn(f"error: {QUOTE}", verify[4])
        self.assertEqual(
            fake.scry("scry", "memory", "reaffirm"),
            [["scry", "memory", "reaffirm", "31", "--evidence", f"[#5] {QUOTE}"]],
        )

    def test_index_runs_once_before_the_stale_lookup(self):
        out = reaffirm_output()
        out["memories"] = [lesson()]
        fake = Fake(out, stale=STALE_LINE)
        self.run_capture(fake)
        order = [" ".join(a[:3]) for a, _ in fake.calls if a[0] == "scry"]
        self.assertLess(order.index("scry index"), order.index("scry memory stale"))
        self.assertEqual(len(fake.scry("scry", "index")), 1)

    def test_not_configured_verifier_still_reaffirms(self):
        fake = Fake(reaffirm_output(), stale=STALE_LINE)
        self.assertEqual(self.run_capture(fake)["reaffirmed"], [31])

    def test_reaffirm_verify_failure_is_recorded_and_not_applied(self):
        fake = Fake(reaffirm_output(), stale=STALE_LINE, verify=("", 2))
        result = self.run_capture(fake)
        self.assertEqual((result["reaffirmed"], result["reaffirm_failed"]), ([], [31]))
        self.assertEqual(len(result["verify_errors"]), 1)
        self.assertEqual(fake.scry("scry", "memory", "reaffirm"), [])

    def test_unconfigured_verifier_is_called_once_across_reaffirms(self):
        second = "[fact #32] (stale)\tsrc/lib.rs\tother claim\n"
        out = reaffirm_output()
        out["reaffirm"].append({"id": 32, "evidence": {"entry": 5, "quote": QUOTE}})
        fake = Fake(out, stale=STALE_LINE + second)
        result = self.run_capture(fake)
        self.assertEqual(result["reaffirmed"], [31, 32])
        self.assertEqual(len(fake.scry("scry", "memory", "verify")), 1)

    def test_unsupported_reaffirm_is_dropped(self):
        fake = Fake(
            reaffirm_output(), stale=STALE_LINE, verify=("unsupported 0.1", 0)
        )
        result = self.run_capture(fake)
        self.assertEqual((result["reaffirmed"], result["reaffirm_failed"]), ([], []))
        self.assertEqual(result["reaffirm_unsupported"], [31])
        self.assertEqual(fake.scry("scry", "memory", "reaffirm"), [])

    def test_id_not_in_stale_block_is_ignored(self):
        fake = Fake(reaffirm_output(), stale="")
        self.assertEqual(self.run_capture(fake)["reaffirmed"], [])
        self.assertEqual(fake.scry("scry", "memory", "reaffirm"), [])

    def test_stale_tsv_keeps_spaced_paths_and_unescapes_content(self):
        line = "[fact #31] (stale)\tsrc/my file.rs,b.rs\tline one\\nline two\\\\n\n"
        fake = Fake(reaffirm_output(), stale=line, verify=("supported 0.9", 0))
        self.run_capture(fake)
        (verify,) = fake.scry("scry", "memory", "verify")
        self.assertEqual(verify[-1], "line one\nline two\\n")

    def test_unproven_citation_is_ignored(self):
        bad = reaffirm_output(evidence={"entry": 1, "quote": "why does the build fail?"})
        fake = Fake(bad, stale=STALE_LINE)
        self.assertEqual(self.run_capture(fake)["reaffirmed"], [])

    def test_reaffirm_exit_failure_is_logged(self):
        fake = Fake(reaffirm_output(), stale=STALE_LINE, reaffirm_code=1)
        result = self.run_capture(fake)
        self.assertEqual((result["reaffirmed"], result["reaffirm_failed"]), ([], [31]))

    def test_stale_memory_can_still_be_superseded(self):
        out = {
            "memories": [lesson(supersedes=31)],
            "reinforce": [],
            "contradict": [],
            "reaffirm": [],
        }
        fake = Fake(out, stale=STALE_LINE)
        self.run_capture(fake)
        self.assertEqual(
            fake.scry("scry", "memory", "supersede"),
            [["scry", "memory", "supersede", "31", "50"]],
        )

    def test_stale_id_cannot_be_reinforced_or_contradicted(self):
        claim = {"id": 31, "evidence": {"entry": 5, "quote": QUOTE}}
        out = {
            "memories": [],
            "reinforce": [claim],
            "contradict": [claim],
            "reaffirm": [],
        }
        fake = Fake(out, stale=STALE_LINE)
        result = self.run_capture(fake)
        self.assertEqual((result["helpful"], result["noise"]), (0, 0))
        self.assertEqual(fake.scry("scry", "memory", "helpful"), [])
        self.assertEqual(fake.scry("scry", "memory", "noise"), [])

    def test_stale_id_shown_by_the_brief_still_cannot_be_marked(self):
        claim = {"id": 9, "evidence": {"entry": 5, "quote": QUOTE}}
        out = {
            "memories": [],
            "reinforce": [claim],
            "contradict": [claim],
            "reaffirm": [],
        }
        fake = Fake(out, stale="[decision #9] (stale)\tsrc/lib.rs\tuse rrf\n")
        result = self.run_capture(fake)
        self.assertEqual((result["helpful"], result["noise"]), (0, 0))
        self.assertEqual(fake.scry("scry", "memory", "helpful"), [])

    def test_stale_tsv_unescapes_content_with_escaped_paths(self):
        line = "[fact #31] (stale)\ta\\,b.rs\tcol\\tumn\\r\\n\n"
        fake = Fake(reaffirm_output(), stale=line)
        with mock.patch("subprocess.run", fake):
            self.assertEqual(
                capture_scry.stale(["a,b.rs"], "."),
                {31: ("fact", "col\tumn\r\n")},
            )

    def test_stale_timeout_is_logged_and_the_pass_continues(self):
        out = reaffirm_output()
        out["memories"] = [lesson()]
        fake = Fake(out, stale=STALE_LINE, stale_code=None)
        result = self.run_capture(fake)
        self.assertIn("stale_error", result)
        self.assertEqual(result["written"], 1)

    def assert_index_failure_is_survived(self, code):
        out = reaffirm_output()
        out["memories"] = [lesson()]
        fake = Fake(out, stale=STALE_LINE, index_code=code)
        result = self.run_capture(fake)
        self.assertIn("scry index", result["index_error"])
        self.assertEqual(fake.scry("scry", "memory", "stale"), [])
        self.assertEqual(fake.scry("scry", "memory", "reaffirm"), [])
        self.assertEqual(result["written"], 1)
        (remember,) = fake.scry("scry", "remember")
        self.assertNotIn("--anchor", remember)

    def test_index_exit_failure_is_logged_and_skips_stale_and_anchors(self):
        self.assert_index_failure_is_survived(1)

    def test_index_timeout_is_logged_and_skips_stale_and_anchors(self):
        self.assert_index_failure_is_survived(None)

    def test_stale_failure_is_logged_and_the_pass_continues(self):
        out = reaffirm_output()
        out["memories"] = [lesson()]
        fake = Fake(out, stale=STALE_LINE, stale_code=2)
        result = self.run_capture(fake)
        self.assertIn("exited 2", result["stale_error"])
        self.assertEqual(result["written"], 1)
        self.assertEqual(result["reaffirmed"], [])
        (remember,) = fake.scry("scry", "remember")
        self.assertIn("--anchor", remember)


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
