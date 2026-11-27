//! Whole-store vector replacement for a change of embedding model.

use std::path::Path;

use rusqlite::{Connection, params};

use super::{Store, register_vec_extension, vec_memories_ddl, vector_bytes};
use crate::{Error, Result};

/// The stored parts a chunk's embedding input is built from.
#[derive(Debug, Clone)]
pub struct ChunkSource {
    pub chunk_id: i64,
    pub repo_id: i64,
    pub repo_key: String,
    pub relpath: String,
    pub symbol: Option<String>,
    pub content: String,
}

/// A memory that currently has a vector; archived memories have none.
#[derive(Debug, Clone)]
pub struct MemorySource {
    pub memory_id: i64,
    pub repo_id: i64,
    pub kind: String,
    pub content: String,
}

impl Store {
    /// Opens an existing db whatever model stamped it, without migrating
    /// its schema. Only `reembed` may use this: everything else must go
    /// through the guarded `open`.
    pub fn open_for_reembed(path: &Path) -> Result<Self> {
        register_vec_extension();
        if !path.exists() {
            return Err(Error::Config(format!("no scry db at {}", path.display())));
        }
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        Ok(Self { conn })
    }

    /// Brings a db opened by `open_for_reembed` up to the current schema.
    pub fn migrate_schema(&self) -> Result<()> {
        let (_, dim) = self.embedding_meta()?;
        super::migrate(&self.conn, dim)
    }

    pub fn embedding_meta(&self) -> Result<(String, usize)> {
        let read = |key: &str| -> Result<String> {
            Ok(self
                .conn
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                    row.get(0)
                })?)
        };
        let dim = read("embedding_dim")?
            .parse()
            .map_err(|_| Error::Config("db has a non-numeric embedding_dim".to_string()))?;
        Ok((read("embedding_model")?, dim))
    }

    /// Writes a consistent copy of the db; fails rather than overwrite an
    /// existing file.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(Error::Config(format!(
                "backup {} already exists",
                path.display()
            )));
        }
        self.conn
            .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
        Ok(())
    }

    pub fn chunk_sources(&self) -> Result<Vec<ChunkSource>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, f.repo_id, r.key, f.relpath, c.symbol, c.content
             FROM chunks c JOIN files f ON f.id = c.file_id JOIN repos r ON r.id = f.repo_id
             ORDER BY c.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ChunkSource {
                chunk_id: row.get(0)?,
                repo_id: row.get(1)?,
                repo_key: row.get(2)?,
                relpath: row.get(3)?,
                symbol: row.get(4)?,
                content: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn memory_sources(&self) -> Result<Vec<MemorySource>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.memory_id, v.repo_id, m.kind, m.content
             FROM vec_memories v JOIN memories m ON m.id = v.memory_id
             ORDER BY v.memory_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(MemorySource {
                memory_id: row.get(0)?,
                repo_id: row.get(1)?,
                kind: row.get(2)?,
                content: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Swaps every chunk and memory vector and restamps the model and
    /// document prefix in one transaction. Fails without writing when the
    /// chunks or memory vectors differ from the `sources` read earlier.
    pub fn replace_all_vectors(
        &mut self,
        embedding_model: &str,
        dim: usize,
        document_prefix: &str,
        chunks: &[(ChunkSource, Vec<f32>)],
        memories: &[(MemorySource, Vec<f32>)],
    ) -> Result<()> {
        let lengths = chunks.iter().map(|(_, v)| v.len());
        let mut lengths = lengths.chain(memories.iter().map(|(_, v)| v.len()));
        if let Some(len) = lengths.find(|len| *len != dim) {
            return Err(Error::Embedding(format!(
                "endpoint returned a {len}-dim vector, config says {dim}"
            )));
        }
        let tx = self.conn.transaction()?;
        let ids = |sql: &str| -> Result<(i64, i64)> {
            Ok(tx.query_row(sql, [], |row| Ok((row.get(0)?, row.get(1)?)))?)
        };
        let read = |ids: &mut dyn Iterator<Item = i64>| {
            ids.fold((0i64, 0i64), |(n, sum), id| (n + 1, sum + id))
        };
        let stored_chunks = ids("SELECT count(*), coalesce(sum(id), 0) FROM chunks")?;
        let stored_memories =
            ids("SELECT count(*), coalesce(sum(memory_id), 0) FROM vec_memories")?;
        if stored_chunks != read(&mut chunks.iter().map(|(c, _)| c.chunk_id))
            || stored_memories != read(&mut memories.iter().map(|(m, _)| m.memory_id))
        {
            return Err(Error::Config(
                "db changed since reembed read it; stop `scry serve` and run reembed again"
                    .to_string(),
            ));
        }
        tx.execute_batch(&format!(
            "DROP TABLE vec_chunks_bit;
             DROP TABLE vec_memories;
             DELETE FROM chunk_vectors;
             CREATE VIRTUAL TABLE vec_chunks_bit USING vec0(
                 chunk_id INTEGER PRIMARY KEY,
                 repo_id INTEGER PARTITION KEY,
                 embedding bit[{dim}]
             );
             {}",
            vec_memories_ddl(dim)
        ))?;
        for (chunk, vector) in chunks {
            let bytes = vector_bytes(vector);
            tx.execute(
                "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)",
                params![chunk.chunk_id, bytes],
            )?;
            tx.execute(
                "INSERT INTO vec_chunks_bit (chunk_id, repo_id, embedding)
                 VALUES (?1, ?2, vec_quantize_binary(?3))",
                params![chunk.chunk_id, chunk.repo_id, bytes],
            )?;
        }
        for (memory, vector) in memories {
            tx.execute(
                "INSERT INTO vec_memories (memory_id, repo_id, embedding) VALUES (?1, ?2, ?3)",
                params![memory.memory_id, memory.repo_id, vector_bytes(vector)],
            )?;
        }
        tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'embedding_model'",
            [embedding_model],
        )?;
        tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'embedding_dim'",
            [dim.to_string()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('embedding_document_prefix', ?1)",
            [document_prefix],
        )?;
        tx.commit()?;
        Ok(())
    }
}
