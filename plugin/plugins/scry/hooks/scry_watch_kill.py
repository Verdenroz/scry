import json
import os
import signal
import sys
from datetime import datetime
from pathlib import Path

DEBUG_LOG_FILE = Path(os.environ.get("SCRY_WATCH_KILL_LOG", "/tmp/scry-watch-kill.log"))


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


if __name__ == "__main__":
    payload = read_hook_input() or {}
    pid_file = f"/tmp/scry-watch-pid-{payload.get('session_id')}.txt"
    if not os.path.exists(pid_file):
        debug_log(f"PID file not found: {pid_file}")
        sys.exit(0)
    pid = int(open(pid_file).read().strip())
    try:
        os.kill(pid, signal.SIGKILL)
        debug_log(f"Killed scry watch process: {pid}")
    except ProcessLookupError:
        debug_log(f"Process {pid} already exited")
    os.remove(pid_file)
    sys.exit(0)
