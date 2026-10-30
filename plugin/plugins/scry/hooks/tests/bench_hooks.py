import sys
from pathlib import Path

import pytest

HOOKS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HOOKS))

import capture_transcript as ct
import capture_verify as cv

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "session.jsonl"
REPEATS = 400


@pytest.fixture(scope="module")
def session(tmp_path_factory):
    path = tmp_path_factory.mktemp("session") / "session.jsonl"
    path.write_text(FIXTURE.read_text() * REPEATS)
    return str(path)


def test_load_transcript(benchmark, session):
    entries, _ = benchmark(ct.load, session, 0)
    assert len(entries) > REPEATS


def test_render_capped(benchmark, session):
    entries, _ = ct.load(session, 0)
    text, kept = benchmark(ct.render, entries, 20_000)
    assert "omitted" in text
    assert kept


def test_injected_memories(benchmark, session):
    found = benchmark(ct.injected_memories, session)
    assert found == {
        12: ("lesson", "vec partition keys come first"),
        40: ("fact", "chunker is server side"),
    }


def test_plan_memories(benchmark, session):
    entries, _ = ct.load(session, 0)
    _, kept = ct.render(entries, 20_000)
    quoted = next(e for e in kept.values() if e.kind == "tool_result")
    evidence = {"entry": quoted.n, "quote": quoted.body[:40]}
    output = {
        "memories": [
            {
                "kind": "lesson",
                "content": "linker is missing on arm",
                "evidence": evidence,
                "anchors": ["/repo/src/lib.rs:3"],
            },
            {"kind": "episode", "content": "debugged the arm build"},
        ],
        "reinforce": [{"id": 12, "evidence": evidence}],
        "contradict": [{"id": 40, "evidence": evidence}],
    }
    out = benchmark(cv.plan, output, kept, {"/repo/src/lib.rs"}, {12, 40})
    assert len(out["memories"]) == 2
