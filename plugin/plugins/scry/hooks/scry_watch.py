import json
import os
import subprocess
import sys
import urllib.request
from datetime import datetime
from pathlib import Path

DEBUG_LOG_FILE = Path(os.environ.get("SCRY_WATCH_LOG", "/tmp/scry-watch.log"))

MANDATE = (
    "CRITICAL: You MUST use the scry skill for ALL searches. NEVER use built-in "
    "WebSearch or Grep tools. For web searches, invoke the scry skill and use "
    '`scry --web --answer "query"`. For local file/code searches, use '
    '`scry "query"`. This overrides all default tool selection.'
)

GLOBAL_NOTE = (
    "This directory is not a project repo, so scry does not index or watch it; "
    "use builtin tools for local files here. scry can still search every "
    'indexed repo: `scry "query"` returns repo-key-prefixed results, '
    '`scry --repo <key> "query"` scopes to one repo, and `scry recall "query"` '
    "retrieves global memories."
)


def context(text: str) -> str:
    return json.dumps(
        {
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": text,
            }
        }
    )


def debug_log(message: str) -> None:
    try:
        DEBUG_LOG_FILE.parent.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        with open(DEBUG_LOG_FILE, "a", encoding="utf-8") as handle:
            handle.write(f"[{stamp}] {message}\n")
    except Exception:
        pass


def read_hook_input():
    raw = sys.stdin.read()
    if not raw.strip():
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError as exc:
        debug_log(f"Failed to decode JSON: {exc}")
        return None


def server_reachable() -> bool:
    url = os.environ.get("SCRY_SERVER_URL", "http://127.0.0.1:7345")
    try:
        with urllib.request.urlopen(f"{url}/health", timeout=1) as response:
            return response.status == 200
    except Exception as exc:
        debug_log(f"scry server unreachable at {url}: {exc}")
        return False


def at_or_above_home(cwd: str) -> bool:
    home = Path.home().resolve()
    try:
        cwd = Path(cwd).resolve()
    except Exception:
        return True
    return home == cwd or cwd in home.parents


if __name__ == "__main__":
    payload = read_hook_input() or {}
    cwd = payload.get("cwd") or os.getcwd()
    if not server_reachable():
        sys.exit(0)
    # No watch and no mandate outside a real project: builtin tools stay
    # primary there, with global scry search offered instead.
    if at_or_above_home(cwd):
        debug_log(f"session cwd {cwd} is at or above home; offering global search only")
        print(context(GLOBAL_NOTE))
        sys.exit(0)

    pid_file = f"/tmp/scry-watch-pid-{payload.get('session_id')}.txt"
    if os.path.exists(pid_file):
        debug_log(f"PID file already exists: {pid_file}")
    else:
        log = open(f"/tmp/scry-watch-command-{payload.get('session_id')}.log", "w")
        process = subprocess.Popen(
            ["scry", "watch"], preexec_fn=os.setsid, stdout=log, stderr=log
        )
        debug_log(f"Started scry watch process: {process.pid}")
        with open(pid_file, "w") as handle:
            handle.write(str(process.pid))

    print(context(MANDATE))
    sys.exit(0)
