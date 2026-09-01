# scry

Self-hosted semantic code search, shared across devices.

One `scry serve` instance holds a hybrid dense + BM25 index in a single
SQLite file. Repos are keyed by their git remote, so every checkout of the
same project, on any machine, syncs and searches the same index. Embeddings
come from any OpenAI-compatible endpoint (local llama.cpp by default), web
search from Tavily, answers from any OpenAI-compatible chat model. No
third-party index ever sees your code.

```
scry "how are chunks defined"          # semantic search in the current repo
scry "auth middleware" src/server      # scoped to a subdirectory
scry --web --answer "sqlite-vec quantization options"
scry watch                             # keep the index in sync while you work
scry serve                             # run the index server
```

Docs live in `docs/`. Performance is measured and gated with
[soothfast](https://github.com/Verdenroz/soothfast).
