# Memory

Memories are typed notes about a codebase: `lesson`, `decision`,
`convention`, `skill`, `fact`, or `episode`. They live in the same store
as the search index and are shared across devices the same way.

```sh
scry remember "sqlite-vec partition keys must come before the vector column" \
  --kind lesson --pain 7 --anchor crates/scry-core/src/store/mod.rs
scry recall "sqlite-vec table definition order"
scry memory helpful 12    # reinforce; `noise` demotes instead
```

## Scopes

A memory is scoped to a repo key, or is global (`--global`). Global
memories are visible from every scope. Inside a repo the scope is the
normalized remote. In a directory at or above `$HOME` there is no repo,
so the scope is a path key: `path:~` for home itself, `path:~/notes` for
a folder under it, `path:/srv/data` elsewhere. Keys under home are
home-relative, so the same layout shares memories across devices. Path
scopes index nothing, so they cannot take `--anchor`. Search is not
affected: at or above home it still spans all indexed repos.

## Provenance

`--source agent|capture` records who wrote a memory (default `agent`;
`capture` is for automatic extraction) and `--evidence "text"` keeps what
it was derived from. Both are stored with the memory and `scry memory
stats` reports per source.

## Salience

Encoding strength is composed at write time from three valence-neutral
signals: surprise (embedding distance to the nearest existing memory, so
duplicates encode weakly), cost (`--cost`, what the insight took to earn;
`--pain` is the explicit override), and explicit importance. Nothing
privileges negative experiences; a good decision encodes as strongly as a
hard-won bug fix.

## Recall

Recall score multiplies semantic similarity by stored salience, a recency
decay (`[memory] half_life_days`, default 29), and learned utility
(helpful retrievals over total retrievals). Memories fade with disuse and
never hard-delete; one helpful retrieval re-strengthens them.

## Staleness

A memory anchored to a file (`--anchor path[:start-end]`) is tied to that
file's content hash. When a sync commits a different hash, the memory
flips to `stale`: still retrievable, demoted to half weight, and flagged
in output. This is the mechanical answer to "does this memory need
updating"; anchored memories cannot silently stay confidently wrong.

Scry records when a memory goes stale, so a brief can surface what
changed recently.

## Brief

```sh
scry memory brief [-m 8] [--stale-days 7]
```

Prints the top memories in scope without a query, ranked like recall with
similarity fixed at 1.0:

```
[lesson #12] (0.81) content
may be wrong now (anchored code changed):
[fact #40] (stale) content
```

The second block lists up to 5 memories that went stale within the last
`--stale-days`. Inside a repo or path scope at most 2 global memories
make the top list, so scope memories are not crowded out. Nothing prints
when both lists are empty. A brief does not count as a retrieval and never
refreshes recency; neither does `scry recall --no-touch`.

## Supersede

`scry memory supersede <old_id> <new_id>` retires the old memory in favor
of the new one, keeping the chain. It fails if either id is missing or
the old memory is already superseded. Superseded memories drop out of
recall and brief.

## Stats

`scry memory stats` prints one line per source over live and stale
memories:

```
capture: 12 memories (3 stale), 40 retrievals, 9 helpful, 2 noise
```

