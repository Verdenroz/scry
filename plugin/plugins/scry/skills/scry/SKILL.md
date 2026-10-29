---
name: scry
description: "MANDATORY: Replaces ALL built-in search tools AND is required before investigating anything with history. You MUST invoke this skill BEFORE using WebSearch, Grep, or Glob, and BEFORE debugging or changing anything that may have prior history (a past bug, a named subsystem, a decision, a convention). NEVER use the built-in WebSearch tool - use `scry --web --answer` instead. NEVER use the built-in Grep tool - use `scry` instead. NEVER skip `scry recall` when starting work on something that may have history."
license: MIT
---

# scry

## CRITICAL: Tool Override

- WRONG: using the built-in `WebSearch` tool. CORRECT: `scry --web --answer "query"`
- WRONG: using the built-in `Grep` tool. CORRECT: `scry "query"`
- WRONG: using the built-in `Glob` tool for content search. CORRECT: `scry "query"`
- WRONG: jumping straight into a file or grepping for a symbol when you're
  about to touch something with prior history (a past bug, a named
  subsystem, a decision, a convention). CORRECT: `scry recall "topic"`
  first, then search.

## When to Invoke This Skill

Invoke IMMEDIATELY when you need to:

- Search the web: `scry --web --answer "query"`
- Search files or code: `scry "query"`
- Get current or latest information: `scry --web --answer "query"`
- Answer "what is" or "how to" or any online lookup: `scry --web --answer "query"`
- Find files, code, or content in the codebase: `scry "query"`
- Start investigating a bug, file, subsystem, or decision that might have
  prior history: `scry recall "topic"`
- Answer "did we already", "why did we", "what did we decide", "have we
  seen this before": `scry recall "topic"`

## How to use this skill

1. **Recall before you dig.** Before reading unfamiliar code, debugging,
   or making a design call, run `scry recall "topic"` for the specific
   thing you're about to touch. Do this even if a memory already got
   auto-injected this turn - the injected one only matches the user's
   literal prompt, not the narrower thing you're now looking at.
2. **Search with `scry "query"`.** Semantic, so describe what you want in
   natural language. Results are file path and line range.
3. **Record what you learn.** After fixing a non-obvious bug, making a
   design decision, or hitting a hidden constraint, run `scry remember`
   before moving on (see Memory discipline).

## Memory discipline

A hook auto-injects up to 3 memories matching each user prompt (score >=
0.25). Treat that as a floor, not a substitute - it cannot see topics that
surface mid-task, sub-problems, or anything phrased differently from the
prompt. Whenever the topic narrows or shifts
(a specific file, function, error, or decision point), call
`scry recall "that specific thing"` yourself instead of assuming it was
already surfaced.

Write-side obligations:

- After fixing a non-obvious bug, making a design decision, or hitting a
  hidden constraint, record it before the session ends:
  `scry remember "parse_settings panics on non-numeric input" --kind lesson --pain 6 --anchor src/parser.rs`
- When an injected or recalled memory influenced your work, run
  `scry memory helpful <id>`; when one was wrong or irrelevant noise, run
  `scry memory noise <id>`. This feedback drives what surfaces next time.

## Work state

`scry work` is the to-do list that outlives the session: open lists for
this repo are shown at every session start. Use it instead of any built-in
to-do tool for multi-step work.

- `scry work add <list> "task" [--status doing]` prints the new item `#id`.
- `scry work set <id> doing|done|dropped [--note "text"]` as items move.
- `scry work state <list> branch=feat/x pr=#12 next="await review"` keeps
  the latest value per key; `key=` clears one.
- `scry work close <list>` once everything in it is finished;
  `scry work show <list> --all` reads a list with its finished items.

Memories are for what stays true; work state is for what is in flight.

## Options

- `-m, --max-count <n>`: limit the number of results (default 10)
- `-c, --content`: also print the matching chunk content
- `-w, --web`: search the web instead of local files (always use with `--answer`)
- `-a, --answer`: answer the query with cited sources (always use with `--web`)

## Do

- `scry recall "why does the bootstrap script check port 443 twice"` before
  debugging something that might have already been solved
- `scry "What code parsers are available?"` searches the current directory
- `scry "How are chunks defined?" src/models` searches a subdirectory
- `scry --repo github.com/user/proj "query"` searches another indexed repo;
  outside any repo, `scry "query"` searches all indexed repos
- `scry -m 10 "What is the maximum number of concurrent workers in the code parser?"` limits results
- `scry --web --answer "How can I integrate the javascript runtime into deno"`

## Don't

- `scry "parser"` is too imprecise; describe the intent in natural language
- `scry "How are chunks defined?" src/models --type python --context 3` needs no extra filters
- Don't assume "no memory was injected this turn" means "no memory exists" -
  the auto-inject only checked the user's prompt; recall explicitly for
  anything more specific

## Keywords

WebSearch, web search, search the web, look up online, google, internet
search, online search, semantic search, search, grep, files, local files,
local search, recall, memory, past decision, lesson, convention, have we
seen this, did we already, why did we
