# Changelog

## 0.2.0 - 2026-09-05

<!-- soothfast:notes -->
### Overview

Retrieval quality release. Tree-sitter chunks now keep the doc comment
and attributes above each definition, and spans under four lines merge
into a neighbour, so a bare `#[cfg]` line or a one-line `mod` declaration
never surfaces as a hit on its own; on finance-query that removed a fifth
of all chunks and lifted recall@10 on the golden set from 0.717 to 0.833.
Answers go through the same rerank as search and say what the sources do
not cover instead of guessing. The reranker reads at most `max_chars`
(3000) of each document, which is both faster and more accurate than full
length, and the server caches the last 256 query vectors so a repeated
query skips HyDE and embedding. Files marked `@generated` are no longer
indexed. A cross-encoder is still optional; nothing changes for a server
without `[rerank]`.

### Upgrade notes

Run `scry index --full` once per repo after upgrading the server. Files
are only re-chunked when their content changes, so an existing index
keeps the old short chunks until every file is re-uploaded; a plain
`scry index` reports everything unchanged. The full pass re-embeds the
repo, which took eight minutes for a 1,200-file repo on an iGPU. The new
`[rerank] max_chars` key defaults to 3000 and the `rerank` field on
`/v1/answer` defaults to true, so no config change is required.
<!-- /soothfast:notes -->

### ✨ Features

- Rerank answer sources and admit gaps
- Add scry index --full to re-chunk a repo
- Skip @generated files when chunking
- Keep trivia with defs and merge short chunks
- Rerank stage fused as a weighted RRF leg (#4)
- Greedy chat completions, HyDE capped at 120 tokens
- Eval --limit for recall at deeper cutoffs
- Golden eval sets and multi-run eval report
- Path-aware BM25 and prefix-tolerant expansion
- Memory write discipline in session context
- Soothfast changelog with release-notes wiring

### 🐛 Fixes

- hydrate_chunks errors on a missing id
- Embed timeout and batch size for busy endpoints
- Timeouts on every outbound HTTP client

### ⚡ Performance

- Cache query vectors on the server
- Cut reranker input to max_chars
- One allocation for the rescore id list, exact alloc claim
- Allocation-free exact scan and schema guards
- Float vectors in a plain table, coarse pass rescored from it
- Rescore coarse candidates in one KNN query

### 📝 Documentation

- Release notes for 0.2.0
- Allocation claim for the dense search path
- Add CLAUDE.md

### 🔧 Internal

- Bump version to 0.2.0
- Open a PR for CHANGELOG regeneration
- Tie-aware recall and recall@10 in refiner_curve
- Store pragmas, optimize on shutdown, vacuum on prune
- Store-layer benches and refiner recall harness

---

### 🔍 API surface

```
# scry-core
ADDED    scry_core::chunker::is_generated
ADDED    scry_core::config::RerankConfig
ADDED    scry_core::rerank
ADDED    scry_core::rerank::RerankClient
ADDED    scry_core::rerank::RerankResult
ADDED    scry_core::rerank::fuse
ADDED    scry_core::search::RRF_K
REMOVED  scry_core::chunker::line_window::chunk_lines
CHANGED  scry_core::Error (body)
CHANGED  scry_core::chunker::chunk_file (body)
CHANGED  scry_core::chunker::line_window::chunk (body)
CHANGED  scry_core::config::Config (body)
CHANGED  scry_core::error::Error (body)
CHANGED  scry_core::search::expand_symbols (body)
CHANGED  scry_core::search::fts_query (body)
CHANGED  scry_core::search::query_vector (body)

# scry-server
ADDED    scry_server::QueryCache
ADDED    scry_server::query_cache::QueryCache
CHANGED  scry_server::AppState (body)
CHANGED  scry_server::api::AnswerRequest (body)
CHANGED  scry_server::api::SearchRequest (body)
CHANGED  scry_server::serve (body)
```

### 📊 Gate movement

| item | metric | was | now | delta |
|---|---|---:|---:|---:|
| `scry_core::bench_expand_symbols` | instructions | 50170.0 | 27518.0 | -45.2% |
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