Retrievals sum each memory's access count; noise counts `scry memory
noise` votes. The Claude Code plugin injects the top-scoring memories on
each prompt and lets the agent report which ones helped.

## Verification

An optional `[verify]` section adds a claim verifier: `scry memory verify
"claim" --evidence "text"` prints `supported 0.83`, `unsupported 0.04`, or
`not configured`. The score is P(evidence supports claim); `supported` means
the score reaches `threshold`, which lives only in the server config. Without
`[verify]` nothing is verified and nothing falls back to `[chat]`. A model
or server error exits nonzero, as does a model that answers neither yes nor
no, so callers should treat failure as unsupported.

| Key | Default | Meaning |
|---|---|---|
| `api` | `chat` | `chat` or `systemone` |
| `base_url` | required | endpoint root ending in `/v1` for both modes |
| `model` | required | model name sent in the request |
| `api_key` | `ollama` | bearer token; `env:VAR` reads an environment variable |
| `threshold` | `0.5` | cut for `supported`; not tuned to any model |
| `prompt` | built in | chat prompt override; must contain `{claim}` and `{evidence}` |

`chat` asks an OpenAI-compatible model a yes/no question at
`{base_url}/chat/completions` with one output token and logprobs, and scores
P(yes) / (P(yes) + P(no)) over the top tokens. Servers that return no
logprobs are scored from the answer text (`yes` is 1.0, `no` is 0.0).
Thinking is always disabled for Qwen-style models. `systemone` posts
`{claim, evidence}` to `{base_url}/systemone` and reads the `noul`
answer as the score.

```toml
[verify]
base_url = "http://localhost:12434/v1"
model = "qwen3.6:35b-a3b"
threshold = 0.2
```

```toml
[verify]
api = "systemone"
base_url = "http://localhost:8017/v1"
model = "jevos-v4"
```

### Calibration

`scry memory verify --eval eval/verify-pairs.jsonl` scores labelled rows
(`{"claim", "evidence", "label": 1|0}`) through the configured verifier and
prints the AUROC, median latency, false accepts and false rejects at the
configured threshold, and the score above which no unsupported claim was
accepted. Pick a threshold from your own model and data.

`eval/verify-pairs.jsonl` holds 28 pairs from real sessions (14 supported).
On it, the llama-swap `qwen3.6:35b-a3b` example above measured AUROC 0.98,
with zero false accepts and two of 14 good claims rejected at 0.2 (six
rejected at 0.5). That is one measurement on a small set, not a default.
The same set scored `jevos-v4` through `systemone` at AUROC 0.45 to 0.77
depending on phrasing, so calibrate before trusting any verifier.

## Automatic capture

The Claude Code plugin captures memories without the agent asking. At
`PreCompact` and `SessionEnd` a detached worker renders the new part of
the session transcript (user prompts, assistant text, tool calls, and the
tail of each tool result, each numbered `[#n]`) and makes one headless
`claude -p` pass over it. The model sees the memories injected during the
session plus a `scry recall` on the first prompt, and returns up to five
memories plus memories to reinforce or contradict. A per-session state
file records how far the transcript was processed, so `PreCompact` then
`SessionEnd` covers each part once; concurrent passes for one session
serialize on a per-session lock. The recall it runs does not count as a
retrieval (`--no-touch`). Sessions with fewer than three tool
calls in the new range are skipped. `SCRY_CAPTURE_MODEL` picks the model
and `SCRY_CAPTURE_LOG` the one-line-per-pass log.

### Proof gating

The script, not the model, decides what counts as proven. Every claim
carries an entry number and a verbatim quote (20+ characters,
whitespace-normalized) that must appear in that entry of the rendered
transcript the model saw.

- `lesson` and `fact` need a quote from a tool result.
- `decision` and `convention` need a quote from a user prompt or a tool
  result.
- A memory that fails is stored as an `episode` with its anchors dropped,
  so a wrong guess never lands as a fact.
- Tool results that echo memory or the agent's own writing (edits, writes,
  `scry recall|remember|memory`) never count as proof.
- Anchors are limited to files the session edited.

### Compounding

A reinforce claim needs a tool-result quote and runs `scry memory helpful`;
a contradict claim needs a tool-result or user quote and runs
`scry memory noise`. Unproven claims are ignored, ids the model was not
shown (reinforce, contradict, supersedes) are dropped, and each id is
marked at most once per session. A new memory may name the one it replaces; the
supersession is applied only if the new memory kept its kind after the
proof check. At `SessionStart` the plugin injects `scry memory brief`, so
the next session starts from these memories and marks them in turn.
