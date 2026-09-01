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

Supersession replaces editing: a newer memory takes over via
`superseded_by`, keeping the chain. The Claude Code plugin injects the
top-scoring memories on each prompt and lets the agent report which ones
helped.
