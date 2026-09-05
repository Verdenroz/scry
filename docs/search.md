# Search

A query runs both retrieval legs and fuses them:

- Dense: the query (optionally HyDE-expanded through the `[chat]` model)
  is embedded and matched against chunk vectors with cosine distance.
  Repos past ~8k chunks get a binary-quantized coarse pass first, then a
  float rescore of the survivors.
- Lexical: FTS5 BM25 over chunk content, symbols, and file paths, with
  query tokens expanded by fuzzy matches against the repo's tree-sitter
  symbol table.

Routing weights the fusion by query shape: short keyword or identifier
queries lean on BM25, natural-language questions lean on the dense leg.
Fused candidates get a small recency boost for recently edited files and
a greedy near-duplicate filter before the final ranking.

With a `[rerank]` endpoint configured, the fused top `top_n` candidates
are scored by a cross-encoder and its ranking joins the fusion as a
third reciprocal-rank leg with `weight` (k=60, default 1.0). The
reranker changes order only; the displayed score stays the dense cosine.
A reranker that errors or takes longer than six seconds is skipped for
that query, and `--no-rerank` skips it per query.

Chunks are function-level where a tree-sitter grammar exists (16
languages), blank-line-snapped windows elsewhere, and each chunk is
embedded with a `repo > path > symbol` header for context. Comments and
attributes directly above a definition stay with it, and spans under four
lines merge into a neighbour, so a bare `#[cfg]` line or a one-line `mod`
declaration never becomes its own hit.

Measured hot paths are gated in CI by soothfast:

<!-- soothfast:claim scry_core::bench_hash_bytes.alloc.allocs <= 0 -->
Hashing file content for the sync diff allocates nothing.
<!-- /soothfast:claim -->

<!-- soothfast:claim scry_core::bench_normalize_remote_url.alloc.allocs <= 5 -->
Deriving a repo key from a remote URL costs at most five allocations.
<!-- /soothfast:claim -->

<!-- soothfast:claim scry_core::bench_dense_search.alloc.allocs <= 11 -->
A dense query over 16k chunks allocates at most eleven times in total:
neither the exact scan nor the coarse rescore allocates per row.
<!-- /soothfast:claim -->

Retrieval quality is measured with `scry eval <cases.toml>`: a golden set
of queries with expected `path` or `path:line` answers, reported as
Recall@10 and MRR. Run it before and after touching anything in the
ranking pipeline.
