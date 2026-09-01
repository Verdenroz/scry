//! Code-anchored memory: kinds from the CoALA taxonomy, valence-neutral
//! salience at write time, ACT-R style activation at recall, staleness
//! driven by the sync pipeline's file hashes.

use crate::embed::Embedder;
use crate::store::{MemoryAnchor, MemoryRow, NewMemory, Store};
use crate::{Error, Result};

pub const KINDS: &[&str] = &[
    "lesson",
    "decision",
    "convention",
    "skill",
    "fact",
    "episode",
];

const SURPRISE_WEIGHT: f64 = 0.25;
const COST_WEIGHT: f64 = 0.35;
const EXPLICIT_WEIGHT: f64 = 0.4;
const STALE_FACTOR: f64 = 0.5;
const CANDIDATES: usize = 40;

#[derive(Debug, Clone)]
pub struct MemoryDraft {
    pub repo_id: Option<i64>,
    pub kind: String,
    pub content: String,
    /// Explicit importance 0-10 (`--pain`); the strongest salience term.
    pub pain: Option<f64>,
    /// Session cost signal 0-10 (retries, time burned) from the writer.
    pub cost: Option<f64>,
    pub anchors: Vec<AnchorSpec>,
}

#[derive(Debug, Clone)]
pub struct AnchorSpec {
    pub relpath: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub id: i64,
    pub kind: String,
    pub content: String,
    pub score: f64,
    pub stale: bool,
}

pub fn embed_text(kind: &str, content: &str) -> String {
    format!("memory > {kind}\n{content}")
}

fn compose_salience(surprise: f64, cost: f64, explicit: f64) -> f64 {
    (SURPRISE_WEIGHT * surprise + COST_WEIGHT * cost + EXPLICIT_WEIGHT * explicit).clamp(0.05, 1.0)
}

/// Recall score: similarity gated by stored salience, recency decay
/// (half-life in days), learned utility, and staleness demotion. Memories
/// fade rather than vanish; one helpful retrieval re-strengthens them.
pub fn recall_score(similarity: f64, row: &MemoryRow, now_epoch: i64, half_life_days: f64) -> f64 {
    let age_days = ((now_epoch - row.last_access).max(0) as f64) / 86_400.0;
    let recency = (-(std::f64::consts::LN_2) * age_days / half_life_days.max(0.1)).exp();
    let utility = row.helpful_count as f64 / (row.access_count.max(1) as f64);
    let status = if row.status == "stale" {
        STALE_FACTOR
    } else {
        1.0
    };
    similarity * (0.2 + 0.8 * row.salience) * (0.3 + 0.7 * recency) * (1.0 + 0.5 * utility) * status
}

pub fn validate_kind(kind: &str) -> Result<()> {
    if KINDS.contains(&kind) {
        return Ok(());
    }
    Err(Error::Config(format!(
        "unknown memory kind {kind:?}; expected one of {}",
        KINDS.join(", ")
    )))
}

pub async fn remember(
    store: &mut Store,
    embedder: &dyn Embedder,
    draft: MemoryDraft,
) -> Result<(i64, f64, f64)> {
    validate_kind(&draft.kind)?;
    let embedding = embedder
        .embed(&[embed_text(&draft.kind, &draft.content)])
        .await?
        .pop()
        .ok_or_else(|| Error::Embedding("empty embedding response".to_string()))?;
    remember_with_embedding(store, draft, embedding)
}

