//! Re-verification of stale memories: the anchored files' current chunks are
//! scored as evidence for the memory's claim, and a pass makes it live again.

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::memory::reaffirm;
use crate::store::{ChunkRow, MemoryAnchor, MemoryRow, Store};
use crate::verify::ClaimScorer;

const EVIDENCE_CHARS: usize = 6000;
const UNRANGED_CHUNKS: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverifyReport {
    pub checked: usize,
    pub reaffirmed: usize,
    pub still_stale: usize,
    pub skipped: usize,
    pub errors: usize,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: i64,
    pub claim: String,
    pub evidence: String,
    pub cited: String,
}

#[derive(Debug, Default)]
pub struct Prepared {
    pub candidates: Vec<Candidate>,
    pub skipped: usize,
}

/// Builds evidence for up to `limit` stale memories, oldest first. Memories
/// whose anchored files have no indexed chunks are skipped and left stale.
pub fn prepare(store: &Store, limit: usize) -> Result<Prepared> {
    let mut prepared = Prepared::default();
    for (row, anchors) in store.reverifiable_memories()? {
        if prepared.candidates.len() == limit {
            break;
        }
        match candidate(store, &row, &anchors)? {
            Some(candidate) => prepared.candidates.push(candidate),
            None => prepared.skipped += 1,
        }
    }
    Ok(prepared)
}

fn candidate(
    store: &Store,
    row: &MemoryRow,
    anchors: &[MemoryAnchor],
) -> Result<Option<Candidate>> {
    let Some(repo_id) = row.repo_id else {
        return Ok(None);
    };
    let embedding = store.memory_embedding(row.id)?;
    let mut chunks = Vec::new();
    for anchor in anchors {
        let found = anchor_chunks(store, repo_id, anchor, &embedding)?;
        if found.is_empty() {
            return Ok(None);
        }
        chunks.extend(found);
    }
    let (evidence, cited) = render(&chunks);
    Ok(Some(Candidate {
        id: row.id,
        claim: row.content.clone(),
        evidence,
        cited,
    }))
}

fn anchor_chunks(
    store: &Store,
    repo_id: i64,
    anchor: &MemoryAnchor,
    embedding: &[f32],
) -> Result<Vec<ChunkRow>> {
    match (anchor.start_line, anchor.end_line) {
        (Some(start), Some(end)) => store.chunks_overlapping(repo_id, &anchor.relpath, start, end),
        _ => store.nearest_file_chunks(repo_id, &anchor.relpath, embedding, UNRANGED_CHUNKS),
    }
}

fn render(chunks: &[ChunkRow]) -> (String, String) {
    let mut evidence = String::new();
    let mut cited = Vec::new();
    for chunk in chunks {
        if evidence.len() >= EVIDENCE_CHARS {
            break;
        }
        let label = format!("{}:{}-{}", chunk.relpath, chunk.start_line, chunk.end_line);
        if !evidence.is_empty() {
            evidence.push_str("\n\n");
        }
        evidence.push_str(&label);
        evidence.push('\n');
        evidence.push_str(&chunk.content);
        cited.push(label);
    }
    evidence.truncate(evidence.floor_char_boundary(EVIDENCE_CHARS));
    (evidence, cited.join(", "))
}

/// Scores candidates one at a time; concurrent requests perturb a local
/// model server. A scorer error is kept beside its candidate.
pub async fn score(
    scorer: &dyn ClaimScorer,
    candidates: Vec<Candidate>,
) -> Vec<(Candidate, Result<f64>)> {
    let mut scored = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let result = scorer.score(&candidate.claim, &candidate.evidence).await;
        if let Err(error) = &result {
            tracing::warn!(id = candidate.id, %error, "reverify scoring failed");
        }
        scored.push((candidate, result));
    }
    scored
}

