//! Memory retirement: archive what recall should no longer surface, keeping
//! the text so a human can restore it.

use crate::Result;
use crate::config::MemoryConfig;
use crate::memory::{now_epoch, recall_score};
use crate::store::{MemoryRow, Store};

const NOISE_SCOPES: i64 = 2;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub superseded: usize,
    pub stale: usize,
    pub episodes: usize,
    pub noise: usize,
}

impl GcReport {
    pub fn total(&self) -> usize {
        self.superseded + self.stale + self.episodes + self.noise
    }
}

enum Rule {
    Superseded,
    Stale,
    Episode,
    Noise,
}

/// Archives memories that match a rule, the first match claiming the row.
/// With `dry_run` it only counts.
pub fn gc(store: &mut Store, config: &MemoryConfig, dry_run: bool) -> Result<GcReport> {
    let now = now_epoch();
    let mut report = GcReport::default();
    let mut ids = Vec::new();
    let suppressed = store.suppression_scope_counts()?;
    for row in store.archivable_candidates()? {
        let scopes = suppressed.get(&row.id).copied().unwrap_or(0);
        let Some(rule) = matching_rule(&row, scopes, config, now) else {
            continue;
        };
        match rule {
            Rule::Superseded => report.superseded += 1,
            Rule::Stale => report.stale += 1,
            Rule::Episode => report.episodes += 1,
            Rule::Noise => report.noise += 1,
        }
        ids.push(row.id);
    }
    if !dry_run {
        store.archive_memories(&ids)?;
    }
    Ok(report)
}

fn matching_rule(row: &MemoryRow, scopes: i64, config: &MemoryConfig, now: i64) -> Option<Rule> {
    if row.superseded_by.is_some() {
        return Some(Rule::Superseded);
    }
    let stale_cutoff = now - i64::from(config.gc_stale_days) * 86_400;
    if let Some(stale_at) = row.stale_at
        && row.status == "stale"
        && stale_at < stale_cutoff
        && row.helpful_at.is_none_or(|voted| voted < stale_at)
    {
        return Some(Rule::Stale);
    }
    if row.kind == "episode"
        && row.helpful_count == 0
        && recall_score(1.0, row, now, config.half_life_days) < config.gc_episode_floor
    {
        return Some(Rule::Episode);
    }
    if scopes >= NOISE_SCOPES && row.helpful_count == 0 {
        return Some(Rule::Noise);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewMemory;

    fn add(store: &mut Store, kind: &str, seed: f32) -> i64 {
        store
            .add_memory(
                &NewMemory {
                    repo_id: None,
                    kind: kind.to_string(),
                    content: format!("memory {seed}"),
                    salience: 0.5,
                    surprise: 0.0,
                    cost: 0.0,
                    explicit_weight: 0.0,
                    source: "agent".to_string(),
                    evidence: None,
                    embedding: vec![seed, 1.0, 0.0, 0.0],
                },
                &[],
            )
            .unwrap()
    }

    fn exec(store: &Store, sql: &str, id: i64) {
        store.conn.execute(sql, [id]).unwrap();
    }

    #[test]
    fn each_rule_archives_its_rows_and_dry_run_changes_nothing() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let old = add(&mut store, "lesson", 0.1);
        let new = add(&mut store, "lesson", 0.2);
        store.supersede_memory(old, new).unwrap();

        let stale = add(&mut store, "lesson", 0.3);
        exec(
            &store,
            "UPDATE memories SET status = 'stale', stale_at = 1 WHERE id = ?1",
            stale,
        );
        let revived = add(&mut store, "lesson", 0.4);
        exec(
            &store,
            "UPDATE memories SET status = 'stale', stale_at = 1, helpful_at = 5 WHERE id = ?1",
            revived,
        );
        let fresh_stale = add(&mut store, "lesson", 0.5);
        exec(
            &store,
            "UPDATE memories SET status = 'stale', stale_at = unixepoch() WHERE id = ?1",
            fresh_stale,
        );

        let faded = add(&mut store, "episode", 0.6);
        exec(
            &store,
            "UPDATE memories SET last_access = 1, salience = 0.05 WHERE id = ?1",
            faded,
        );
        let helped = add(&mut store, "episode", 0.7);
        exec(
            &store,
            "UPDATE memories SET last_access = 1, salience = 0.05, helpful_count = 1 WHERE id = ?1",
            helped,
        );

        let noisy = add(&mut store, "lesson", 0.8);
        store.memory_feedback(noisy, false, Some(1)).unwrap();
        store.memory_feedback(noisy, false, Some(2)).unwrap();
        let once = add(&mut store, "lesson", 0.9);
        store.memory_feedback(once, false, Some(1)).unwrap();
        store.memory_feedback(once, false, Some(1)).unwrap();

        let config = MemoryConfig::default();
        let expected = GcReport {
            superseded: 1,
            stale: 1,
            episodes: 1,
            noise: 1,
        };
        assert_eq!(gc(&mut store, &config, true).unwrap(), expected);
        assert_eq!(store.archivable_candidates().unwrap().len(), 9);

        assert_eq!(gc(&mut store, &config, false).unwrap(), expected);
        let kept: Vec<i64> = store
            .archivable_candidates()
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(kept, vec![new, revived, fresh_stale, helped, once]);
        assert_eq!(gc(&mut store, &config, false).unwrap().total(), 0);
    }
}
