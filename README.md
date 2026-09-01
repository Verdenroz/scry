# scry

Self-hosted semantic code search and code-anchored memory, shared across
your devices. One `scry serve` instance holds a hybrid dense + BM25 index
in a single SQLite file; every checkout of the same repo, on any machine,
syncs and searches the same index. No third-party service ever sees your
code.

```
scry "how are chunks defined"           # semantic search in the current repo
scry "auth middleware" src/server       # scoped to a subdirectory
scry --repo github.com/you/proj "query" # any indexed repo, from anywhere
scry -a "how does sync decide what to upload"   # cited answer from a local LLM
scry --web --answer "sqlite-vec quantization"   # web-grounded answer (Tavily)
scry watch                              # keep the index in sync while you work
scry remember "lesson learned" --kind lesson --pain 7 --anchor src/store.rs
scry recall "why did we pick sqlite"    # memories about this codebase
```

Outside a project (your home directory, say), `scry "query"` searches
every indexed repo and prefixes each hit with its repo key.

Docs: <https://verdenroz.github.io/scry/>

## How it works

- **Repo-keyed index.** Repos are identified by their normalized git
  remote (fallback: `.scry.toml` name, then directory name) and paths are
  stored repo-relative, so a second checkout of an indexed repo syncs with
  zero uploads. Sync is a manifest diff by xxh64 hash; only changed files
  travel, and chunks whose content is unchanged reuse their vectors.
- **Structure-aware chunks.** Tree-sitter definition-level chunks with
  symbol paths for 16 languages (Rust, Python, TS/JS, Go, Java, C, C++,
  C#, Kotlin, Ruby, PHP, Scala, Lua, Bash, ...), blank-line-snapped
  windows for everything else. Each chunk embeds with a
  `repo > path > symbol` context header.
- **Routed hybrid retrieval.** Dense KNN (sqlite-vec, cosine; binary
  coarse pass + float rescore past ~8k chunks) fused with FTS5 BM25 via
  weighted RRF. Keyword-shaped queries lean lexical, natural-language
  questions lean dense; the BM25 leg is expanded with fuzzy matches from
  the repo's symbol table. Recency boost and near-duplicate filtering on
  top; optional HyDE expansion through the chat model.
- **Memory.** Typed notes (`lesson`, `decision`, `convention`, `skill`,
  `fact`, `episode`) with write-time salience (surprise + cost + explicit
  weight), recency decay with reinforcement on helpful recalls, and
  anchors: a memory tied to a file goes stale the moment a sync commits a
  different content hash, so it can never stay confidently wrong.
- **Local models.** Embeddings come from any OpenAI-compatible
  `/v1/embeddings` endpoint (llama.cpp, llama-swap, Ollama); `--answer`
  uses any OpenAI-compatible chat endpoint; `--web` uses Tavily. Each is
  optional and reports cleanly when unconfigured.

## Quick start

```sh
cargo install --path crates/scry
mkdir -p ~/.config/scry && cp deploy/config.example.toml ~/.config/scry/config.toml
# edit: point [embedding] at your endpoint; add [chat]/[tavily] if wanted
printf 'TAVILY_API_KEY=...\n' > ~/.config/scry/env && chmod 600 ~/.config/scry/env

cp deploy/scry.service ~/.config/systemd/user/
systemctl --user enable --now scry
loginctl enable-linger $USER      # start at boot without a login

cd ~/projects/yourrepo && scry index
scry "where is the retry logic"
```

Secrets stay in `~/.config/scry/env` (loaded by the unit, referenced from
the config as `"env:VAR"`), so `config.toml` is safe to keep in dotfiles.

Other devices point `[client] server_url` (or `SCRY_SERVER_URL`) at the
machine running `scry serve`, set the shared bearer token, and run
`scry index` in their checkout; unchanged files upload nothing. See
`docs/hosting.md` for LAN, WireGuard/Tailscale, docker compose (bundled
llama.cpp embedder), and public-HTTPS setups.

## Claude Code plugin

```sh
claude plugin marketplace add /path/to/scry/plugin
claude plugin install scry@scry
```

Per session it spawns `scry watch` in the project (SessionStart), injects
relevant memories into each prompt (UserPromptSubmit), and cleans up on
exit. In a directory that is not a project (your home directory, for
example) it stands down entirely and Claude keeps its builtin tools.

## Commands

| Command | Purpose |
|---|---|
| `scry "query" [path]` | semantic search; `-m N`, `-c` content, `-w` web, `-a` answer |
| `scry --repo <key> "query"` | search one indexed repo from anywhere; no repo + no flag = all repos |
| `scry serve` | run the index server |
| `scry index` / `scry watch` | one-shot / continuous sync of the current repo |
| `scry status` | repos, files, chunks, memories on the server |
| `scry remember` / `recall` | write / retrieve memories (`--kind`, `--pain`, `--anchor`) |
| `scry memory helpful\|noise <id>` | reinforce or demote a memory |
| `scry repo prune <key> [--into <key>]` | drop an index; migrate or detach its memories |
| `scry eval cases.toml` | Recall@10 / MRR against a golden query set |

## Development

Hot paths carry [soothfast](https://github.com/Verdenroz/soothfast)
benchmarks; CI gates performance against the merge-base and checks doc
claims against fresh measurements. `cargo test --workspace` runs the
suite, `cargo soothfast measure -p scry-core --save-baseline dev` records
a local baseline, `scry eval` guards retrieval quality when touching the
ranking pipeline.
