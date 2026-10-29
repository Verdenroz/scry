//! SQLite store: one file holds repos, files, chunks, FTS5, sqlite-vec
//! vectors, and memories. Vector tables are created at open time because
//! their column type carries the embedding dimension.

mod chunks;
mod files;
mod memories;

pub use chunks::{ChunkRow, DenseHit, LexicalHit, NewChunk};
pub use files::StoredFile;
pub use memories::{MemoryAnchor, MemoryRow, NewMemory, SourceStats};

use std::path::Path;
use std::sync::Once;

use rusqlite::Connection;
use rusqlite::ffi::sqlite3_auto_extension;

use crate::{Error, Result};

const SCHEMA: &str = include_str!("schema.sql");
const SCHEMA_VERSION: u32 = 5;

static VEC_EXTENSION: Once = Once::new();

fn register_vec_extension() {
    VEC_EXTENSION.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

pub struct Store {
    pub(crate) conn: Connection,
}

impl Store {
    /// Opens or creates the database, guarding against a mismatched
    /// embedding model or dimension: vectors from different models share
    /// no space, so a mismatch requires a new db or a full reindex.
    pub fn open(path: &Path, embedding_model: &str, dim: usize) -> Result<Self> {
        register_vec_extension();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn, embedding_model, dim)
    }

    pub fn open_in_memory(embedding_model: &str, dim: usize) -> Result<Self> {
        register_vec_extension();
        Self::init(Connection::open_in_memory()?, embedding_model, dim)
    }

    fn init(conn: Connection, embedding_model: &str, dim: usize) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "cache_size", -65536)?;
        conn.pragma_update(None, "mmap_size", 268_435_456)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let initialized: bool = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)?;

        if !initialized {
            conn.execute_batch(SCHEMA)?;
            conn.execute_batch(&format!(
                "CREATE VIRTUAL TABLE vec_chunks_bit USING vec0(
                     chunk_id INTEGER PRIMARY KEY,
                     repo_id INTEGER PARTITION KEY,
                     embedding bit[{dim}]
                 );
                 CREATE VIRTUAL TABLE vec_memories USING vec0(
                     memory_id INTEGER PRIMARY KEY,
                     repo_id INTEGER PARTITION KEY,
                     embedding float[{dim}] distance_metric=cosine
                 );"
            ))?;
            conn.execute(
                "INSERT INTO meta (key, value) VALUES
                 ('schema_version', ?3), ('embedding_model', ?1), ('embedding_dim', ?2)",
                rusqlite::params![embedding_model, dim.to_string(), SCHEMA_VERSION.to_string()],
            )?;
        }
        let store = Self { conn };
        store.guard_meta("embedding_model", embedding_model)?;
        store.guard_meta("embedding_dim", &dim.to_string())?;
        migrate(&store.conn, dim)?;
        Ok(store)
    }

    /// Merges the FTS b-trees and refreshes planner statistics; cheap
    /// enough to run at shutdown and after a prune.
    pub fn optimize(&self) -> Result<()> {
        self.conn.execute_batch(
            "INSERT INTO chunks_fts(chunks_fts) VALUES('optimize');
             PRAGMA optimize;",
        )?;
        Ok(())
    }

    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }

    fn guard_meta(&self, key: &str, expected: &str) -> Result<()> {
        let stored: String =
            self.conn
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                    row.get(0)
                })?;
        if stored != expected {
            return Err(Error::Config(format!(
                "db has {key} = {stored}, config says {expected}; \
                 use a different db_path or reindex from scratch"
            )));
        }
        Ok(())
    }

    pub fn upsert_repo(&self, key: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO repos (key) VALUES (?1) ON CONFLICT (key) DO NOTHING",
            [key],
        )?;
        let id = self
            .conn
            .query_row("SELECT id FROM repos WHERE key = ?1", [key], |row| {
                row.get(0)
            })?;
        Ok(id)
    }

    pub fn repo_id(&self, key: &str) -> Result<Option<i64>> {
        let id = self
            .conn
            .query_row("SELECT id FROM repos WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })?;
        Ok(id)
    }

    pub fn counts(&self) -> Result<(i64, i64, i64)> {
        let repos = self.scalar("SELECT count(*) FROM repos")?;
        let files = self.scalar("SELECT count(*) FROM files")?;
        let chunks = self.scalar("SELECT count(*) FROM chunks")?;
        Ok((repos, files, chunks))
    }

    fn scalar(&self, sql: &str) -> Result<i64> {
        Ok(self.conn.query_row(sql, [], |row| row.get(0))?)
    }
}

