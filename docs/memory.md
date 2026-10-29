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
never hard-delete; one helpful retrieval re-strengthens them. Recall
ranks scope and global memories together with no cap on globals; only the
brief caps them.

Noise votes suppress a memory per scope. Suppressions apply to scopes
that exist as a repo and to requests with no scope key at all; a scope
key that has no repo yet sees no suppressions.

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
capture: 12 memories (3 stale), 40 retrievals, 9 helpful, 2 noise, 5 archived
```

Retrievals sum each memory's access count; noise counts `scry memory
noise` votes; archived counts rows retired by `scry memory gc`. The Claude Code plugin injects the top-scoring memories on
each prompt and lets the agent report which ones helped.

## Lifecycle

### Scope partitioning

Memory vectors are partitioned by scope (globals share one partition), so
recall runs one nearest-neighbor search over the current scope and one
over the globals and merges them by distance. A large neighbor scope
cannot crowd out the current one.

### Suppression

`scry memory noise <id>` demotes the memory's salience and also hides it
in the scope the command ran in. Other scopes still see it.
`scry memory helpful <id>` lifts the hide in the scope it ran in.
Suppressed memories are skipped by recall and brief.

### Reaffirm

```sh
scry memory stale [--file relpath]...
scry memory reaffirm <id> --evidence "text"
```

`stale` lists stale, non-superseded memories in scope, one per line as
three tab-separated fields: `[kind #id] (stale)`, the anchored paths
joined by `,`, and the content. In content and paths, `\` is written as
`\\`, newline as `\n`, tab as `\t`, and carriage return as `\r`; a `,`
inside a path is written `\,`, so split paths on unescaped commas. Each `--file` narrows the list to memories anchored to it, and
nothing prints when there are none.
`reaffirm` re-hashes the memory's anchors to the files as indexed now,
marks it live, and appends the evidence. It fails unless the memory is
stale, not superseded, and every anchor is still indexed. The server
checks those preconditions only; deciding that the memory still holds is
the caller's job.

### GC and restore

```sh
scry memory gc [--dry-run]
scry memory restore <id>
```

`gc` archives memories that recall should stop surfacing: the text stays,
the vector is deleted, and recall, brief, and stale listings skip them.
Each row counts toward the first rule it matches:

- `superseded`: replaced by another memory.
- `stale`: stale for longer than `[memory] gc_stale_days` with no helpful
  vote since it went stale.
- `episodes`: an `episode` with no helpful votes whose recall score at
  similarity 1.0 is below `[memory] gc_episode_floor`.
- `noise`: no helpful votes, and suppressed in its own scope (a repo
  memory) or in at least two repo scopes (a global memory). A global
  suppressed in one scope is handled by suppression alone.

It prints a count per rule, then `archived N` (`would archive N` with
`--dry-run`).

`restore` re-embeds an archived memory and brings it back. It comes back
`stale` if it was archived stale or an anchored file changed while it was
archived, and `live` otherwise. Restoring clears its suppressions in
every scope and resets its recency, so the next gc does not archive it
again at once. Superseded memories cannot be restored.

```toml
[memory]
half_life_days = 29
gc_stale_days = 30
gc_episode_floor = 0.1
```

### Arguments

`--` ends option parsing for `scry remember`, `scry recall`, and
`scry memory verify`; the rest is the content, query, or claim, so
`scry remember -- "--no-touch skips counting"` keeps the leading dashes.

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
retrieval (`--no-touch`). Sessions with fewer than three tool calls in
the new range are skipped. `SCRY_CAPTURE_MODEL` picks the model
and `SCRY_CAPTURE_LOG` the one-line-per-pass log.

### Proof gating

The script, not the model, decides what counts as proven. Every claim
carries an entry number and a verbatim quote (20+ characters,
whitespace-normalized) that must appear in that entry of the rendered
transcript the model saw.

- `lesson` and `fact` need a quote from a tool result.
- `decision` and `convention` need a quote from a user prompt or a tool
  result.
- A tool result is provable only if it came from `Read`, `Grep`, `Glob`,
  `NotebookRead`, `LSP`, or a Bash command that uses no network client.
  Results from `Agent`/`Task`, `Skill`, `WebFetch`, `WebSearch`, and every
  `mcp__*` tool never count, nor does Bash running `curl` or `wget`
  against anything but loopback, any non-loopback `scheme://` URL,
  `scry -w`/`--web`, `gh api`, `http`/`https`, `git clone|fetch|pull`, or
  `pip`/`npm`/`cargo`/`uv install`. The same rule applies to reinforce,
  contradict, and reaffirm quotes; user prompts are unaffected.
- A memory that fails is stored as an `episode` with its anchors dropped,
  so a wrong guess never lands as a fact.
- Tool results that echo memory or the agent's own writing (edits, writes,
  `scry recall|remember|memory`) never count as proof.
