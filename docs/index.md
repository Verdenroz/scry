# scry

Self-hosted semantic code search. One server holds a hybrid dense + BM25
index of your repos; any device with a checkout of the same remote searches
and syncs the same index. Embeddings come from any OpenAI-compatible
endpoint, so everything can run on your own hardware.

```
scry "how are chunks defined"          # semantic search in the current repo
scry "auth middleware" src/server      # scoped to a subdirectory
scry --web --answer "sqlite-vec quantization options"
scry watch                             # keep the index in sync while you work
scry serve                             # run the index server
```