/// v1 -> v2: the FTS index gains a relpath column so path words rank in
/// BM25; rebuilt in place from chunks + files, vectors untouched.
/// v2 -> v3: float vectors move from vec0 into the plain `chunk_vectors`
/// table and repos gain a maintained chunk_count.
/// v3 -> v4: memories gain provenance, a staleness timestamp, and a noise
/// count.
/// v4 -> v5: memory vectors gain a scope partition (0 for globals),
/// per-scope suppressions get a table, and legacy stale rows get a
/// `stale_at`.
fn migrate(conn: &Connection, dim: usize) -> Result<()> {
    let version: String = conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    )?;
    let version: u32 = version
        .parse()
        .map_err(|_| Error::Config(format!("db schema_version {version} is not a number")))?;
    if version > SCHEMA_VERSION {
        return Err(Error::Config(format!(
            "db schema_version {version} is newer than this binary; upgrade scry"
        )));
    }
    if version < 2 {
        conn.execute_batch(MIGRATE_V1_TO_V2)?;
    }
    if version < 3 {
        conn.execute_batch(MIGRATE_V2_TO_V3)?;
    }
    if version < 4 {
        conn.execute_batch(MIGRATE_V3_TO_V4)?;
    }
    if version < 5 {
        conn.execute_batch(&migrate_v4_to_v5(dim))?;
    }
    Ok(())
}

const MIGRATE_V1_TO_V2: &str = "BEGIN;
    DROP TRIGGER IF EXISTS chunks_ai;
    DROP TRIGGER IF EXISTS chunks_ad;
    DROP TABLE IF EXISTS chunks_fts;
    CREATE VIRTUAL TABLE chunks_fts USING fts5(
        content, symbol, relpath,
        content='', contentless_delete=1
    );
    CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
        INSERT INTO chunks_fts(rowid, content, symbol, relpath)
        VALUES (new.id, new.content, new.symbol,
                (SELECT relpath FROM files WHERE id = new.file_id));
    END;
    CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
        DELETE FROM chunks_fts WHERE rowid = old.id;
    END;
    INSERT INTO chunks_fts(rowid, content, symbol, relpath)
        SELECT c.id, c.content, c.symbol, f.relpath
        FROM chunks c JOIN files f ON f.id = c.file_id;
    UPDATE meta SET value = '2' WHERE key = 'schema_version';
    COMMIT;";

const MIGRATE_V2_TO_V3: &str = "BEGIN;
    CREATE TABLE chunk_vectors (
        chunk_id INTEGER PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
        embedding BLOB NOT NULL
    );
    INSERT INTO chunk_vectors (chunk_id, embedding)
        SELECT chunk_id, embedding FROM vec_chunks;
    DROP TABLE vec_chunks;
    ALTER TABLE repos ADD COLUMN chunk_count INTEGER NOT NULL DEFAULT 0;
    UPDATE repos SET chunk_count = (
        SELECT count(*) FROM chunks c JOIN files f ON f.id = c.file_id
        WHERE f.repo_id = repos.id);
    CREATE TRIGGER chunks_count_ai AFTER INSERT ON chunks BEGIN
        UPDATE repos SET chunk_count = chunk_count + 1
        WHERE id = (SELECT repo_id FROM files WHERE id = new.file_id);
    END;
    CREATE TRIGGER chunks_count_ad AFTER DELETE ON chunks BEGIN
        UPDATE repos SET chunk_count = chunk_count - 1
        WHERE id = (SELECT repo_id FROM files WHERE id = old.file_id);
    END;
    UPDATE meta SET value = '3' WHERE key = 'schema_version';
    COMMIT;";

const MIGRATE_V3_TO_V4: &str = "BEGIN;
    ALTER TABLE memories ADD COLUMN source TEXT NOT NULL DEFAULT 'agent'
        CHECK (source IN ('agent', 'capture'));
    ALTER TABLE memories ADD COLUMN evidence TEXT;
    ALTER TABLE memories ADD COLUMN stale_at INTEGER;
    ALTER TABLE memories ADD COLUMN noise_count INTEGER NOT NULL DEFAULT 0;
    UPDATE meta SET value = '4' WHERE key = 'schema_version';
    COMMIT;";

fn migrate_v4_to_v5(dim: usize) -> String {
    format!(
        "BEGIN;
        UPDATE memories SET stale_at = unixepoch()
            WHERE status = 'stale' AND stale_at IS NULL;
        CREATE TABLE memory_suppressions (
            memory_id INTEGER NOT NULL REFERENCES memories(id),
            repo_id INTEGER NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            UNIQUE (memory_id, repo_id)
        );
        CREATE TEMP TABLE memory_vectors_old AS
            SELECT memory_id, embedding FROM vec_memories;
        DROP TABLE vec_memories;
        CREATE VIRTUAL TABLE vec_memories USING vec0(
            memory_id INTEGER PRIMARY KEY,
            repo_id INTEGER PARTITION KEY,
            embedding float[{dim}] distance_metric=cosine
        );
        INSERT INTO vec_memories (memory_id, repo_id, embedding)
            SELECT o.memory_id, coalesce(m.repo_id, 0), o.embedding
            FROM memory_vectors_old o JOIN memories m ON m.id = o.memory_id
            WHERE m.status != 'archived';
        DROP TABLE memory_vectors_old;
        UPDATE meta SET value = '5' WHERE key = 'schema_version';
        COMMIT;"
    )
}

