MAX_MEMORIES = 5
MAX_CONTENT = 300
MIN_QUOTE = 20
PROOF_KINDS = {
    "lesson": {"tool_result"},
    "fact": {"tool_result"},
    "decision": {"user", "tool_result"},
    "convention": {"user", "tool_result"},
}


def normalize(text: str) -> str:
    return " ".join(text.split())


def holds(evidence, kept, kinds=None) -> bool:
    evidence = evidence or {}
    entry = kept.get(evidence.get("entry"))
    quote = normalize(evidence.get("quote", ""))
    if entry is None or len(quote) < MIN_QUOTE:
        return False
    if kinds is not None and entry.kind not in kinds:
        return False
    return quote in normalize(entry.body)


def cite(evidence) -> str:
    return f"[#{evidence['entry']}] {normalize(evidence['quote'])}"


def settle(memory, kept, edited):
    kind = memory.get("kind")
    evidence = memory.get("evidence")
    proven = kind in PROOF_KINDS and holds(evidence, kept, PROOF_KINDS[kind])
    final = kind if proven or kind == "episode" else "episode"
    anchors = [a for a in memory.get("anchors") or [] if a.split(":")[0] in edited]
    return {
        "kind": final,
        "content": memory.get("content", "")[:MAX_CONTENT],
        "evidence": cite(evidence) if holds(evidence, kept) else None,
        "anchors": [] if final == "episode" else anchors,
        "pain": memory.get("pain"),
        "supersedes": memory.get("supersedes") if final == kind else None,
    }


def plan(output, kept, edited):
    memories = output.get("memories") or []
    return {
        "memories": [settle(m, kept, edited) for m in memories[:MAX_MEMORIES]],
        "helpful": [r["id"] for r in output.get("reinforce") or [] if holds(r.get("evidence"), kept)],
        "noise": [r["id"] for r in output.get("contradict") or [] if holds(r.get("evidence"), kept)],
    }
