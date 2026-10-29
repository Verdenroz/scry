use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use super::memories::{ROW_COLUMNS, memory_row, scope_partition};
use super::{MemoryRow, Store, vector_bytes};
use crate::{Error, Result};

impl Store {
    pub fn memory(&self, id: i64) -> Result<Option<MemoryRow>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {ROW_COLUMNS} FROM memories WHERE id = ?1"),
                [id],
                memory_row,
            )
            .optional()?)
    }

    /// The row of an archived, non-superseded memory: the one that
    /// [`Store::restore_memory`] accepts.
    pub fn archived_memory(&self, id: i64) -> Result<MemoryRow> {
        self.memory(id)?
            .filter(|row| row.status == "archived" && row.superseded_by.is_none())
            .ok_or_else(|| Error::Config(format!("memory #{id} is not archived or is superseded")))
    }

    /// Stale, non-superseded memories in scope with their anchored paths;
    /// `files` narrows to memories anchored to at least one of them.
    pub fn stale_memories(
        &self,
        repo_id: Option<i64>,
        files: &[String],
    ) -> Result<Vec<(MemoryRow, Vec<String>)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM memories
             WHERE status = 'stale' AND superseded_by IS NULL
               AND (repo_id IS NULL OR repo_id = ?1)
             ORDER BY stale_at DESC, id"
        ))?;
        let rows: Vec<MemoryRow> = stmt
            .query_map([repo_id], memory_row)?
            .collect::<std::result::Result<_, _>>()?;
        let mut paths = self.conn.prepare(
            "SELECT DISTINCT relpath FROM memory_anchors WHERE memory_id = ?1 ORDER BY relpath",
        )?;
        let mut stale = Vec::new();
        for row in rows {
            let relpaths: Vec<String> = paths
                .query_map([row.id], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            if files.is_empty() || relpaths.iter().any(|p| files.contains(p)) {
                stale.push((row, relpaths));
            }
        }
        Ok(stale)
    }

    /// Re-hashes a stale memory's anchors to the current indexed files and
    /// marks it live. Fails, changing nothing, unless the memory is stale,
    /// not superseded, and every anchor still resolves to an indexed file.
    pub fn reaffirm_memory(&mut self, id: i64, evidence: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let (status, superseded): (String, Option<i64>) = tx
            .query_row(
                "SELECT status, superseded_by FROM memories WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::Config(format!("no memory #{id}")))?;
        if status != "stale" || superseded.is_some() {
            return Err(Error::Config(format!(
                "memory #{id} is not stale or is superseded"
            )));
        }
        let missing: Option<String> = tx
            .query_row(
                "SELECT a.relpath FROM memory_anchors a
                 LEFT JOIN files f ON f.repo_id = a.repo_id AND f.relpath = a.relpath
                 WHERE a.memory_id = ?1 AND f.id IS NULL LIMIT 1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(relpath) = missing {
            return Err(Error::Config(format!(
                "anchor {relpath} of memory #{id} is not in the index; sync first"
            )));
        }
        tx.execute(
            "UPDATE memory_anchors SET xxh64 = (
                 SELECT f.xxh64 FROM files f
                 WHERE f.repo_id = memory_anchors.repo_id AND f.relpath = memory_anchors.relpath)
             WHERE memory_id = ?1",
            [id],
        )?;
        tx.execute(
            "UPDATE memories SET status = 'live', stale_at = NULL,
                 evidence = CASE WHEN evidence IS NULL THEN ?2
                                 ELSE evidence || char(10) || ?2 END
             WHERE id = ?1",
            params![id, evidence],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Non-archived memories, for the gc rules.
    pub fn archivable_candidates(&self) -> Result<Vec<MemoryRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM memories WHERE status != 'archived'"
        ))?;
        let rows = stmt
            .query_map([], memory_row)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Distinct scopes that have suppressed each memory.
    pub fn suppression_scope_counts(&self) -> Result<HashMap<i64, i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT memory_id, count(*) FROM memory_suppressions GROUP BY memory_id")?;
        let counts = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(counts)
    }

    /// Archives memories: the text stays, the vector goes.
    pub fn archive_memories(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE memories SET status = 'archived' WHERE id = ?1",
                [id],
            )?;
            tx.execute("DELETE FROM vec_memories WHERE memory_id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Re-inserts an archived memory's vector and returns its new status:
    /// stale when it was archived stale or an anchored file has changed
    /// since, else live. Noise votes, suppressions, and the recency clock
    /// reset so gc does not archive it again at once.
    pub fn restore_memory(&mut self, id: i64, embedding: &[f32]) -> Result<String> {
        let tx = self.conn.transaction()?;
        let (repo_id, status, stale_at, superseded): (
            Option<i64>,
            String,
            Option<i64>,
            Option<i64>,
        ) = tx
            .query_row(
                "SELECT repo_id, status, stale_at, superseded_by FROM memories WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| Error::Config(format!("no memory #{id}")))?;
        if status != "archived" || superseded.is_some() {
            return Err(Error::Config(format!(
                "memory #{id} is not archived or is superseded"
            )));
        }
        let drifted: i64 = tx.query_row(
            "SELECT count(*) FROM memory_anchors a
             LEFT JOIN files f ON f.repo_id = a.repo_id AND f.relpath = a.relpath
             WHERE a.memory_id = ?1 AND (f.id IS NULL OR f.xxh64 != a.xxh64)",
            [id],
            |row| row.get(0),
        )?;
        let restored = if stale_at.is_some() || drifted > 0 {
            "stale"
        } else {
            "live"
        };
        tx.execute(
            "INSERT INTO vec_memories (memory_id, repo_id, embedding) VALUES (?1, ?2, ?3)",
            params![id, scope_partition(repo_id), vector_bytes(embedding)],
        )?;
        tx.execute(
            "UPDATE memories SET status = ?2, noise_count = 0, last_access = unixepoch(),
                 stale_at = CASE WHEN ?2 = 'stale' THEN unixepoch() END
             WHERE id = ?1",
            params![id, restored],
        )?;
        tx.execute("DELETE FROM memory_suppressions WHERE memory_id = ?1", [id])?;
        tx.commit()?;
        Ok(restored.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MemoryAnchor, NewMemory};

    fn memory(repo_id: Option<i64>, seed: f32) -> NewMemory {
        NewMemory {
            repo_id,
            kind: "lesson".to_string(),
            content: format!("memory {seed}"),
            salience: 0.5,
            surprise: 0.0,
            cost: 0.0,
            explicit_weight: 0.0,
            source: "agent".to_string(),
            evidence: Some("first".to_string()),
            embedding: vec![seed, 1.0, 0.0, 0.0],
        }
    }

    fn anchored(store: &mut Store, repo: i64, relpath: &str) -> i64 {
        store.upsert_file(repo, relpath, "old", 1, 0).unwrap();
        let anchor = MemoryAnchor {
            relpath: relpath.to_string(),
            start_line: None,
            end_line: None,
            xxh64: "old".to_string(),
        };
        store
            .add_memory(&memory(Some(repo), 0.1), &[anchor])
            .unwrap()
    }

    #[test]
    fn reaffirm_rehashes_anchors_and_appends_evidence() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let repo = store.upsert_repo("github.com/x/y").unwrap();
        let id = anchored(&mut store, repo, "a.rs");
        assert!(store.reaffirm_memory(id, "still true").is_err());

        store.upsert_file(repo, "a.rs", "new", 1, 0).unwrap();
        store.mark_stale_anchors(repo, "a.rs", "new").unwrap();
        store.reaffirm_memory(id, "still true").unwrap();

        let row = store.memory(id).unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.stale_at), ("live", None));
        let (hash, evidence): (String, String) = store
            .conn
            .query_row(
                "SELECT a.xxh64, m.evidence FROM memories m
                 JOIN memory_anchors a ON a.memory_id = m.id WHERE m.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(hash, "new");
        assert_eq!(evidence, "first\nstill true");
        assert_eq!(store.mark_stale_anchors(repo, "a.rs", "new").unwrap(), 0);
    }

    #[test]
    fn reaffirm_refuses_unindexed_anchors_and_superseded_rows() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let repo = store.upsert_repo("github.com/x/y").unwrap();
        let id = anchored(&mut store, repo, "a.rs");
        store.mark_stale_anchors(repo, "a.rs", "new").unwrap();
        store.delete_file(repo, "a.rs").unwrap();
        let err = store.reaffirm_memory(id, "e").unwrap_err();
        assert!(err.to_string().contains("not in the index"));

        store.upsert_file(repo, "a.rs", "new", 1, 0).unwrap();
        let newer = store.add_memory(&memory(Some(repo), 0.2), &[]).unwrap();
        store.supersede_memory(id, newer).unwrap();
        assert!(store.reaffirm_memory(id, "e").is_err());
        assert_eq!(store.memory(id).unwrap().unwrap().status, "stale");
    }

    #[test]
    fn stale_list_filters_by_anchored_file() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let repo = store.upsert_repo("github.com/x/y").unwrap();
        anchored(&mut store, repo, "a.rs");
        let b = anchored(&mut store, repo, "b.rs");
        store.mark_stale_anchors(repo, "a.rs", "new").unwrap();
        store.mark_stale_anchors(repo, "b.rs", "new").unwrap();

        assert_eq!(store.stale_memories(Some(repo), &[]).unwrap().len(), 2);
        let only_b = store
            .stale_memories(Some(repo), &["b.rs".to_string()])
            .unwrap();
        assert_eq!(only_b.len(), 1);
        assert_eq!(
            (only_b[0].0.id, only_b[0].1.clone()),
            (b, vec!["b.rs".to_string()])
        );
        assert!(store.stale_memories(None, &[]).unwrap().is_empty());
    }

    #[test]
    fn archive_drops_the_vector_and_restore_brings_it_back() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let repo = store.upsert_repo("github.com/x/y").unwrap();
        let id = store.add_memory(&memory(Some(repo), 0.1), &[]).unwrap();
        let query = [0.1, 1.0, 0.0, 0.0];
        store.memory_feedback(id, false, Some(repo)).unwrap();

        store.archive_memories(&[id]).unwrap();
        assert!(
            store
                .memory_candidates(Some(repo), &query, 5)
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.memory(id).unwrap().unwrap().status, "archived");

        assert_eq!(store.restore_memory(id, &query).unwrap(), "live");
        assert_eq!(
            store
                .memory_candidates(Some(repo), &query, 5)
                .unwrap()
                .len(),
            1
        );
        assert!(store.restore_memory(id, &query).is_err());
    }

    #[test]
    fn restore_returns_stale_for_stale_archives_and_drifted_anchors() {
        let mut store = Store::open_in_memory("m", 4).unwrap();
        let repo = store.upsert_repo("github.com/x/y").unwrap();
        let query = [0.1, 1.0, 0.0, 0.0];
        let was_stale = anchored(&mut store, repo, "a.rs");
        store.mark_stale_anchors(repo, "a.rs", "new").unwrap();
        let drifted = anchored(&mut store, repo, "b.rs");
        store.archive_memories(&[was_stale, drifted]).unwrap();
        store.upsert_file(repo, "b.rs", "changed", 1, 0).unwrap();

        assert_eq!(store.restore_memory(was_stale, &query).unwrap(), "stale");
        assert_eq!(store.restore_memory(drifted, &query).unwrap(), "stale");
        assert!(store.memory(drifted).unwrap().unwrap().stale_at.is_some());
    }
}
