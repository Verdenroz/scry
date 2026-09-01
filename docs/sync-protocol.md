# Sync protocol

Repos are keyed by their normalized git remote URL (fallback: `.scry.toml`
`[project] name`, then directory basename), and every path is stored
relative to the repo root. Any checkout of the same remote, on any device,
addresses the same index rows.

A sync is a manifest diff:

1. Client walks the repo (`.gitignore`, `.scryignore`, hidden files, and
   default binary patterns excluded; empty and oversized files skipped)
   and hashes each file with xxh64.
2. `POST /v1/manifest` returns the server's `{relpath, xxh64}` list; only
   files whose hash differs are uploaded, in batches, and files missing
   locally are deleted server-side.
3. The server chunks and embeds each upload. Chunks carry a content hash;
   a chunk whose hash already has a vector anywhere in the store reuses it,
   so edits re-embed only what actually changed.
4. Committing a file flips any memory anchored to it with a different
   hash to `stale`.

`scry watch` runs the same diff on debounced filesystem events; a second
checkout of an already-indexed repo syncs with zero uploads.
