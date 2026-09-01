use rusqlite::params;

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
}

impl Store {
    pub fn add_memory(&mut self, memory: &NewMemory, anchors: &[MemoryAnchor]) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO memories (repo_id, kind, content, salience, surprise, cost, explicit_weight)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                memory.repo_id,
                memory.kind,
                memory.content,
                memory.salience,
                memory.surprise,
                memory.cost,
                memory.explicit_weight
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

        let mut fetch = self.conn.prepare(
            "SELECT id, kind, content, salience, status, last_access, access_count, helpful_count
             FROM memories
             WHERE id = ?1 AND status != 'archived' AND superseded_by IS NULL
               AND (repo_id IS NULL OR repo_id = ?2)",
        )?;
        let mut rows = Vec::new();
        for (memory_id, distance) in near {
            let row = fetch
                .query_row(params![memory_id, repo_id], |row| {
                    Ok(MemoryRow {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        content: row.get(2)?,
                        salience: row.get(3)?,
                        status: row.get(4)?,
                        last_access: row.get(5)?,
                        access_count: row.get(6)?,
                        helpful_count: row.get(7)?,
                    })
                })
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
                "UPDATE memories SET salience = salience * 0.8 WHERE id = ?1",
                [id],
            )?
        };
        Ok(changed > 0)
    }

    /// Anchored memories go stale the moment their anchored file's content
    /// hash changes; sync calls this on every committed file.
    pub fn mark_stale_anchors(&self, repo_id: i64, relpath: &str, new_hash: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE memories SET status = 'stale'
             WHERE status = 'live' AND id IN (
                 SELECT memory_id FROM memory_anchors
                 WHERE repo_id = ?1 AND relpath = ?2 AND xxh64 != ?3
             )",
            params![repo_id, relpath, new_hash],
        )?;
        Ok(changed)
    }

    pub fn supersede_memory(&self, old_id: i64, new_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE memories SET superseded_by = ?2 WHERE id = ?1",
            params![old_id, new_id],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO memory_links (memory_id, related_id, relation)
             VALUES (?2, ?1, 'supersedes')",
            params![old_id, new_id],
        )?;
        Ok(())
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