- Anchors are limited to files the session edited.

A true quote does not prove the claim follows from it. Every memory that
survives the quote check as a `lesson`, `fact`, `decision`, or
`convention` is then judged by `scry memory verify`, which compares the
memory content against the full cited entry. `unsupported` downgrades it
to an `episode` (anchors and supersession dropped). If verification is not
configured, the quote-only result stands and the pass log records
`"verify": "not configured"`. A failed or timed-out verify call fails
closed and downgrades. Each pass logs the scores (`verified`) and the
`downgraded` count. See the Verification section for the model and
threshold settings.

### Compounding

A reinforce claim needs a provable tool-result quote and runs `scry memory helpful`;
a contradict claim needs a tool-result or user quote and runs
`scry memory noise`. Unproven claims are ignored, ids the model was not
shown (reinforce, contradict, supersedes) are dropped, and each id is
marked at most once per session. A new memory may name the one it
replaces, but only a `lesson`, `fact`, `decision`, or `convention` that
kept its kind through the proof check can retire it. At `SessionStart` the
plugin injects `scry memory brief`, so the next session starts from these
memories and marks them in turn.

### Reaffirming stale memories

When the session edited files in a repo, the worker runs `scry index`
once and then `scry memory stale --file <path>` for those repo-relative paths, so memories the edits just made stale are
included, and shows
the results to the model in a separate block, "stale memories anchored to
files this session edited". The model may return `reaffirm` claims for
those ids, each with a tool-result quote that shows the memory still
holds after the edit. A claim is applied only if the id was in that block,
the quote passes the same proof check as a `lesson` or `fact`, and
`scry memory verify` does not judge it `unsupported` (or verification is
not configured). It then runs `scry memory reaffirm <id> --evidence`
with the cited quote. The pass log lists `reaffirmed`, `reaffirm_failed`,
and `reaffirm_unsupported` (rejected by the verifier) ids. Replacing a
stale memory with a new one through `supersedes` works as before.

### Web content

Tool results from `WebFetch`, `WebSearch`, any tool whose name contains
`web` or `fetch`, and Bash commands running `scry -w`/`--web`, `curl`, or
`wget` are rendered as `[web content omitted]`, so the model never reads
them. The calls stay visible. `curl` and `wget` match only as a command
word (at the start or after `|`, `;`, `&&`, `||`, `$(`), and values of
`-o`, `-d`, `-H` and their long forms are never read as hosts. They render
normally only when every target and any `--proxy`/`-x` value is a
loopback address (`localhost`, `127.0.0.1`, `[::1]`, `0.0.0.0`). Other
subagent, MCP, and network-command results are not omitted but still never
count as proof (see Proof gating).

### Daily gc

At `SessionStart`, when the server is reachable and
`~/.cache/scry/gc-stamp` is missing or older than 24 hours, the plugin
spawns `scry memory gc` detached, with output going to the watch log
(`SCRY_WATCH_LOG`), and touches the stamp. It never blocks the hook. The
gc rules and their config are described in the lifecycle section of this
page.
