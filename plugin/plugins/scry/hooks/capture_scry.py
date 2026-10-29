import re
import subprocess

import capture_transcript

REMEMBERED = re.compile(r"remembered #(\d+)")


def run(args, cwd, timeout=60):
    return subprocess.run(
        ["scry", *args], cwd=cwd, capture_output=True, text=True, timeout=timeout
    )


def recall(query: str, cwd: str) -> dict:
    result = run(["recall", query, "-m", "8", "--no-touch"], cwd)
    return capture_transcript.parse_memories(result.stdout)


def remember(memory, cwd):
    args = [
        "remember",
        memory["content"],
        "--kind",
        memory["kind"],
        "--source",
        "capture",
    ]
    if memory["evidence"]:
        args += ["--evidence", memory["evidence"]]
    if memory["pain"] is not None:
        args += ["--pain", str(memory["pain"])]
    anchored = args + [x for a in memory["anchors"] for x in ("--anchor", a)]
    result = run(anchored, cwd)
    if result.returncode != 0 and memory["anchors"]:
        result = run(args, cwd)
    found = REMEMBERED.search(result.stdout)
    return int(found.group(1)) if result.returncode == 0 and found else None


def index(cwd: str) -> None:
    run(["index"], cwd, timeout=300)


def mark(verb: str, ident: int, cwd: str) -> None:
    run(["memory", verb, str(ident)], cwd)


def supersede(old: int, new: int, cwd: str) -> None:
    run(["memory", "supersede", str(old), str(new)], cwd)


def verify(claim: str, evidence: str, cwd: str):
    result = run(["memory", "verify", claim, "--evidence", evidence], cwd)
    if result.returncode != 0:
        raise RuntimeError(f"scry memory verify exited {result.returncode}")
    line = result.stdout.strip()
    if line == "not configured":
        return line, None
    verdict, score = line.split()
    return verdict, float(score)
