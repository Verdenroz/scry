import re
import subprocess

import capture_transcript

REMEMBERED = re.compile(r"remembered #(\d+)")
STALE = re.compile(r"^\[(\w+) #(\d+)\] \(stale\)\t[^\t\n]*\t(.*)$", re.MULTILINE)
ESCAPE = re.compile(r"\\([\\ntr,])")
UNESCAPED = {"n": "\n", "t": "\t", "r": "\r"}
PATH = re.compile(r"(?:\\.|[^\\,])+")


def run(args, cwd, timeout=60):
    return subprocess.run(
        ["scry", *args], cwd=cwd, capture_output=True, text=True, timeout=timeout
    )


def recall(query: str, cwd: str) -> dict:
    result = run(["recall", "-m", "8", "--no-touch", "--", query], cwd)
    return capture_transcript.parse_memories(result.stdout)


def remember(memory, cwd):
    args = ["--kind", memory["kind"], "--source", "capture"]
    if memory["evidence"]:
        args += ["--evidence", memory["evidence"]]
    if memory["pain"] is not None:
        args += ["--pain", str(memory["pain"])]
    anchored = args + [x for a in memory["anchors"] for x in ("--anchor", a)]
    result = run(["remember", *anchored, "--", memory["content"]], cwd)
    if result.returncode != 0 and memory["anchors"]:
        result = run(["remember", *args, "--", memory["content"]], cwd)
    found = REMEMBERED.search(result.stdout)
    return int(found.group(1)) if result.returncode == 0 and found else None


def unescape(text: str) -> str:
    return ESCAPE.sub(lambda m: UNESCAPED.get(m.group(1), m.group(1)), text)


def split_paths(field: str) -> list:
    return [unescape(path) for path in PATH.findall(field)]


def stale(files, cwd: str) -> dict:
    args = ["memory", "stale"] + [x for f in files for x in ("--file", f)]
    result = run(args, cwd)
    if result.returncode != 0:
        raise RuntimeError(f"scry memory stale exited {result.returncode}")
    return {
        int(ident): (kind, unescape(content))
        for kind, ident, content in STALE.findall(result.stdout)
    }


def reaffirm(ident: int, evidence: str, cwd: str) -> bool:
    result = run(["memory", "reaffirm", str(ident), "--evidence", evidence], cwd)
    return result.returncode == 0


def index(cwd: str):
    try:
        result = run(["index"], cwd, timeout=300)
    except subprocess.TimeoutExpired:
        return "scry index timed out"
    if result.returncode != 0:
        return f"scry index exited {result.returncode}"
    return None


def mark(verb: str, ident: int, cwd: str) -> None:
    run(["memory", verb, str(ident)], cwd)


def supersede(old: int, new: int, cwd: str) -> None:
    run(["memory", "supersede", str(old), str(new)], cwd)


def verify(claim: str, evidence: str, cwd: str):
    result = run(["memory", "verify", "--evidence", evidence, "--", claim], cwd)
    if result.returncode != 0:
        raise RuntimeError(f"scry memory verify exited {result.returncode}")
    line = result.stdout.strip()
    if line == "not configured":
        return line, None
    verdict, score = line.split()
    return verdict, float(score)