pub(crate) fn vector_bytes(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_extension_loads() {
        let store = Store::open_in_memory("test-model", 4).unwrap();
        let version: String = store
            .conn
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .unwrap();
        assert!(version.starts_with('v'));
    }

    #[test]
    fn rejects_mismatched_embedding_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        Store::open(&path, "model-a", 4).unwrap();
        let err = Store::open(&path, "model-b", 4).err().unwrap();
        assert!(err.to_string().contains("embedding_model"));
    }

    #[test]
    fn migrates_v1_through_v5() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        {
            let mut store = Store::open(&path, "m", 8).unwrap();
            let repo_id = store.upsert_repo("github.com/x/y").unwrap();
            let file_id = store
                .upsert_file(repo_id, "src/domains/crypto.rs", "abcd", 4, 0)
                .unwrap();
            store
                .replace_file_chunks(
                    file_id,
                    repo_id,
                    &[crate::store::NewChunk {
                        start_line: 1,
                        end_line: 2,
                        symbol: Some("get_quotes".to_string()),
                        content: "pub fn get_quotes() {}".to_string(),
                        content_hash: "h1".to_string(),
                        embedding: vec![0.1; 8],
                    }],
                )
                .unwrap();
            store
                .conn
                .execute_batch(
                    "DROP TRIGGER chunks_ai; DROP TRIGGER chunks_ad;
                     DROP TABLE chunks_fts;
                     CREATE VIRTUAL TABLE chunks_fts USING fts5(
                         content, symbol, content='chunks', content_rowid='id');
                     INSERT INTO chunks_fts(rowid, content, symbol)
                         SELECT id, content, symbol FROM chunks;
                     CREATE VIRTUAL TABLE vec_chunks USING vec0(
                         chunk_id INTEGER PRIMARY KEY,
                         repo_id INTEGER PARTITION KEY,
                         embedding float[8] distance_metric=cosine);
                     INSERT INTO vec_chunks (chunk_id, repo_id, embedding)
                         SELECT v.chunk_id, 1, v.embedding FROM chunk_vectors v;
                     DROP TABLE chunk_vectors;
                     DROP TRIGGER chunks_count_ai; DROP TRIGGER chunks_count_ad;
                     ALTER TABLE repos DROP COLUMN chunk_count;
                     ALTER TABLE memories DROP COLUMN source;
                     ALTER TABLE memories DROP COLUMN evidence;
                     ALTER TABLE memories DROP COLUMN stale_at;
                     ALTER TABLE memories DROP COLUMN noise_count;
                     DROP TABLE memory_suppressions;
                     DROP TABLE vec_memories;
                     CREATE VIRTUAL TABLE vec_memories USING vec0(
                         memory_id INTEGER PRIMARY KEY,
                         embedding float[8] distance_metric=cosine);
                     INSERT INTO memories (kind, content) VALUES ('lesson', 'old');
                     INSERT INTO memories (kind, content, status)
                         VALUES ('lesson', 'legacy stale', 'stale');
                     UPDATE memories SET repo_id = 1 WHERE content = 'legacy stale';
                     INSERT INTO memories (kind, content, status)
                         VALUES ('lesson', 'archived', 'archived');
                     UPDATE meta SET value = '1' WHERE key = 'schema_version';",
                )
                .unwrap();
            for id in [1, 2, 3] {
                store
                    .conn
                    .execute(
                        "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)",
                        rusqlite::params![id, vector_bytes(&[0.5; 8])],
                    )
                    .unwrap();
            }
        }
        let store = Store::open(&path, "m", 8).unwrap();
        let version: String = store
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "5");
        let hits = store.lexical_search(None, "\"domains\"", 5, None).unwrap();
        assert_eq!(hits.len(), 1);
        let copied: i64 = store
            .conn
            .query_row("SELECT count(*) FROM chunk_vectors", [], |row| row.get(0))
            .unwrap();
        assert_eq!(copied, 1);
        let count: i64 = store
            .conn
            .query_row("SELECT chunk_count FROM repos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let (source, noise): (String, i64) = store
            .conn
            .query_row("SELECT source, noise_count FROM memories", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!((source.as_str(), noise), ("agent", 0));

        let stale_at: Option<i64> = store
            .conn
            .query_row(
                "SELECT stale_at FROM memories WHERE status = 'stale'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stale_at.is_some());
        let partitions: Vec<(i64, i64)> = store
            .conn
            .prepare("SELECT memory_id, repo_id FROM vec_memories ORDER BY memory_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(
            partitions,
            vec![(1, 0), (2, 1)],
            "archived row keeps no vector"
        );
        let suppressions: i64 = store
            .conn
            .query_row("SELECT count(*) FROM memory_suppressions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(suppressions, 0);
    }

    #[test]
    fn upsert_repo_is_idempotent() {
        let store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/y").unwrap();
        let b = store.upsert_repo("github.com/x/y").unwrap();
        assert_eq!(a, b);
        assert_eq!(store.repo_id("github.com/x/y").unwrap(), Some(a));
        assert_eq!(store.repo_id("github.com/x/z").unwrap(), None);
    }
}
