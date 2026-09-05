# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo test --workspace                       # full suite (no network; tests use HashEmbedder)
cargo test -p scry-core memory               # one integration test file
cargo test -p scry-core repo::tests          # one unit test module
cargo clippy --workspace --all-targets -- -D warnings   # CI treats warnings as errors
cargo fmt --all

cargo soothfast measure -p scry-core --save-baseline base   # refresh perf baseline
cargo soothfast gate -p scry-core --against-ref origin/main # what gate.yml runs on PRs
cargo soothfast docs check                   # doc claims vs the "base" baseline
cargo soothfast docs build --baseline base   # site to ./site (published by docs.yml)

cargo install --path crates/scry             # the one binary: CLI + `scry serve`
scry eval cases.toml                         # Recall@10 / MRR when touching ranking
```

Live end-to-end needs an OpenAI-compatible embedding endpoint (default
`http://localhost:12434/v1`, model `harrier-oss:0.6b`, 1024-dim); unit and
integration tests never do.

## Architecture

Three crates: `scry-core` (all engine logic), `scry-server` (axum handlers
as a library), `scry` (single binary dispatching CLI subcommands and
`serve`). The wire types both server and CLI use live in
`scry-server/src/api.rs`.

**The async/sync split is the load-bearing convention.** rusqlite's
`Connection` is `!Sync`, so `Store` never crosses an `.await`. Every
pipeline is split into an async half (network: embedding, chat) and a sync
half (all store access): `query_vector` / `search_with_vector`,
`remember` / `remember_with_embedding`, `prepare_file` / `known_vectors` /
`commit_file`. The server owns the store on a dedicated thread
(`store_actor.rs`); handlers send closures via `state.store.call(...)`.
Adding a route that mixes an await into store access will fail the
`Handler` bound with an opaque `!Send` error - split it instead.

**Identity and sharing.** A repo is keyed by its normalized git remote
(`repo::normalize_remote_url`; fallback `.scry.toml` `[project] name`,
then dir basename) and all paths are repo-relative, which is what lets
checkouts on different machines share index rows. The embedding input for
a chunk is `repo_key > relpath > symbol\n content` and the chunk's
`content_hash` is the hash of that whole input - so vector reuse survives
edits elsewhere in a file, but renaming a file or changing the repo key
deliberately re-embeds. Directories at or above `$HOME` are never treated
as repos; search there goes cross-repo (`repo_id = None` end to end).

**Chunking happens server-side** in `prepare_file`, and a file is only
re-chunked when its content hash changes. After touching `chunker/`, run
`scry index --full` in a test repo to see the effect; plain `scry index`
will report everything unchanged. `@generated` files yield no chunks.

**One SQLite file** holds everything: repos/files/chunks + FTS5 +
sqlite-vec tables + memories. The vec tables are created at `Store::open`
because their DDL carries the embedding dimension; `meta` stamps the
embedding model/dim and `open` refuses a mismatch. sqlite-vec KNN cannot
express an optional partition filter, so `dense_search_*` build two SQL
variants. Past `BINARY_COARSE_THRESHOLD` chunks, dense search runs a
bit-vector Hamming pass then rescores floats in Rust.

**Search pipeline** (`search.rs`): query routing by shape (keyword-ish
leans BM25, natural language leans dense), symbol-table expansion into the
FTS query, weighted RRF (k=60), recency boost, greedy Jaccard dedup.
HyDE runs only when a `[chat]` endpoint is configured (mode `[search]
hyde`). Display score is dense cosine similarity when known, a decayed
floor otherwise.

**Memory** (`memory.rs`, `store/memories.rs`): salience is composed at
write (surprise = distance to nearest existing memory, cost, explicit
`--pain`); recall multiplies similarity, salience, recency decay
(`[memory] half_life_days`), learned utility, and a 0.5 stale factor.
Staleness is mechanical: `commit_file` flips any memory whose anchor hash
no longer matches - both sync paths share that one function, keep it that
way.

**mgrep parity is a contract.** The CLI's search path must keep tolerant
parsing (unknown flags skipped, excess positionals ignored - agents invent
flags) and the `./path:start-end (NN.NN% match)` output format; the
Claude Code plugin's skill and hooks depend on both.

**Plugin releases are version-pinned.** Editing `plugin/` does nothing for
installed copies until the version is bumped in BOTH
`plugin/.claude-plugin/marketplace.json` and
`plugin/plugins/scry/.claude-plugin/plugin.json`, then
`claude plugin update scry@scry`.

**soothfast is wired into CI.** Benches live in
`crates/scry-core/benches/soothfast.rs`; `docs/search.md` carries claims
checked against the `base` baseline (alloc claims are exact - changing a
measured function usually means re-measuring and updating the claim);
`gate.yml` measures PRs against their merge-base, so perf regressions in
measured functions fail CI by design, not by flake.

## Config

Layered: explicit path > `$SCRY_CONFIG` > `~/.config/scry/config.toml` >
defaults, then env overrides (`SCRY_LISTEN`, `SCRY_DB_PATH`,
`SCRY_SERVER_URL`, `SCRY_TOKEN`, `TAVILY_API_KEY`). Secret-bearing keys
accept `"env:VAR"` indirection; the systemd unit loads
`~/.config/scry/env` so the config file stays dotfiles-safe. `[chat]` and
`[tavily]` are optional and their features report "not configured"
cleanly - preserve that when adding config-gated features.
