use rusqlite::{Connection, Row, params};

use super::{Store, vector_bytes};
use crate::Result;

#[derive(Debug, Clone)]
pub struct NewMemory {
    pub repo_id: Option<i64>,
    pub kind: String,
    pub content: String,
    pub salience: f64,
    pub surprise: f64,
    pub cost: f64,
    pub explicit_weight: f64,
    pub source: String,
    pub evidence: Option<String>,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct MemoryAnchor {
    pub relpath: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub xxh64: String,
}

#[derive(Debug, Clone)]
pub struct MemoryRow {
    pub id: i64,
    pub repo_id: Option<i64>,
    pub kind: String,
    pub content: String,
    pub salience: f64,
    pub status: String,
    pub last_access: i64,
    pub access_count: i64,
    pub helpful_count: i64,
    pub stale_at: Option<i64>,
    pub superseded_by: Option<i64>,
    pub helpful_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SourceStats {
    pub source: String,
    pub total: i64,
    pub stale: i64,
    pub retrievals: i64,
    pub helpful: i64,
    pub noise: i64,
    pub archived: i64,
}

/// Vector partition for a scope; globals share partition 0 because repo
/// ids start at 1.
pub(super) fn scope_partition(repo_id: Option<i64>) -> i64 {
    repo_id.unwrap_or(0)
}

/// Who is asking, which decides the visible memories and the suppressions
/// that apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// No scope key: globals only, under the unscoped suppression votes.
    Unscoped,
    /// A scope key with a repo row.
    Repo(i64),
    /// A scope key with no repo row yet: globals only, no suppressions.
    Unseen,
}

impl Scope {
    pub fn repo_id(self) -> Option<i64> {
        match self {
            Scope::Repo(id) => Some(id),
            Scope::Unscoped | Scope::Unseen => None,
        }
    }

    pub fn is_scoped(self) -> bool {
        self != Scope::Unscoped
    }

    /// The partition whose suppression votes apply, if any.
    fn suppression_partition(self) -> Option<i64> {
        match self {
            Scope::Unscoped => Some(0),
            Scope::Repo(id) => Some(id),
            Scope::Unseen => None,
        }
    }
}

/// Salience factor of one noise vote; restoring a memory undoes it.
pub(super) const NOISE_DECAY: f64 = 0.8;

/// Excludes memories marked noise in the asking scope.
fn not_suppressed(scope: Scope) -> String {
    scope
        .suppression_partition()
        .map_or_else(|| "TRUE".to_string(), suppressed_in)
}

fn suppressed_in(partition: i64) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM memory_suppressions s
                     WHERE s.memory_id = memories.id AND s.repo_id = {partition})"
    )
}

/// Matches an anchor `a` whose file is no longer indexed under the hash the
/// anchor recorded. The one definition of "the anchored code changed".
pub(super) const ANCHOR_DRIFTED: &str = "NOT EXISTS (SELECT 1 FROM files f
     WHERE f.repo_id = a.repo_id AND f.relpath = a.relpath AND f.xxh64 = a.xxh64)";

pub(super) const ROW_COLUMNS: &str = "id, repo_id, kind, content, salience, status, last_access, access_count, helpful_count, stale_at, superseded_by, helpful_at";

pub(super) fn memory_row(row: &Row) -> rusqlite::Result<MemoryRow> {
    Ok(MemoryRow {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        kind: row.get(2)?,
        content: row.get(3)?,
        salience: row.get(4)?,
        status: row.get(5)?,
        last_access: row.get(6)?,
        access_count: row.get(7)?,
        helpful_count: row.get(8)?,
        stale_at: row.get(9)?,
        superseded_by: row.get(10)?,
        helpful_at: row.get(11)?,
    })
}

pub(super) fn insert_anchors(
    conn: &Connection,
    memory_id: i64,
    repo_id: Option<i64>,
    anchors: &[MemoryAnchor],
) -> Result<()> {
    for anchor in anchors {
        conn.execute(
            "INSERT INTO memory_anchors (memory_id, repo_id, relpath, start_line, end_line, xxh64)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                memory_id,
                repo_id,
                anchor.relpath,
                anchor.start_line,
                anchor.end_line,
                anchor.xxh64
            ],
        )?;
    }
    Ok(())
}

