import json
import os
import subprocess
import sys
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import capture_model
import capture_scry
import capture_transcript
import capture_verify

DEBUG_LOG_FILE = Path(os.environ.get("SCRY_CAPTURE_LOG", "/tmp/scry-capture.log"))
STATE_DIR = Path(
    os.environ.get("SCRY_CAPTURE_DIR", Path.home() / ".cache" / "scry" / "capture")
)
MIN_TOOL_CALLS = 3


def debug_log(record: dict) -> None:
    try:
        DEBUG_LOG_FILE.parent.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        with open(DEBUG_LOG_FILE, "a", encoding="utf-8") as handle:
            handle.write(json.dumps({"ts": stamp, **record}) + "\n")
    except Exception:
        pass


def read_hook_input():
    raw = sys.stdin.read()
    if not raw.strip():
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError as exc:
        debug_log({"error": f"Failed to decode JSON: {exc}"})
        return None


def load_state(state_file: Path) -> dict:
    if not state_file.exists():
        return {"offset": 0, "helpful": [], "noise": []}
    return json.loads(state_file.read_text())


def save_state(state_file: Path, state: dict) -> None:
    state_file.parent.mkdir(parents=True, exist_ok=True)
    state_file.write_text(json.dumps(state))


def repo_relative(paths, cwd: str) -> set:
    root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], cwd=cwd, capture_output=True, text=True
    )
    if root.returncode != 0:
        return set()
    top = root.stdout.strip()
    relative = (os.path.relpath(p, top) for p in paths)
    return {r for r in relative if not r.startswith("..")}


def write_plan(plan: dict, cwd: str) -> int:
    if any(m["anchors"] for m in plan["memories"]):
        capture_scry.index(cwd)
    written = 0
    for memory in plan["memories"]:
        ident = capture_scry.remember(memory, cwd)
        if ident is None:
            continue
        written += 1
        if memory["supersedes"] is not None:
            capture_scry.supersede(memory["supersedes"], ident, cwd)
    for ident in plan["helpful"]:
        capture_scry.mark("helpful", ident, cwd)
    for ident in plan["noise"]:
        capture_scry.mark("noise", ident, cwd)
    return written


def unmarked(ids, done) -> list:
    return [i for i in dict.fromkeys(ids) if i not in done]


def capture(payload: dict) -> dict:
    cwd = payload.get("cwd") or os.getcwd()
    path = payload["transcript_path"]
    state_file = STATE_DIR / f"{payload['session_id']}.json"
    state = load_state(state_file)
    entries, total = capture_transcript.load(path, state["offset"])
    calls = capture_transcript.tool_calls(entries)
    if calls < MIN_TOOL_CALLS:
        return {"skipped": "few tool calls", "tool_calls": calls}

    existing = capture_transcript.injected_memories(path)
    existing.update(capture_scry.recall(capture_transcript.first_prompt(entries), cwd))
    text, kept = capture_transcript.render(entries)
    edited = repo_relative(capture_transcript.edited_paths(entries), cwd)
    output = capture_model.call(
        capture_model.build_prompt(text, existing), str(STATE_DIR)
    )
    plan = capture_verify.plan(output, kept, edited)
    plan["helpful"] = unmarked(plan["helpful"], state["helpful"])
    plan["noise"] = unmarked(plan["noise"], state["noise"])
    written = write_plan(plan, cwd)
    save_state(
        state_file,
        {
            "offset": total,
            "helpful": state["helpful"] + plan["helpful"],
            "noise": state["noise"] + plan["noise"],
        },
    )
    return {
        "tool_calls": calls,
        "proposed": len(output.get("memories") or []),
        "written": written,
        "episodes": sum(m["kind"] == "episode" for m in plan["memories"]),
        "helpful": len(plan["helpful"]),
        "noise": len(plan["noise"]),
    }


def worker(payload: dict) -> None:
    record = {
        "session": payload.get("session_id"),
        "event": payload.get("hook_event_name"),
    }
    try:
        record.update(capture(payload))
    except Exception as exc:
        record["error"] = f"{type(exc).__name__}: {exc}"
    debug_log(record)


def spawn(raw: str) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    child = subprocess.Popen(
        [sys.executable, __file__, "--worker"],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    child.stdin.write(raw.encode())
    child.stdin.close()


if __name__ == "__main__":
    if os.environ.get("SCRY_CAPTURE") == "1":
        sys.exit(0)
    if "--worker" in sys.argv:
        worker(read_hook_input() or {})
    else:
        raw = sys.stdin.read()
        if raw.strip():
            spawn(raw)
    sys.exit(0)
