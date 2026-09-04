"""Offline rerank simulation from a dump run.

Usage: simulate_rerank.py <dump.jsonl> <eval-dir>

Dump format, one JSON line per query, written by a measurement build of
the search route with SCRY_RERANK_DUMP set:
  {"query": str,
   "candidates": [{"relpath", "start", "end", "symbol", "score"}, ...],
   "rerank": [{"index": int, "score": float}, ...]}
`candidates` is the pool search_with_vector returns to the route (after
weighted RRF, recency boost, and Jaccard dedup), in rank order, top_n
long; `rerank` holds the reranker's score per candidate index.

Fusion is computed exactly where the shipped leg sits, at the route level
after the pool exists: score[i] = 1/(k + pool_rank_i + 1)
+ w / (k + rerank_rank_i + 1), k = 60, then sort descending. The NL gate
applies the leg only to queries route_query marks natural language.
"""
import json, re, sys, tomllib
from pathlib import Path

dump_path, eval_dir = sys.argv[1], Path(sys.argv[2])
K = 60.0

def tokens(q): return [t for t in re.split(r'[^0-9A-Za-z_:]+', q) if len(t) >= 2]
def inner_upper(t): return t[:1].islower() and any(c.isupper() for c in t[1:])
def natural_language(q):
    ts = tokens(q); ident = any('_' in t or '::' in t or inner_upper(t) for t in ts)
    return not (len(ts) <= 3 or (ident and len(ts) <= 6))

def matches(cand, expectation):
    path, _, line = expectation.rpartition(':')
    if path and line.isdigit():
        return cand['relpath'] == path and cand['start'] <= int(line) <= cand['end']
    return cand['relpath'] == expectation

dumps = {}
for line in open(dump_path):
    d = json.loads(line); dumps.setdefault(d['query'], d)

def order_for(d, mode, weight):
    cands = d['candidates']; n = len(cands)
    if mode == 'fused':
        return list(range(n))
    rerank = sorted(d['rerank'], key=lambda r: -r['score'])
    if mode == 'replace':
        return [r['index'] for r in rerank]
    score = [1.0 / (K + i + 1) for i in range(n)]
    for rank, r in enumerate(rerank):
        score[r['index']] += weight / (K + rank + 1)
    return sorted(range(n), key=lambda i: -score[i])

def evaluate(cases, mode, weight=1.0, gate=False):
    hits, rr = 0, 0.0
    for case in cases:
        d = dumps.get(case['query'])
        if d is None:
            continue
        m = 'fused' if (gate and not natural_language(case['query'])) else mode
        order = order_for(d, m, weight)[:10]
        rank = next((i for i, idx in enumerate(order)
                     if any(matches(d['candidates'][idx], e) for e in case['expect'])), None)
        if rank is not None:
            hits += 1; rr += 1 / (rank + 1)
    n = len(cases)
    return f"{hits/n:.3f}/{rr/n:.3f}"

for set_name in ['scry', 'finance-query', 'soothfast']:
    cases = tomllib.load(open(eval_dir / f'{set_name}.toml', 'rb'))['case']
    covered = sum(c['query'] in dumps for c in cases)
    row = [f"{set_name:14} ({covered}/{len(cases)} dumped)",
           f"fused {evaluate(cases, 'fused')}",
           f"replace {evaluate(cases, 'replace')}"]
    for w in (1.0, 2.0, 3.0, 5.0):
        row.append(f"rrf w{w:g} {evaluate(cases, 'rrf', w)}")
    row.append(f"rrf w2 nl-gate {evaluate(cases, 'rrf', 2.0, gate=True)}")
    row.append(f"replace nl-gate {evaluate(cases, 'replace', gate=True)}")
    print('  '.join(row))
