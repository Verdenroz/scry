import json
import os
import subprocess

MODEL = os.environ.get("SCRY_CAPTURE_MODEL", "claude-sonnet-5-5")
TIMEOUT = 600

EVIDENCE = {
    "type": "object",
    "properties": {"entry": {"type": "integer"}, "quote": {"type": "string"}},
    "required": ["entry", "quote"],
}
CLAIMS = {
    "type": "array",
    "items": {
        "type": "object",
        "properties": {"id": {"type": "integer"}, "evidence": EVIDENCE},
        "required": ["id", "evidence"],
    },
}
SCHEMA = {
    "type": "object",
    "properties": {
        "memories": {
            "type": "array",
            "maxItems": 5,
            "items": {
                "type": "object",
                "properties": {
                    "kind": {
                        "enum": ["lesson", "fact", "decision", "convention", "episode"]
                    },
                    "content": {"type": "string", "maxLength": 300},
                    "evidence": EVIDENCE,
                    "anchors": {"type": "array", "items": {"type": "string"}},
                    "pain": {"type": "integer", "minimum": 0, "maximum": 10},
                    "supersedes": {"type": ["integer", "null"]},
                },
                "required": [
                    "kind",
                    "content",
                    "evidence",
                    "anchors",
                    "pain",
                    "supersedes",
                ],
            },
        },
        "reinforce": CLAIMS,
        "contradict": CLAIMS,
        "reaffirm": CLAIMS,
    },
    "required": ["memories", "reinforce", "contradict", "reaffirm"],
}

INSTRUCTIONS = """\
You maintain a memory store for a coding agent. Below is a numbered transcript of
one working session, plus memories that already exist. Decide what is worth
remembering for future sessions on this codebase.

Rules:
- Write 0-2 memories typically. Zero is a good answer. Never narrate what happened;
  that is at most an episode.
- Each memory must stand alone, say why it matters, and fit in 300 characters.
- kind: lesson (a non-obvious cause or pitfall), fact (a verified property of the
  code or environment), decision (a choice and its reason), convention (a rule the
  user wants followed), episode (what happened, weakest).
- Every memory and every reinforce/contradict needs evidence: the entry number and a
  verbatim quote (20+ characters) copied exactly from that entry. lesson and fact
  must quote a tool result. decision and convention must quote a user prompt or a
  tool result. Claims that rest only on the assistant's own words are not proof.
- Do not state guesses as facts. If the transcript shows a theory that was never
  confirmed by a tool result, leave it out.
- anchors: repo-relative files the session edited that the memory is about, optionally
  as path:start-end. Use [] if unsure.
- pain: 0-10, how costly the insight was to earn.
- supersedes: the id of an existing memory this one replaces, else null.
- reinforce: existing memory ids that this session confirmed. contradict: ids this
  session showed to be wrong or misleading.
- reaffirm: ids from the stale block whose claim this session showed still holds after
  the edit, quoting a tool result as above. Leave out any you cannot show. To replace
  a stale memory instead, use supersedes.
"""


def render_context(existing) -> str:
    if not existing:
        return "(none)"
    return "\n".join(
        f"[{kind} #{ident}] {content}"
        for ident, (kind, content) in sorted(existing.items())
    )


def build_prompt(transcript: str, existing, stale=None) -> str:
    stale = stale or {}
    current = {i: m for i, m in existing.items() if i not in stale}
    return (
        f"{INSTRUCTIONS}\nExisting memories:\n{render_context(current)}\n\n"
        "Stale memories anchored to files this session edited:\n"
        f"{render_context(stale)}\n\n"
        f"Transcript:\n{transcript}\n"
    )


def call(prompt: str, workdir: str) -> dict:
    result = subprocess.run(
        [
            "claude",
            "-p",
            "--model",
            MODEL,
            "--tools",
            "",
            "--no-session-persistence",
            "--output-format",
            "json",
            "--json-schema",
            json.dumps(SCHEMA),
        ],
        input=prompt,
        cwd=workdir,
        env={**os.environ, "SCRY_CAPTURE": "1"},
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
    )
    envelope = json.loads(result.stdout)
    if envelope.get("is_error") or "structured_output" not in envelope:
        raise RuntimeError(f"claude -p failed: {envelope.get('subtype')}")
    return envelope["structured_output"]
