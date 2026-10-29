MAX_MEMORIES = 5
MAX_CONTENT = 300
MIN_QUOTE = 20
REINFORCE_KINDS = {"tool_result"}
CONTRADICT_KINDS = {"tool_result", "user"}
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
    if entry.echoed or (kinds is not None and entry.kind not in kinds):
        return False
    if entry.kind == "tool_result" and not entry.provable:
        return False
    return quote in normalize(entry.body)


def cite(evidence) -> str:
    return f"[#{evidence['entry']}] {normalize(evidence['quote'])}"


def settle(memory, kept, edited):
    kind = memory.get("kind")
    evidence = memory.get("evidence")
    proven = kind in PROOF_KINDS and holds(evidence, kept, PROOF_KINDS[kind])
    final = kind if proven or kind == "episode" else "episode"
    cited = holds(evidence, kept)
    anchors = [a for a in memory.get("anchors") or [] if a.split(":")[0] in edited]
    return {
        "kind": final,
        "content": memory.get("content", "")[:MAX_CONTENT],
        "evidence": cite(evidence) if cited else None,
        "source": kept[evidence["entry"]].body if cited else None,
        "anchors": [] if final == "episode" else anchors,
        "pain": memory.get("pain"),
        "supersedes": memory.get("supersedes") if final == kind and final in PROOF_KINDS else None,
    }


def downgrade(memory):
    return {**memory, "kind": "episode", "anchors": [], "supersedes": None}


def proven_ids(claims, kept, kinds, shown) -> list:
    return [
        c["id"]
        for c in claims or []
        if c["id"] in shown and holds(c.get("evidence"), kept, kinds)
    ]


def reaffirmable(claims, kept, stale, superseded=()) -> list:
    taken = set(superseded)
    out = []
    for c in claims or []:
        if c["id"] in taken or c["id"] not in stale:
            continue
        if not holds(c.get("evidence"), kept, REINFORCE_KINDS):
            continue
        taken.add(c["id"])
        out.append(
            {
                "id": c["id"],
                "content": stale[c["id"]][1],
                "evidence": cite(c["evidence"]),
                "source": kept[c["evidence"]["entry"]].body,
            }
        )
    return out


def plan(output, kept, edited, shown, stale=()):
    memories = [settle(m, kept, edited) for m in output.get("memories") or []]
    replaceable = set(shown) | set(stale)
    shown = set(shown) - set(stale)
    memories = [
        {**m, "supersedes": m["supersedes"] if m["supersedes"] in replaceable else None}
        for m in memories[:MAX_MEMORIES]
    ]
    superseded = {m["supersedes"] for m in memories}
    return {
        "memories": memories,
        "helpful": proven_ids(output.get("reinforce"), kept, REINFORCE_KINDS, shown),
        "noise": proven_ids(output.get("contradict"), kept, CONTRADICT_KINDS, shown),
        "reaffirm": reaffirmable(output.get("reaffirm"), kept, stale, superseded),
    }
