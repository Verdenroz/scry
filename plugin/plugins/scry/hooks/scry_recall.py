import json
import os
import subprocess
import sys


def read_hook_input():
    raw = sys.stdin.read()
    if not raw.strip():
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return None


if __name__ == "__main__":
    payload = read_hook_input() or {}
    prompt = (payload.get("prompt") or "").strip()
    if len(prompt) < 12:
        sys.exit(0)

    try:
        result = subprocess.run(
            ["scry", "recall", prompt, "-m", "3", "--min-score", "0.25"],
            cwd=payload.get("cwd") or os.getcwd(),
            capture_output=True,
            text=True,
            timeout=5,
        )
    except Exception:
        sys.exit(0)
    memories = result.stdout.strip()
    if result.returncode != 0 or not memories:
        sys.exit(0)

    print(
        json.dumps(
            {
                "hookSpecificOutput": {
                    "hookEventName": "UserPromptSubmit",
                    "additionalContext": (
                        "Memories from past work on this codebase (mark useful ones "
                        "with `scry memory helpful <id>`, wrong ones with "
                        "`scry memory noise <id>`):\n" + memories
                    ),
                }
            }
        )
    )
    sys.exit(0)