impl Store {
    pub fn add_memory(&mut self, memory: &NewMemory, anchors: &[MemoryAnchor]) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO memories (repo_id, kind, content, salience, surprise, cost,
                                   explicit_weight, source, evidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                memory.repo_id,
                memory.kind,
                memory.content,
                memory.salience,
                memory.surprise,
                memory.cost,
                memory.explicit_weight,
                memory.source,
                memory.evidence
            ],
        )?;
        let memory_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO vec_memories (memory_id, repo_id, embedding) VALUES (?1, ?2, ?3)",
            params![
                memory_id,
                scope_partition(memory.repo_id),
                vector_bytes(&memory.embedding)
            ],
        )?;
        insert_anchors(&tx, memory_id, memory.repo_id, anchors)?;
        tx.commit()?;
        Ok(memory_id)
    }

    /// Nearest live/stale memories for a repo (plus repo-less globals),
    /// excluding superseded, archived, and suppressed rows. Each scope is its own
    /// partitioned KNN so a large neighbor scope cannot crowd the result; the
    /// merged list is cut to the `k` nearest before filtering.
    pub fn memory_candidates(
        &self,
        scope: Scope,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<(MemoryRow, f64)>> {
        let repo_id = scope.repo_id();
        let mut near = self.memory_neighbors(scope_partition(repo_id), query, k)?;
        if repo_id.is_some() {
            near.extend(self.memory_neighbors(0, query, k)?);
        }
        near.sort_by(|a, b| a.1.total_cmp(&b.1));
        near.truncate(k);

        let mut fetch = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM memories
             WHERE id = ?1 AND status != 'archived' AND superseded_by IS NULL
               AND {}",
            not_suppressed(scope)
        ))?;
        let mut rows = Vec::new();
        for (memory_id, distance) in near {
            let row = fetch
                .query_row(params![memory_id], memory_row)
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    e => Err(e),
                })?;
            if let Some(row) = row {
                rows.push((row, distance));
            }
        }
        Ok(rows)
    }

    fn memory_neighbors(&self, partition: i64, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT memory_id, distance FROM vec_memories
             WHERE embedding MATCH ?1 AND k = ?2 AND repo_id = ?3",
        )?;
        let near = stmt
            .query_map(params![vector_bytes(query), k as i64, partition], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(near)
    }

    /// Highest cosine similarity to any existing memory; the complement is
    /// the write-time surprise signal.
    pub fn nearest_memory_similarity(&self, embedding: &[f32]) -> Result<f64> {
        let distance: Option<f64> = self
            .conn
            .query_row(
                "SELECT distance FROM vec_memories WHERE embedding MATCH ?1 AND k = 1",
                params![vector_bytes(embedding)],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })?;
        Ok(distance.map_or(0.0, |d| (1.0 - d).clamp(0.0, 1.0)))
    }

    pub fn touch_memories(&self, ids: &[i64]) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "UPDATE memories SET access_count = access_count + 1, last_access = unixepoch()
             WHERE id = ?1",
        )?;
        for id in ids {
            stmt.execute([id])?;
        }
        Ok(())
    }

    /// A helpful vote stamps `helpful_at` and lifts the memory's suppression
    /// in `scope`; a noise vote suppresses it there. An unseen scope has no
    /// suppressions to lift, and noise never arrives from one.
    pub fn memory_feedback(&self, id: i64, helpful: bool, scope: Scope) -> Result<bool> {
        let partition = scope.suppression_partition();
        let tx = self.conn.unchecked_transaction()?;
        let changed = if helpful {
            let changed = tx.execute(
                "UPDATE memories SET helpful_count = helpful_count + 1,
                 last_access = unixepoch(), helpful_at = unixepoch() WHERE id = ?1",
                [id],
            )?;
            if let Some(partition) = partition {
                tx.execute(
                    "DELETE FROM memory_suppressions WHERE memory_id = ?1 AND repo_id = ?2",
                    params![id, partition],
                )?;
            }
            changed
        } else {
            let changed = tx.execute(
                "UPDATE memories SET salience = salience * ?2, noise_count = noise_count + 1
                 WHERE id = ?1",
                params![id, NOISE_DECAY],
            )?;
            if let (true, Some(partition)) = (changed > 0, partition) {
                tx.execute(
                    "INSERT INTO memory_suppressions (memory_id, repo_id) VALUES (?1, ?2)
                     ON CONFLICT (memory_id, repo_id) DO UPDATE SET created_at = unixepoch()",
                    params![id, partition],
                )?;
            }
            changed
        };
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Moves the vectors of a repo's memories to `target`'s partition, or
    /// to the global partition when `target` is `None`.
    pub(super) fn repartition_memory_vectors(
        conn: &Connection,
        repo_id: i64,
        target: Option<i64>,
    ) -> Result<()> {
        conn.execute(
            "CREATE TEMP TABLE memory_vectors_moving AS
             SELECT v.memory_id, v.embedding FROM vec_memories v
             JOIN memories m ON m.id = v.memory_id
             WHERE m.repo_id = ?1 AND m.status != 'archived'",
            [repo_id],
        )?;
        conn.execute_batch(
            "DELETE FROM vec_memories
             WHERE memory_id IN (SELECT memory_id FROM memory_vectors_moving)",
        )?;
        conn.execute(
            "INSERT INTO vec_memories (memory_id, repo_id, embedding)
             SELECT memory_id, ?1, embedding FROM memory_vectors_moving",
            [scope_partition(target)],
        )?;
        conn.execute_batch("DROP TABLE memory_vectors_moving")?;
        Ok(())
    }

    /// Anchored memories go stale the moment their anchored file's content
    /// hash changes; sync calls this on every committed file, after the file
    /// row holds the new hash.
    pub fn mark_stale_anchors(&self, repo_id: i64, relpath: &str) -> Result<usize> {
        let changed = self.conn.execute(
            &format!(
                "UPDATE memories SET status = 'stale', stale_at = unixepoch()
                 WHERE status = 'live' AND id IN (
                     SELECT a.memory_id FROM memory_anchors a
                     WHERE a.repo_id = ?1 AND a.relpath = ?2 AND {ANCHOR_DRIFTED}
                 )"
            ),
            params![repo_id, relpath],
        )?;
        Ok(changed)
    }

    /// Returns false, changing nothing, unless both memories exist and
    /// neither is already superseded.
    pub fn supersede_memory(&self, old_id: i64, new_id: i64) -> Result<bool> {
        let found: i64 = self.conn.query_row(
            "SELECT count(*) FROM memories
             WHERE id IN (?1, ?2) AND superseded_by IS NULL",
            params![old_id, new_id],
            |row| row.get(0),
        )?;
        if found != 2 {
            return Ok(false);
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE memories SET superseded_by = ?2 WHERE id = ?1",
            params![old_id, new_id],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO memory_links (memory_id, related_id, relation)
             VALUES (?2, ?1, 'supersedes')",
            params![old_id, new_id],
        )?;
        tx.execute("DELETE FROM vec_memories WHERE memory_id = ?1", [old_id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Live, unsuppressed memories in scope (plus repo-less globals); no
    /// access bookkeeping.
    pub fn live_memories(&self, scope: Scope) -> Result<Vec<MemoryRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM memories
             WHERE status = 'live' AND superseded_by IS NULL
               AND (repo_id IS NULL OR repo_id = ?1) AND {}",
            not_suppressed(scope)
        ))?;
        let rows = stmt
            .query_map([scope.repo_id()], memory_row)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Unsuppressed memories in scope that went stale at or after `since`,
    /// newest first.
    pub fn recently_stale_memories(
        &self,
        scope: Scope,
        since: i64,
        limit: usize,
    ) -> Result<Vec<MemoryRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM memories
             WHERE status = 'stale' AND superseded_by IS NULL AND stale_at >= ?2
               AND (repo_id IS NULL OR repo_id = ?1) AND {}
             ORDER BY stale_at DESC LIMIT ?3",
            not_suppressed(scope)
        ))?;
        let rows = stmt
            .query_map(params![scope.repo_id(), since, limit as i64], memory_row)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    pub fn memory_source_stats(&self) -> Result<Vec<SourceStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT source,
                    count(*) FILTER (WHERE live),
                    count(*) FILTER (WHERE live AND status = 'stale'),
                    coalesce(sum(access_count) FILTER (WHERE live), 0),
                    coalesce(sum(helpful_count) FILTER (WHERE live), 0),
                    coalesce(sum(noise_count) FILTER (WHERE live), 0),
                    count(*) FILTER (WHERE status = 'archived')
             FROM (SELECT *, superseded_by IS NULL AND status != 'archived' AS live
                   FROM memories)
             GROUP BY source
             HAVING count(*) FILTER (WHERE live OR status = 'archived') > 0
             ORDER BY source",
        )?;
        let stats = stmt
            .query_map([], |row| {
                Ok(SourceStats {
                    source: row.get(0)?,
                    total: row.get(1)?,
                    stale: row.get(2)?,
                    retrievals: row.get(3)?,
                    helpful: row.get(4)?,
                    noise: row.get(5)?,
                    archived: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(stats)
    }

    pub fn memory_counts(&self) -> Result<(i64, i64)> {
        let total = self.conn.query_row(
            "SELECT count(*) FROM memories WHERE superseded_by IS NULL AND status != 'archived'",
            [],
            |row| row.get(0),
        )?;
        let stale = self.conn.query_row(
            "SELECT count(*) FROM memories WHERE status = 'stale' AND superseded_by IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok((total, stale))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(repo_id: Option<i64>, source: &str, seed: f32) -> NewMemory {
        NewMemory {
            repo_id,
            kind: "lesson".to_string(),
            content: format!("memory {seed}"),
            salience: 0.5,
            surprise: 0.0,
            cost: 0.0,
            explicit_weight: 0.0,
            source: source.to_string(),
            evidence: Some("transcript".to_string()),
            embedding: vec![seed, 1.0, 0.0, 0.0],
        }
    }

    #[test]
    fn staleness_stamps_stale_at_and_feeds_the_recent_list() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let repo = store.upsert_repo("github.com/x/y").unwrap();
        let anchor = MemoryAnchor {
            relpath: "a.rs".to_string(),
            start_line: None,
            end_line: None,
            xxh64: "old".to_string(),
        };
        let id = store
            .add_memory(&memory(Some(repo), "agent", 0.1), &[anchor])
            .unwrap();
        assert_eq!(store.live_memories(Scope::Repo(repo)).unwrap().len(), 1);
        assert!(
            store
                .recently_stale_memories(Scope::Repo(repo), 0, 5)
                .unwrap()
                .is_empty()
        );

        assert_eq!(store.mark_stale_anchors(repo, "a.rs").unwrap(), 1);
        assert!(store.live_memories(Scope::Repo(repo)).unwrap().is_empty());
        let stale = store
            .recently_stale_memories(Scope::Repo(repo), 0, 5)
            .unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, id);
        assert!(stale[0].stale_at.is_some());
        let future = stale[0].stale_at.unwrap() + 1;
        assert!(
            store
                .recently_stale_memories(Scope::Repo(repo), future, 5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn scope_filter_includes_globals_and_excludes_other_repos() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/a").unwrap();
        let b = store.upsert_repo("github.com/x/b").unwrap();
        store
            .add_memory(&memory(Some(a), "agent", 0.1), &[])
            .unwrap();
        store
            .add_memory(&memory(Some(b), "agent", 0.2), &[])
            .unwrap();
        store.add_memory(&memory(None, "agent", 0.3), &[]).unwrap();
        assert_eq!(store.live_memories(Scope::Repo(a)).unwrap().len(), 2);
        assert_eq!(store.live_memories(Scope::Unscoped).unwrap().len(), 1);
    }

    #[test]
    fn candidates_search_each_scope_so_a_large_neighbor_cannot_crowd() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/a").unwrap();
        let b = store.upsert_repo("github.com/x/b").unwrap();
        for i in 0..6 {
            store
                .add_memory(&memory(Some(b), "agent", 0.01 * i as f32), &[])
                .unwrap();
        }
        let mine = store
            .add_memory(&memory(Some(a), "agent", 0.9), &[])
            .unwrap();
        let global = store.add_memory(&memory(None, "agent", 0.8), &[]).unwrap();

        let found = store
            .memory_candidates(Scope::Repo(a), &[0.0, 1.0, 0.0, 0.0], 2)
            .unwrap();
        let ids: Vec<i64> = found.iter().map(|(row, _)| row.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&mine) && ids.contains(&global));
        assert!(found.windows(2).all(|w| w[0].1 <= w[1].1));
    }

    #[test]
    fn suppression_is_per_scope_and_lifted_only_by_a_helpful_vote_there() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/a").unwrap();
        let b = store.upsert_repo("github.com/x/b").unwrap();
        let global = store.add_memory(&memory(None, "agent", 0.1), &[]).unwrap();
        let query = [0.1, 1.0, 0.0, 0.0];
        let visible = |scope: i64| {
            store
                .memory_candidates(Scope::Repo(scope), &query, 5)
                .unwrap()
                .len()
                + store.live_memories(Scope::Repo(scope)).unwrap().len()
        };
        assert_eq!(visible(a), 2);

        store
            .memory_feedback(global, false, Scope::Repo(a))
            .unwrap();
        assert_eq!(visible(a), 0);
        assert_eq!(visible(b), 2);

        store.memory_feedback(global, true, Scope::Repo(b)).unwrap();
        assert_eq!(visible(a), 0);

        store
            .memory_feedback(global, false, Scope::Repo(a))
            .unwrap();
        assert_eq!(visible(a), 0);
        store.memory_feedback(global, true, Scope::Repo(a)).unwrap();
        assert_eq!(visible(a), 2);
    }

    #[test]
    fn a_noise_vote_without_a_key_does_not_hide_the_memory_in_a_new_scope() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let global = store.add_memory(&memory(None, "agent", 0.1), &[]).unwrap();
        let query = [0.1, 1.0, 0.0, 0.0];
        store
            .memory_feedback(global, false, Scope::Unscoped)
            .unwrap();

        let unseen = store.scope(Some("github.com/x/new")).unwrap();
        assert_eq!(unseen, Scope::Unseen);
        assert_eq!(store.memory_candidates(unseen, &query, 5).unwrap().len(), 1);
        assert_eq!(store.live_memories(unseen).unwrap().len(), 1);
        assert!(
            store
                .memory_candidates(Scope::Unscoped, &query, 5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn pruning_moves_vectors_and_suppressions_together() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/a").unwrap();
        let b = store.upsert_repo("github.com/x/b").unwrap();
        let id = store
            .add_memory(&memory(Some(a), "agent", 0.1), &[])
            .unwrap();
        store.memory_feedback(id, false, Scope::Repo(a)).unwrap();
        store.prune_repo("github.com/x/a", Some(b)).unwrap();

        let partitions = |table: &str| -> Vec<i64> {
            let mut stmt = store
                .conn
                .prepare(&format!("SELECT repo_id FROM {table} WHERE memory_id = ?1"))
                .unwrap();
            stmt.query_map([id], |row| row.get(0))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap()
        };
        assert_eq!(partitions("vec_memories"), vec![b]);
        assert_eq!(partitions("memory_suppressions"), vec![b]);
    }

    #[test]
    fn pruning_a_repo_moves_its_memory_vectors() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/a").unwrap();
        let b = store.upsert_repo("github.com/x/b").unwrap();
        let id = store
            .add_memory(&memory(Some(a), "agent", 0.1), &[])
            .unwrap();
        let query = [0.1, 1.0, 0.0, 0.0];
        store.prune_repo("github.com/x/a", Some(b)).unwrap();
        assert_eq!(
            store.memory_candidates(Scope::Repo(b), &query, 5).unwrap()[0]
                .0
                .id,
            id
        );
        store.prune_repo("github.com/x/b", None).unwrap();
        assert_eq!(
            store.memory_candidates(Scope::Unscoped, &query, 5).unwrap()[0]
                .0
                .id,
            id
        );
    }

    #[test]
    fn pruning_without_a_target_archives_what_the_repo_suppressed() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/a").unwrap();
        let noisy = store
            .add_memory(&memory(Some(a), "agent", 0.1), &[])
            .unwrap();
        let kept = store
            .add_memory(&memory(Some(a), "agent", 0.2), &[])
            .unwrap();
        store.memory_feedback(noisy, false, Scope::Repo(a)).unwrap();
        store.prune_repo("github.com/x/a", None).unwrap();

        assert_eq!(store.memory(noisy).unwrap().unwrap().status, "archived");
        assert_eq!(store.memory(kept).unwrap().unwrap().status, "live");
        let query = [0.1, 1.0, 0.0, 0.0];
        let found = store.memory_candidates(Scope::Unscoped, &query, 5).unwrap();
        assert_eq!(found.iter().map(|(m, _)| m.id).collect::<Vec<_>>(), [kept]);
    }

    #[test]
    fn pruning_without_a_target_detaches_a_suppressed_memory_with_helpful_votes() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/a").unwrap();
        let helped = store
            .add_memory(&memory(Some(a), "agent", 0.1), &[])
            .unwrap();
        store.memory_feedback(helped, true, Scope::Repo(a)).unwrap();
        store
            .memory_feedback(helped, false, Scope::Repo(a))
            .unwrap();
        store.prune_repo("github.com/x/a", None).unwrap();

        let row = store.memory(helped).unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.repo_id), ("live", None));
        let query = [0.1, 1.0, 0.0, 0.0];
        let found = store.memory_candidates(Scope::Unscoped, &query, 5).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn superseding_drops_the_old_vector() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let old = store.add_memory(&memory(None, "agent", 0.1), &[]).unwrap();
        let new = store.add_memory(&memory(None, "agent", 0.2), &[]).unwrap();
        assert!(store.supersede_memory(old, new).unwrap());

        let vectors: Vec<i64> = store
            .conn
            .prepare("SELECT memory_id FROM vec_memories")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(vectors, [new]);
    }

    #[test]
    fn pruning_skips_a_live_memory_that_has_no_vector() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/a").unwrap();
        let b = store.upsert_repo("github.com/x/b").unwrap();
        let orphan = store
            .add_memory(&memory(Some(a), "agent", 0.1), &[])
            .unwrap();
        let moved = store
            .add_memory(&memory(Some(a), "agent", 0.2), &[])
            .unwrap();
        store
            .conn
            .execute("DELETE FROM vec_memories WHERE memory_id = ?1", [orphan])
            .unwrap();
        store.prune_repo("github.com/x/a", Some(b)).unwrap();

        let query = [0.2, 1.0, 0.0, 0.0];
        let found = store.memory_candidates(Scope::Repo(b), &query, 5).unwrap();
        assert_eq!(found.iter().map(|(m, _)| m.id).collect::<Vec<_>>(), [moved]);
    }

    #[test]
    fn noise_feedback_counts_and_supersede_validates_ids() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let old = store
            .add_memory(&memory(None, "capture", 0.1), &[])
            .unwrap();
        let new = store.add_memory(&memory(None, "agent", 0.2), &[]).unwrap();
        store.memory_feedback(old, false, Scope::Unscoped).unwrap();
        store.memory_feedback(old, true, Scope::Unscoped).unwrap();

        assert!(!store.supersede_memory(old, 999).unwrap());
        assert_eq!(store.live_memories(Scope::Unscoped).unwrap().len(), 2);
        assert!(store.supersede_memory(old, new).unwrap());
        let third = store.add_memory(&memory(None, "agent", 0.4), &[]).unwrap();
        assert!(!store.supersede_memory(old, third).unwrap());
        assert!(!store.supersede_memory(new, old).unwrap());
        assert!(!store.supersede_memory(third, old).unwrap());
        assert_eq!(store.live_memories(Scope::Unscoped).unwrap().len(), 2);

        let noise: i64 = store
            .conn
            .query_row(
                "SELECT noise_count FROM memories WHERE id = ?1",
                [old],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(noise, 1);
    }

    #[test]
    fn source_stats_group_non_superseded_rows() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let a = store
            .add_memory(&memory(None, "capture", 0.1), &[])
            .unwrap();
        store
            .add_memory(&memory(None, "capture", 0.2), &[])
            .unwrap();
        store.add_memory(&memory(None, "agent", 0.3), &[]).unwrap();
        store.touch_memories(&[a, a]).unwrap();
        store.memory_feedback(a, true, Scope::Unscoped).unwrap();
        store.memory_feedback(a, false, Scope::Unscoped).unwrap();

        let stats = store.memory_source_stats().unwrap();
        let capture = stats.iter().find(|s| s.source == "capture").unwrap();
        assert_eq!(
            (
                capture.total,
                capture.stale,
                capture.retrievals,
                capture.helpful,
                capture.noise
            ),
            (2, 0, 2, 1, 1)
        );
        let agent = stats.iter().find(|s| s.source == "agent").unwrap();
        assert_eq!(agent.total, 1);
    }
}
