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
never hard-delete; one helpful retrieval re-strengthens them.

## Staleness

A memory anchored to a file (`--anchor path[:start-end]`) is tied to that
file's content hash. When a sync commits a different hash, the memory
flips to `stale`: still retrievable, demoted to half weight, and flagged
in output. This is the mechanical answer to "does this memory need
updating"; anchored memories cannot silently stay confidently wrong.

The moment a memory goes stale is recorded, so a brief can surface what
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
`--stale-days`. Nothing prints when both lists are empty. A brief does
not count as a retrieval and never refreshes recency; neither does
`scry recall --no-touch`.

## Supersede

`scry memory supersede <old_id> <new_id>` retires the old memory in favor
of the new one, keeping the chain. It fails if either id is missing or
the old memory is already superseded. Superseded memories drop out of
recall and brief.

## Stats

`scry memory stats` prints one line per source over live and stale
memories:

```
capture: 12 memories (3 stale), 40 retrievals, 9 helpful, 2 noise
```

Retrievals sum each memory's access count; noise counts `scry memory
noise` votes. The Claude Code plugin injects the top-scoring memories on
each prompt and lets the agent report which ones helped.
