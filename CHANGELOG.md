# Changelog

## Unreleased (draft vs v0.1.0)

<!-- soothfast:notes -->
<!-- ### Overview -->
<!-- What this release means for someone using it. One paragraph. -->

<!-- ### Upgrade notes -->
<!-- What a consumer has to do. "Nothing" is a useful answer. -->
<!-- /soothfast:notes -->

### ✨ Features

- Path-aware BM25 and prefix-tolerant expansion
- Memory write discipline in session context
- Soothfast changelog with release-notes wiring

### 🐛 Fixes

- Embed timeout and batch size for busy endpoints
- Timeouts on every outbound HTTP client

### 📝 Documentation

- Add CLAUDE.md

---

### 🔍 API surface

```
# scry-core
CHANGED  scry_core::search::expand_symbols (body)
CHANGED  scry_core::search::fts_query (body)
```

### 📊 Gate movement

| item | metric | was | now | delta |
|---|---|---:|---:|---:|
| `scry_core::bench_expand_symbols` | instructions | 59001.0 | 29674.0 | -49.7% |
| `scry_core::bench_expand_symbols` | allocs | 88.0 | 55.0 | -37.5% |


## 0.1.0 - 2026-09-01

<!-- soothfast:notes -->
### Overview

First release: a self-hosted replacement for cloud semantic code search,
plus a code-anchored memory layer. One server holds a hybrid dense + BM25
index in a single SQLite file; repos are keyed by their git remote, so
every checkout on every device shares the same index and a second
checkout syncs with zero uploads. Embeddings, answers, and memories all
run against local OpenAI-compatible endpoints; the only optional cloud
call is Tavily for `--web`.

### Upgrade notes

Nothing: this is the first release. The database stamps its embedding
model and dimension at creation and refuses a mismatched config, so the
one thing to decide up front is the embedding model.
<!-- /soothfast:notes -->

### ✨ Features

- Routed hybrid retrieval: dense KNN (sqlite-vec, cosine) fused with FTS5
  BM25 by weighted RRF; keyword-shaped queries lean lexical, questions
  lean dense; symbol-table query expansion, recency boost, near-duplicate
  filtering, optional HyDE through the configured chat model
- Tree-sitter definition-level chunking with symbol paths for 16
  languages, blank-line-snapped windows elsewhere, contextual
  `repo > path > symbol` embedding headers
- Manifest-diff sync keyed by xxh64 with chunk-level vector reuse;
  `scry watch` resyncs on debounced filesystem events
- Memory layer: CoALA kinds, valence-neutral salience
  (surprise/cost/explicit), recency decay with reinforcement, and code
  anchors that flip a memory stale when its file's hash changes
- Cross-repo search outside any project with repo-key-prefixed results;
  `--repo <key>` scopes explicitly; `scry repo prune` retires an index
  and migrates or detaches its memories
- Cited answers (`-a`) via any OpenAI-compatible chat endpoint; web
  search and web-grounded answers (`-w`) via Tavily, key held server-side
- Claude Code plugin: session-scoped watch, per-prompt memory injection,
  and a full stand-down outside project repos
- axum server with bearer auth (constant-time compare), the store on a
  dedicated actor thread; deploy artifacts for bare metal (systemd +
  linger), docker compose with a bundled llama.cpp embedder, and Caddy

### 🐛 Fixes

- Disable thinking mode for local chat models by default: Qwen-style
  models spent the whole token budget on `reasoning_content` and returned
  empty answers; `[chat] thinking = true` re-enables it

### ⚡ Performance

- Binary-quantized coarse pass (Hamming) with float rescore past 8k
  chunks per scope; content-hash vector reuse makes edits re-embed only
  changed chunks; soothfast baselines gate hashing, chunking, and fusion
  hot paths in CI