/// Reaffirms memories scoring at or above `threshold`. With `dry_run` it only
/// counts.
pub fn commit(
    store: &mut Store,
    scored: Vec<(Candidate, Result<f64>)>,
    skipped: usize,
    threshold: f64,
    dry_run: bool,
) -> Result<ReverifyReport> {
    let mut report = ReverifyReport {
        skipped,
        ..ReverifyReport::default()
    };
    for (candidate, result) in scored {
        report.checked += 1;
        match result {
            Err(_) => report.errors += 1,
            Ok(p) if p >= threshold => {
                if !dry_run {
                    let evidence = format!("reverified against {} (p={p:.2})", candidate.cited);
                    reaffirm(store, candidate.id, &evidence, &[])?;
                }
                report.reaffirmed += 1;
            }
            Ok(_) => report.still_stale += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::store::{NewChunk, NewMemory, Scope};
    use async_trait::async_trait;

    const DIM: usize = 8;

    struct Stub<F>(F);

    #[async_trait]
    impl<F: Fn(&str) -> Result<f64> + Send + Sync> ClaimScorer for Stub<F> {
        async fn score(&self, claim: &str, _evidence: &str) -> Result<f64> {
            (self.0)(claim)
        }
    }

    fn padded(head: [f32; 4]) -> Vec<f32> {
        let mut vector = head.to_vec();
        vector.resize(DIM, 0.0);
        vector
    }

    fn chunk(start: u32, embedding: [f32; 4]) -> NewChunk {
        NewChunk {
            start_line: start,
            end_line: start + 9,
            symbol: None,
            content: format!("body at {start}"),
            content_hash: format!("h{start}"),
            embedding: padded(embedding),
        }
    }

    fn store_with_file() -> (Store, i64) {
        let mut store = Store::open_in_memory("m", DIM).unwrap();
        let repo = store.upsert_repo("github.com/x/y").unwrap();
        let file = store.upsert_file(repo, "a.rs", "new", 1, 0).unwrap();
        let chunks = [
            chunk(1, [1.0, 0.0, 0.0, 0.0]),
            chunk(11, [0.0, 1.0, 0.0, 0.0]),
            chunk(21, [0.0, 0.0, 1.0, 0.0]),
            chunk(31, [0.0, 0.0, 0.0, 1.0]),
            chunk(41, [1.0, 1.0, 0.0, 0.0]),
        ];
        store.replace_file_chunks(file, repo, &chunks).unwrap();
        (store, repo)
    }

    fn stale_memory(
        store: &mut Store,
        repo: i64,
        content: &str,
        relpath: &str,
        range: Option<(u32, u32)>,
    ) -> i64 {
        let memory = NewMemory {
            repo_id: Some(repo),
            kind: "lesson".to_string(),
            content: content.to_string(),
            salience: 0.5,
            surprise: 0.0,
            cost: 0.0,
            explicit_weight: 0.0,
            source: "agent".to_string(),
            evidence: None,
            embedding: padded([0.0, 1.0, 0.5, 0.0]),
        };
        let anchor = MemoryAnchor {
            relpath: relpath.to_string(),
            start_line: range.map(|r| r.0),
            end_line: range.map(|r| r.1),
            xxh64: "old".to_string(),
        };
        let id = store.add_memory(&memory, &[anchor]).unwrap();
        store.mark_stale_anchors(repo, relpath).unwrap();
        id
    }

    fn status(store: &Store, id: i64) -> String {
        store.memory(id).unwrap().unwrap().status
    }

    #[test]
    fn ranged_anchor_picks_overlapping_chunks() {
        let (mut store, repo) = store_with_file();
        stale_memory(&mut store, repo, "claim", "a.rs", Some((15, 25)));
        let prepared = prepare(&store, 10).unwrap();
        assert_eq!(prepared.candidates[0].cited, "a.rs:11-20, a.rs:21-30");
        assert!(
            prepared.candidates[0]
                .evidence
                .starts_with("a.rs:11-20\nbody at 11")
        );
    }

    #[test]
    fn unranged_anchor_picks_nearest_chunks() {
        let (mut store, repo) = store_with_file();
        stale_memory(&mut store, repo, "claim", "a.rs", None);
        let prepared = prepare(&store, 10).unwrap();
        assert_eq!(
            prepared.candidates[0].cited,
            "a.rs:11-20, a.rs:41-50, a.rs:21-30"
        );
    }

    #[test]
    fn evidence_is_capped() {
        let (mut store, repo) = store_with_file();
        let file = store.upsert_file(repo, "big.rs", "new", 1, 0).unwrap();
        let mut big = chunk(1, [0.0, 1.0, 0.0, 0.0]);
        big.content = "x".repeat(EVIDENCE_CHARS * 2);
        store.replace_file_chunks(file, repo, &[big]).unwrap();
        stale_memory(&mut store, repo, "claim", "big.rs", None);
        let prepared = prepare(&store, 10).unwrap();
        assert_eq!(prepared.candidates[0].evidence.len(), EVIDENCE_CHARS);
    }

    #[tokio::test]
    async fn pass_makes_the_memory_live_with_a_refreshed_hash() {
        let (mut store, repo) = store_with_file();
        let id = stale_memory(&mut store, repo, "claim", "a.rs", None);
        let prepared = prepare(&store, 10).unwrap();
        let scored = score(&Stub(|_: &str| Ok(0.93)), prepared.candidates).await;
        let report = commit(&mut store, scored, prepared.skipped, 0.5, false).unwrap();
        assert_eq!(
            report,
            ReverifyReport {
                checked: 1,
                reaffirmed: 1,
                ..ReverifyReport::default()
            }
        );
        assert_eq!(status(&store, id), "live");
        assert_eq!(store.mark_stale_anchors(repo, "a.rs").unwrap(), 0);
        assert!(store.reverifiable_memories().unwrap().is_empty());
        assert_eq!(store.live_memories(Scope::Repo(repo)).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fail_leaves_the_memory_stale() {
        let (mut store, repo) = store_with_file();
        let id = stale_memory(&mut store, repo, "claim", "a.rs", None);
        let prepared = prepare(&store, 10).unwrap();
        let scored = score(&Stub(|_: &str| Ok(0.1)), prepared.candidates).await;
        let report = commit(&mut store, scored, prepared.skipped, 0.5, false).unwrap();
        assert_eq!((report.checked, report.still_stale), (1, 1));
        assert_eq!(status(&store, id), "stale");
    }

    #[test]
    fn missing_file_is_skipped() {
        let (mut store, repo) = store_with_file();
        let id = stale_memory(&mut store, repo, "claim", "gone.rs", None);
        let prepared = prepare(&store, 10).unwrap();
        assert!(prepared.candidates.is_empty());
        assert_eq!(prepared.skipped, 1);
        assert_eq!(status(&store, id), "stale");
    }

    #[test]
    fn skipped_memories_do_not_consume_the_limit() {
        let (mut store, repo) = store_with_file();
        stale_memory(&mut store, repo, "gone", "gone.rs", None);
        stale_memory(&mut store, repo, "kept", "a.rs", None);
        let prepared = prepare(&store, 1).unwrap();
        assert_eq!(prepared.candidates.len(), 1);
        assert_eq!(prepared.skipped, 1);
    }

    #[tokio::test]
    async fn verifier_error_is_isolated() {
        let (mut store, repo) = store_with_file();
        let first = stale_memory(&mut store, repo, "boom", "a.rs", None);
        let second = stale_memory(&mut store, repo, "fine", "a.rs", None);
        let prepared = prepare(&store, 10).unwrap();
        let scorer = Stub(|claim: &str| {
            if claim == "boom" {
                Err(Error::Verify("down".to_string()))
            } else {
                Ok(0.9)
            }
        });
        let scored = score(&scorer, prepared.candidates).await;
        let report = commit(&mut store, scored, prepared.skipped, 0.5, false).unwrap();
        assert_eq!(
            (report.checked, report.errors, report.reaffirmed),
            (2, 1, 1)
        );
        assert_eq!(status(&store, first), "stale");
        assert_eq!(status(&store, second), "live");
    }

    #[tokio::test]
    async fn dry_run_changes_nothing() {
        let (mut store, repo) = store_with_file();
        let id = stale_memory(&mut store, repo, "claim", "a.rs", None);
        let prepared = prepare(&store, 10).unwrap();
        let scored = score(&Stub(|_: &str| Ok(0.93)), prepared.candidates).await;
        let report = commit(&mut store, scored, prepared.skipped, 0.5, true).unwrap();
        assert_eq!(report.reaffirmed, 1);
        assert_eq!(status(&store, id), "stale");
    }
}
