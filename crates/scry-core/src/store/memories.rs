use rusqlite::{Row, params};

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
    pub kind: String,
    pub content: String,
    pub salience: f64,
    pub status: String,
    pub last_access: i64,
    pub access_count: i64,
    pub helpful_count: i64,
    pub stale_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SourceStats {
    pub source: String,
    pub total: i64,
    pub stale: i64,
    pub retrievals: i64,
    pub helpful: i64,
    pub noise: i64,
}

const ROW_COLUMNS: &str =
    "id, kind, content, salience, status, last_access, access_count, helpful_count, stale_at";

fn memory_row(row: &Row) -> rusqlite::Result<MemoryRow> {
    Ok(MemoryRow {
        id: row.get(0)?,
        kind: row.get(1)?,
        content: row.get(2)?,
        salience: row.get(3)?,
        status: row.get(4)?,
        last_access: row.get(5)?,
        access_count: row.get(6)?,
        helpful_count: row.get(7)?,
        stale_at: row.get(8)?,
    })
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
            "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)",
            params![memory_id, vector_bytes(&memory.embedding)],
        )?;
        for anchor in anchors {
            tx.execute(
                "INSERT INTO memory_anchors (memory_id, repo_id, relpath, start_line, end_line, xxh64)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    memory_id,
                    memory.repo_id,
                    anchor.relpath,
                    anchor.start_line,
                    anchor.end_line,
                    anchor.xxh64
                ],
            )?;
        }
        tx.commit()?;
        Ok(memory_id)
    }

    /// Nearest live/stale memories for a repo (plus repo-less globals),
    /// excluding superseded and archived rows.
    pub fn memory_candidates(
        &self,
        repo_id: Option<i64>,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<(MemoryRow, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT memory_id, distance FROM vec_memories
             WHERE embedding MATCH ?1 AND k = ?2",
        )?;
        let near: Vec<(i64, f64)> = stmt
            .query_map(params![vector_bytes(query), k as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<std::result::Result<_, _>>()?;

        let mut fetch = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM memories
             WHERE id = ?1 AND status != 'archived' AND superseded_by IS NULL
               AND (repo_id IS NULL OR repo_id = ?2)"
        ))?;
        let mut rows = Vec::new();
        for (memory_id, distance) in near {
            let row = fetch
                .query_row(params![memory_id, repo_id], memory_row)
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

    pub fn memory_feedback(&self, id: i64, helpful: bool) -> Result<bool> {
        let changed = if helpful {
            self.conn.execute(
                "UPDATE memories SET helpful_count = helpful_count + 1,
                 last_access = unixepoch() WHERE id = ?1",
                [id],
            )?
        } else {
            self.conn.execute(
                "UPDATE memories SET salience = salience * 0.8, noise_count = noise_count + 1
                 WHERE id = ?1",
                [id],
            )?
        };
        Ok(changed > 0)
    }

    /// Anchored memories go stale the moment their anchored file's content
    /// hash changes; sync calls this on every committed file.
    pub fn mark_stale_anchors(&self, repo_id: i64, relpath: &str, new_hash: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE memories SET status = 'stale', stale_at = unixepoch()
             WHERE status = 'live' AND id IN (
                 SELECT memory_id FROM memory_anchors
                 WHERE repo_id = ?1 AND relpath = ?2 AND xxh64 != ?3
             )",
            params![repo_id, relpath, new_hash],
        )?;
        Ok(changed)
    }

    /// Returns false, changing nothing, unless both memories exist and the
    /// old one is not already superseded.
    pub fn supersede_memory(&self, old_id: i64, new_id: i64) -> Result<bool> {
        let found: i64 = self.conn.query_row(
            "SELECT count(*) FROM memories
             WHERE (id = ?1 AND superseded_by IS NULL) OR id = ?2",
            params![old_id, new_id],
            |row| row.get(0),
        )?;
        if found != 2 {
            return Ok(false);
        }
        self.conn.execute(
            "UPDATE memories SET superseded_by = ?2 WHERE id = ?1",
            params![old_id, new_id],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO memory_links (memory_id, related_id, relation)
             VALUES (?2, ?1, 'supersedes')",
            params![old_id, new_id],
        )?;
        Ok(true)
    }

    /// Live memories in scope (plus repo-less globals); no access bookkeeping.
    pub fn live_memories(&self, repo_id: Option<i64>) -> Result<Vec<MemoryRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM memories
             WHERE status = 'live' AND superseded_by IS NULL
               AND (repo_id IS NULL OR repo_id = ?1)"
        ))?;
        let rows = stmt
            .query_map([repo_id], memory_row)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Memories in scope that went stale at or after `since`, newest first.
    pub fn recently_stale_memories(
        &self,
        repo_id: Option<i64>,
        since: i64,
        limit: usize,
    ) -> Result<Vec<MemoryRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM memories
             WHERE status = 'stale' AND superseded_by IS NULL AND stale_at >= ?2
               AND (repo_id IS NULL OR repo_id = ?1)
             ORDER BY stale_at DESC LIMIT ?3"
        ))?;
        let rows = stmt
            .query_map(params![repo_id, since, limit as i64], memory_row)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    pub fn memory_source_stats(&self) -> Result<Vec<SourceStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT source, count(*), count(*) FILTER (WHERE status = 'stale'),
                    sum(access_count), sum(helpful_count), sum(noise_count)
             FROM memories
             WHERE superseded_by IS NULL AND status != 'archived'
             GROUP BY source ORDER BY source",
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
        assert_eq!(store.live_memories(Some(repo)).unwrap().len(), 1);
        assert!(
            store
                .recently_stale_memories(Some(repo), 0, 5)
                .unwrap()
                .is_empty()
        );

        assert_eq!(store.mark_stale_anchors(repo, "a.rs", "new").unwrap(), 1);
        assert!(store.live_memories(Some(repo)).unwrap().is_empty());
        let stale = store.recently_stale_memories(Some(repo), 0, 5).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, id);
        assert!(stale[0].stale_at.is_some());
        let future = stale[0].stale_at.unwrap() + 1;
        assert!(
            store
                .recently_stale_memories(Some(repo), future, 5)
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
        assert_eq!(store.live_memories(Some(a)).unwrap().len(), 2);
        assert_eq!(store.live_memories(None).unwrap().len(), 1);
    }

    #[test]
    fn noise_feedback_counts_and_supersede_validates_ids() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let old = store
            .add_memory(&memory(None, "capture", 0.1), &[])
            .unwrap();
        let new = store.add_memory(&memory(None, "agent", 0.2), &[]).unwrap();
        store.memory_feedback(old, false).unwrap();
        store.memory_feedback(old, true).unwrap();

        assert!(!store.supersede_memory(old, 999).unwrap());
        assert_eq!(store.live_memories(None).unwrap().len(), 2);
        assert!(store.supersede_memory(old, new).unwrap());
        let third = store.add_memory(&memory(None, "agent", 0.4), &[]).unwrap();
        assert!(!store.supersede_memory(old, third).unwrap());
        assert_eq!(store.live_memories(None).unwrap().len(), 2);

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
        store.memory_feedback(a, true).unwrap();
        store.memory_feedback(a, false).unwrap();

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
