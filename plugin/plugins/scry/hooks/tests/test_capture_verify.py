import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import capture_verify as cv
from capture_transcript import Entry

KEPT = {
    1: Entry(1, "user", "always run  the linker\nbefore pushing, please"),
    2: Entry(2, "assistant", "I think the linker is missing entirely"),
    3: Entry(3, "tool_result", "error: linker `cc` not found on this machine"),
}
QUOTE = "linker `cc` not found on this machine"


def cited(entry, quote):
    return {"entry": entry, "quote": quote}


def memory(kind, evidence, **extra):
    return {"kind": kind, "content": "c", "evidence": evidence, "anchors": [], "pain": 3, **extra}


class HoldsTests(unittest.TestCase):
    def test_verbatim_quote_holds(self):
        self.assertTrue(cv.holds(cited(3, QUOTE), KEPT))

    def test_whitespace_is_normalized(self):
        self.assertTrue(cv.holds(cited(1, "run the linker before pushing"), KEPT))

    def test_short_quote_fails(self):
        self.assertFalse(cv.holds(cited(3, "linker `cc`"), KEPT))

    def test_missing_quote_fails(self):
        self.assertFalse(cv.holds(cited(3, "this text is not in the entry at all"), KEPT))

    def test_unknown_entry_fails(self):
        self.assertFalse(cv.holds(cited(9, QUOTE), KEPT))

    def test_missing_evidence_fails(self):
        self.assertFalse(cv.holds(None, KEPT))


class SettleTests(unittest.TestCase):
    def test_lesson_proven_by_tool_result_keeps_kind_and_anchors(self):
        out = cv.settle(memory("lesson", cited(3, QUOTE), anchors=["src/lib.rs:1-5", "other.rs"]), KEPT, {"src/lib.rs"})
        self.assertEqual(out["kind"], "lesson")
        self.assertEqual(out["anchors"], ["src/lib.rs:1-5"])
        self.assertEqual(out["evidence"], f"[#3] {QUOTE}")

    def test_lesson_cited_to_assistant_prose_downgrades_to_episode(self):
        out = cv.settle(memory("lesson", cited(2, "the linker is missing entirely"), anchors=["a.rs"]), KEPT, {"a.rs"})
        self.assertEqual(out["kind"], "episode")
        self.assertEqual(out["anchors"], [])

    def test_lesson_cited_to_user_prompt_downgrades(self):
        out = cv.settle(memory("lesson", cited(1, "run the linker before pushing")), KEPT, set())
        self.assertEqual(out["kind"], "episode")

    def test_decision_accepts_user_prompt(self):
        out = cv.settle(memory("decision", cited(1, "run the linker before pushing")), KEPT, set())
        self.assertEqual(out["kind"], "decision")

    def test_unproven_fact_downgrades_and_drops_evidence(self):
        out = cv.settle(memory("fact", cited(3, "a quote that is not present anywhere")), KEPT, set())
        self.assertEqual(out["kind"], "episode")
        self.assertIsNone(out["evidence"])

    def test_episode_needs_no_proof(self):
        out = cv.settle(memory("episode", None), KEPT, set())
        self.assertEqual(out["kind"], "episode")

    def test_supersedes_honored_only_when_kind_survives(self):
        kept = cv.settle(memory("lesson", cited(3, QUOTE), supersedes=7), KEPT, set())
        dropped = cv.settle(memory("lesson", cited(2, "the linker is missing entirely"), supersedes=7), KEPT, set())
        self.assertEqual(kept["supersedes"], 7)
        self.assertIsNone(dropped["supersedes"])

    def test_anchor_must_be_an_edited_file(self):
        out = cv.settle(memory("lesson", cited(3, QUOTE), anchors=["untouched.rs"]), KEPT, {"src/lib.rs"})
        self.assertEqual(out["anchors"], [])

    def test_content_is_capped(self):
        out = cv.settle(memory("episode", None, content="x" * 500), KEPT, set())
        self.assertEqual(len(out["content"]), cv.MAX_CONTENT)


class PlanTests(unittest.TestCase):
    def test_unproven_reinforce_and_contradict_are_ignored(self):
        out = cv.plan(
            {
                "memories": [],
                "reinforce": [{"id": 1, "evidence": cited(3, QUOTE)}, {"id": 2, "evidence": cited(2, "no such text in the entry")}],
                "contradict": [{"id": 3, "evidence": cited(3, QUOTE)}, {"id": 4, "evidence": None}],
            },
            KEPT,
            set(),
        )
        self.assertEqual(out["helpful"], [1])
        self.assertEqual(out["noise"], [3])

    def test_memories_capped_at_five(self):
        out = cv.plan({"memories": [memory("episode", None)] * 8}, KEPT, set())
        self.assertEqual(len(out["memories"]), cv.MAX_MEMORIES)


if __name__ == "__main__":
    unittest.main()