/// The synchronous half of [`remember`]; the embedding comes from the
/// caller so no await ever holds the store.
pub fn remember_with_embedding(
    store: &mut Store,
    draft: MemoryDraft,
    embedding: Vec<f32>,
) -> Result<(i64, f64, f64)> {
    let surprise = 1.0 - store.nearest_memory_similarity(&embedding)?;
    let cost = draft.cost.map_or(0.0, |c| (c / 10.0).clamp(0.0, 1.0));
    let explicit = draft.pain.map_or(0.3, |p| (p / 10.0).clamp(0.0, 1.0));
    let salience = compose_salience(surprise, cost, explicit);

    let anchors = resolve_anchors(store, draft.repo_id, &draft.anchors)?;
    let id = store.add_memory(
        &NewMemory {
            repo_id: draft.repo_id,
            kind: draft.kind,
            content: draft.content,
            salience,
            surprise,
            cost,
            explicit_weight: explicit,
            embedding,
        },
        &anchors,
    )?;
    Ok((id, salience, surprise))
}

fn resolve_anchors(
    store: &Store,
    repo_id: Option<i64>,
    specs: &[AnchorSpec],
) -> Result<Vec<MemoryAnchor>> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let Some(repo_id) = repo_id else {
        return Err(Error::Config(
            "anchors require a repo-scoped memory".to_string(),
        ));
    };
    let files: std::collections::HashMap<String, String> = store
        .list_files(repo_id)?
        .into_iter()
        .map(|f| (f.relpath, f.xxh64))
        .collect();
    specs
        .iter()
        .map(|spec| {
            let xxh64 = files.get(&spec.relpath).cloned().ok_or_else(|| {
                Error::Config(format!(
                    "anchor {} is not in the index; sync first",
                    spec.relpath
                ))
            })?;
            Ok(MemoryAnchor {
                relpath: spec.relpath.clone(),
                start_line: spec.start_line,
                end_line: spec.end_line,
                xxh64,
            })
        })
        .collect()
}

pub fn recall_with_vector(
    store: &Store,
    repo_id: Option<i64>,
    vector: &[f32],
    limit: usize,
    half_life_days: f64,
) -> Result<Vec<MemoryHit>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let mut hits: Vec<MemoryHit> = store
        .memory_candidates(repo_id, vector, CANDIDATES)?
        .into_iter()
        .map(|(row, distance)| {
            let similarity = (1.0 - distance).clamp(0.0, 1.0);
            MemoryHit {
                id: row.id,
                kind: row.kind.clone(),
                content: row.content.clone(),
                score: recall_score(similarity, &row, now, half_life_days),
                stale: row.status == "stale",
            }
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(limit.max(1));
    store.touch_memories(&hits.iter().map(|h| h.id).collect::<Vec<_>>())?;
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(salience: f64, status: &str, helpful: i64, access: i64, last_access: i64) -> MemoryRow {
        MemoryRow {
            id: 1,
            kind: "lesson".to_string(),
            content: String::new(),
            salience,
            status: status.to_string(),
            last_access,
            access_count: access,
            helpful_count: helpful,
        }
    }

    #[test]
    fn stale_memories_are_demoted_not_dropped() {
        let now = 1_000_000;
        let live = recall_score(0.8, &row(0.7, "live", 0, 0, now), now, 29.0);
        let stale = recall_score(0.8, &row(0.7, "stale", 0, 0, now), now, 29.0);
        assert!(stale > 0.0);
        assert!((stale / live - 0.5).abs() < 1e-9);
    }

    #[test]
    fn disuse_decays_and_utility_reinforces() {
        let now = 1_000_000;
        let month = 30 * 86_400;
        let fresh = recall_score(0.8, &row(0.7, "live", 0, 0, now), now, 29.0);
        let old = recall_score(0.8, &row(0.7, "live", 0, 0, now - month), now, 29.0);
        assert!(old < fresh);
        let helpful = recall_score(0.8, &row(0.7, "live", 5, 5, now - month), now, 29.0);
        assert!(helpful > old);
    }

    #[test]
    fn salience_composition_is_bounded() {
        assert!(compose_salience(0.0, 0.0, 0.0) >= 0.05);
        assert!(compose_salience(1.0, 1.0, 1.0) <= 1.0);
        assert!(compose_salience(0.2, 0.0, 0.9) > compose_salience(0.2, 0.0, 0.2));
    }
}
