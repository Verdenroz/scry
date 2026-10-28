import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import capture_transcript as ct

FIXTURE = str(Path(__file__).resolve().parent / "fixtures" / "session.jsonl")


class LoadTests(unittest.TestCase):
    def test_entries_skip_meta_sidechain_and_garbage(self):
        entries, total = ct.load(FIXTURE, 0)
        self.assertEqual(total, 9)
        self.assertEqual(
            [e.kind for e in entries],
            [
                "user",
                "assistant",
                "tool_use",
                "tool_result",
                "tool_use",
                "tool_use",
                "tool_result",
                "tool_result",
            ],
        )
        self.assertEqual([e.n for e in entries], list(range(1, 9)))

    def test_compact_summary_and_list_text_blocks(self):
        summary = {"type": "user", "isCompactSummary": True, "message": {"content": "sum"}}
        blocks = {
            "type": "user",
            "message": {"content": [{"type": "text", "text": "always use X"}]},
        }
        self.assertEqual(list(ct.entry_parts(summary)), [])
        self.assertEqual(
            list(ct.entry_parts(blocks)), [("user", "always use X", "", False)]
        )

    def test_list_content_tool_result_is_joined(self):
        entries, _ = ct.load(FIXTURE, 0)
        self.assertEqual(entries[6].body, "edited ok")

    def test_offset_skips_processed_lines_and_renumbers(self):
        entries, total = ct.load(FIXTURE, 6)
        self.assertEqual(total, 9)
        self.assertEqual(entries[0].n, 1)
        self.assertEqual(entries[0].kind, "tool_use")

    def test_edited_paths_only_from_edit_tools(self):
        entries, _ = ct.load(FIXTURE, 0)
        self.assertEqual(ct.edited_paths(entries), ["/repo/src/lib.rs"])

    def test_first_prompt_and_tool_call_count(self):
        entries, _ = ct.load(FIXTURE, 0)
        self.assertEqual(ct.first_prompt(entries), "why does the build fail on arm?")
        self.assertEqual(ct.tool_calls(entries), 3)

    def test_injected_memories_parse_kind_id_and_content(self):
        self.assertEqual(
            ct.injected_memories(FIXTURE),
            {
                12: ("lesson", "vec partition keys come first"),
                40: ("fact", "chunker is server side"),
            },
        )


def results_after(tool, args, offset=0):
    use = {
        "type": "assistant",
        "message": {
            "content": [{"type": "tool_use", "id": "x", "name": tool, "input": args}]
        },
    }
    result = {
        "type": "user",
        "message": {
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "x",
                    "content": "[lesson #12] echoed",
                }
            ]
        },
    }
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl") as handle:
        handle.write(json.dumps(use) + "\n" + json.dumps(result) + "\n")
        handle.flush()
        return ct.load(handle.name, offset)[0][-1]


class EchoTests(unittest.TestCase):
    def test_scry_memory_commands_are_echoed(self):
        for command in (
            'scry recall "vec"',
            "cd x && scry memory brief",
            'scry remember "a"',
        ):
            self.assertTrue(results_after("Bash", {"command": command}).echoed, command)

    def test_edit_results_are_echoed(self):
        self.assertTrue(results_after("Write", {"file_path": "/a"}).echoed)

    def test_ordinary_results_are_not(self):
        self.assertFalse(results_after("Bash", {"command": "cargo build"}).echoed)
        self.assertFalse(results_after("Bash", {"command": "scry index"}).echoed)

    def test_call_before_offset_still_taints_result(self):
        self.assertTrue(
            results_after("Bash", {"command": "scry recall x"}, offset=1).echoed
        )


class WebTests(unittest.TestCase):
    def test_web_tool_results_are_omitted_and_never_proof(self):
        for tool, args in (
            ("WebFetch", {"url": "https://x.dev"}),
            ("WebSearch", {"query": "q"}),
            ("mcp__tavily__tavily_search_web", {}),
            ("mcp__ref__fetch_docs", {}),
            ("Bash", {"command": 'scry --web --answer "q"'}),
            ("Bash", {"command": "cd x && curl -s https://x.dev"}),
            ("Bash", {"command": "wget https://x.dev/a.tgz"}),
        ):
            entry = results_after(tool, args)
            self.assertEqual(entry.body, ct.WEB_OMITTED, tool)
            self.assertTrue(entry.echoed, tool)

    def test_web_tool_calls_stay_visible(self):
        entries, _ = ct.load(web_transcript(), 0)
        self.assertIn("WebFetch", entries[0].body)
        self.assertEqual(entries[1].body, ct.WEB_OMITTED)

    def test_local_scry_search_is_not_web(self):
        entry = results_after("Bash", {"command": 'scry "vec" --repo x'})
        self.assertEqual(entry.body, "[lesson #12] echoed")


def web_transcript():
    handle = tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False)
    use = {
        "type": "assistant",
        "message": {
            "content": [
                {
                    "type": "tool_use",
                    "id": "w",
                    "name": "WebFetch",
                    "input": {"url": "https://x.dev"},
                }
            ]
        },
    }
    result = {
        "type": "user",
        "message": {
            "content": [
                {"type": "tool_result", "tool_use_id": "w", "content": "page text"}
            ]
        },
    }
    handle.write(json.dumps(use) + "\n" + json.dumps(result) + "\n")
    handle.close()
    return handle.name


class RenderTests(unittest.TestCase):
    def test_numbered_lines(self):
        entries, _ = ct.load(FIXTURE, 0)
        text, kept = ct.render(entries)
        self.assertIn("[#1] user: why does the build fail on arm?", text)
        self.assertIn('[#3] tool_use: Bash {"command":"cargo build"}', text)
        self.assertEqual(set(kept), set(range(1, 9)))

    def test_tool_result_truncated_to_tail(self):
        obj = {
            "type": "user",
            "message": {
                "content": [{"type": "tool_result", "content": "a" * 3000 + "END"}]
            },
        }
        ((_, body, _, _),) = ct.entry_parts(obj)
        self.assertEqual(len(body), ct.RESULT_TAIL + 3)
        self.assertTrue(body.startswith("...") and body.endswith("END"))

    def test_cap_keeps_head_and_tail_and_drops_middle(self):
        entries = [ct.Entry(i, "assistant", "w" * 100) for i in range(1, 101)]
        text, kept = ct.render(entries, cap=3000)
        self.assertIn(1, kept)
        self.assertIn(100, kept)
        self.assertNotIn(50, kept)
        self.assertIn("omitted", text)
        self.assertLessEqual(len(text), 3200)


if __name__ == "__main__":
    unittest.main()
