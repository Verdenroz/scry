//! SQLite store: one file holds repos, files, chunks, FTS5, sqlite-vec
//! vectors, and memories. Vector tables are created at open time because
//! their column type carries the embedding dimension.

mod chunks;
mod files;
mod memories;

pub use chunks::{ChunkRow, DenseHit, LexicalHit, NewChunk};
pub use files::StoredFile;
pub use memories::{MemoryAnchor, MemoryRow, NewMemory};

use std::path::Path;
use std::sync::Once;

use rusqlite::Connection;
use rusqlite::ffi::sqlite3_auto_extension;

use crate::{Error, Result};

const SCHEMA: &str = include_str!("schema.sql");

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
                "CREATE VIRTUAL TABLE vec_chunks USING vec0(
                     chunk_id INTEGER PRIMARY KEY,
                     repo_id INTEGER PARTITION KEY,
                     embedding float[{dim}] distance_metric=cosine
                 );
                 CREATE VIRTUAL TABLE vec_chunks_bit USING vec0(
                     chunk_id INTEGER PRIMARY KEY,
                     repo_id INTEGER PARTITION KEY,
                     embedding bit[{dim}]
                 );
                 CREATE VIRTUAL TABLE vec_memories USING vec0(
                     memory_id INTEGER PRIMARY KEY,
                     embedding float[{dim}] distance_metric=cosine
                 );"
            ))?;
            conn.execute(
                "INSERT INTO meta (key, value) VALUES
                 ('schema_version', '1'), ('embedding_model', ?1), ('embedding_dim', ?2)",
                rusqlite::params![embedding_model, dim.to_string()],
            )?;
        }

        let store = Self { conn };
        store.guard_meta("embedding_model", embedding_model)?;
        store.guard_meta("embedding_dim", &dim.to_string())?;
        Ok(store)
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
    fn upsert_repo_is_idempotent() {
        let store = Store::open_in_memory("m", 4).unwrap();
        let a = store.upsert_repo("github.com/x/y").unwrap();
        let b = store.upsert_repo("github.com/x/y").unwrap();
        assert_eq!(a, b);
        assert_eq!(store.repo_id("github.com/x/y").unwrap(), Some(a));
        assert_eq!(store.repo_id("github.com/x/z").unwrap(), None);
    }
}
