import json
import re
from dataclasses import dataclass

RESULT_TAIL = 1500
INPUT_CAP = 400
RENDER_CAP = 120_000
EDIT_TOOLS = {"Edit", "Write", "NotebookEdit", "MultiEdit"}
INJECTED = re.compile(r"^\[(\w+) #(\d+)\](?: \([^)]*\))*\s*(.*)$", re.MULTILINE)


@dataclass(frozen=True)
class Entry:
    n: int
    kind: str
    body: str
    edited: str = ""


def tool_result_text(content) -> str:
    if isinstance(content, str):
        return content
    return "\n".join(
        block.get("text", "") for block in content if isinstance(block, dict)
    )


def entry_parts(obj):
    content = (obj.get("message") or {}).get("content")
    if obj.get("type") == "user":
        if isinstance(content, str):
            if not obj.get("isMeta"):
                yield "user", content, ""
            return
        for block in content or []:
            if block.get("type") == "tool_result":
                text = tool_result_text(block.get("content") or "")
                yield "tool_result", ("..." + text[-RESULT_TAIL:] if len(text) > RESULT_TAIL else text), ""
    elif obj.get("type") == "assistant":
        for block in content or []:
            if block.get("type") == "text":
                yield "assistant", block.get("text", ""), ""
            elif block.get("type") == "tool_use":
                name = block.get("name")
                args = block.get("input") or {}
                shown = json.dumps(args, separators=(",", ":"))[:INPUT_CAP]
                edited = args.get("file_path") or args.get("notebook_path") or ""
                yield "tool_use", f"{name} {shown}", edited if name in EDIT_TOOLS else ""


def read_lines(path: str):
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            try:
                yield json.loads(line)
            except json.JSONDecodeError:
                yield {}


def load(path: str, offset: int):
    entries = []
    total = 0
    for total, obj in enumerate(read_lines(path), 1):
        if total <= offset or obj.get("isSidechain"):
            continue
        for kind, body, edited in entry_parts(obj):
            entries.append(Entry(len(entries) + 1, kind, body, edited))
    return entries, total


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


def injected_memories(path: str) -> dict:
    found = {}
    for obj in read_lines(path):
        if obj.get("type") not in ("attachment", "system"):
            continue
        for text in strings(obj):
            for kind, ident, content in INJECTED.findall(text):
                found[int(ident)] = (kind, content)
    return found


def first_prompt(entries) -> str:
    return next((e.body for e in entries if e.kind == "user"), "")


def tool_calls(entries) -> int:
    return sum(e.kind == "tool_use" for e in entries)


def edited_paths(entries) -> list:
    return sorted({e.edited for e in entries if e.edited})


def render_line(entry: Entry) -> str:
    return f"[#{entry.n}] {entry.kind}: {entry.body}"


def render(entries, cap: int = RENDER_CAP):
    sizes = [len(render_line(e)) + 1 for e in entries]
    if sum(sizes) <= cap:
        return "\n".join(map(render_line, entries)), {e.n: e for e in entries}
    head, used = [], 0
    for entry, size in zip(entries, sizes):
        if used + size > cap // 3:
            break
        head.append(entry)
        used += size
    tail, budget = [], cap - used
    for entry, size in zip(reversed(entries[len(head):]), reversed(sizes[len(head):])):
        if size > budget:
            break
        tail.append(entry)
        budget -= size
    tail.reverse()
    gap_start = len(head) + 1
    gap_end = tail[0].n - 1 if tail else len(entries)
    lines = [render_line(e) for e in head]
    lines.append(f"[... entries #{gap_start}-#{gap_end} omitted ...]")
    lines += [render_line(e) for e in tail]
    return "\n".join(lines), {e.n: e for e in head + tail}
