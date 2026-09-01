use rusqlite::params;

use super::{Store, vector_bytes};
use crate::Result;

/// Below this many chunks in a repo, the float table is scanned directly;
/// above it, a binary coarse pass over `vec_chunks_bit` runs first and the
/// float table only rescores the survivors.
const BINARY_COARSE_THRESHOLD: i64 = 8192;
const COARSE_FACTOR: usize = 8;

#[derive(Debug, Clone)]
pub struct NewChunk {
    pub start_line: u32,
    pub end_line: u32,
    pub symbol: Option<String>,
    pub content: String,
    pub content_hash: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct ChunkRow {
    pub id: i64,
    pub relpath: String,
    pub start_line: u32,
    pub end_line: u32,
    pub symbol: Option<String>,
    pub content: String,
    pub file_mtime: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct DenseHit {
    pub chunk_id: i64,
    pub distance: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct LexicalHit {
    pub chunk_id: i64,
    pub rank: f64,
}

fn bytes_to_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        dot += f64::from(*x) * f64::from(*y);
        na += f64::from(*x) * f64::from(*x);
        nb += f64::from(*y) * f64::from(*y);
    }
    if na == 0.0 || nb == 0.0 {
        return 1.0;
    }
    1.0 - dot / (na.sqrt() * nb.sqrt())
}

impl Store {
    pub fn vector_for_hash(&self, content_hash: &str) -> Result<Option<Vec<f32>>> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT v.embedding FROM vec_chunks v
                 JOIN chunks c ON c.id = v.chunk_id
                 WHERE c.content_hash = ?1 LIMIT 1",
                [content_hash],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })?;
        Ok(bytes.map(|b| bytes_to_vector(&b)))
    }

    pub fn replace_file_chunks(
        &mut self,
        file_id: i64,
        repo_id: i64,
        chunks: &[NewChunk],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM vec_chunks WHERE chunk_id IN
             (SELECT id FROM chunks WHERE file_id = ?1)",
            [file_id],
        )?;
        tx.execute(
            "DELETE FROM vec_chunks_bit WHERE chunk_id IN
             (SELECT id FROM chunks WHERE file_id = ?1)",
            [file_id],
        )?;
        tx.execute("DELETE FROM chunks WHERE file_id = ?1", [file_id])?;
        for chunk in chunks {
            tx.execute(
                "INSERT INTO chunks (file_id, start_line, end_line, symbol, content, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    file_id,
                    chunk.start_line,
                    chunk.end_line,
                    chunk.symbol,
                    chunk.content,
                    chunk.content_hash
                ],
            )?;
            let chunk_id = tx.last_insert_rowid();
            let bytes = vector_bytes(&chunk.embedding);
            tx.execute(
                "INSERT INTO vec_chunks (chunk_id, repo_id, embedding) VALUES (?1, ?2, ?3)",
                params![chunk_id, repo_id, bytes],
            )?;
            tx.execute(
                "INSERT INTO vec_chunks_bit (chunk_id, repo_id, embedding)
                 VALUES (?1, ?2, vec_quantize_binary(?3))",
                params![chunk_id, repo_id, bytes],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn dense_search(&self, repo_id: i64, query: &[f32], k: usize) -> Result<Vec<DenseHit>> {
        let chunk_count: i64 = self.conn.query_row(
            "SELECT count(*) FROM chunks c JOIN files f ON f.id = c.file_id WHERE f.repo_id = ?1",
            [repo_id],
            |row| row.get(0),
        )?;
        if chunk_count > BINARY_COARSE_THRESHOLD {
            self.dense_search_two_stage(repo_id, query, k)
        } else {
            self.dense_search_flat(repo_id, query, k)
        }
    }

    fn dense_search_flat(&self, repo_id: i64, query: &[f32], k: usize) -> Result<Vec<DenseHit>> {
        let mut stmt = self.conn.prepare(
            "SELECT chunk_id, distance FROM vec_chunks
             WHERE embedding MATCH ?1 AND repo_id = ?2 AND k = ?3
             ORDER BY distance",
        )?;
        let rows = stmt.query_map(params![vector_bytes(query), repo_id, k as i64], |row| {
            Ok(DenseHit {
                chunk_id: row.get(0)?,
                distance: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn dense_search_two_stage(
        &self,
        repo_id: i64,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<DenseHit>> {
        let coarse_k = (k * COARSE_FACTOR).max(200);
        let mut stmt = self.conn.prepare(
            "SELECT chunk_id FROM vec_chunks_bit
             WHERE embedding MATCH vec_quantize_binary(?1) AND repo_id = ?2 AND k = ?3",
        )?;
        let candidates: Vec<i64> = stmt
            .query_map(
                params![vector_bytes(query), repo_id, coarse_k as i64],
                |row| row.get(0),
            )?
            .collect::<std::result::Result<_, _>>()?;

        let mut fetch = self
            .conn
            .prepare("SELECT embedding FROM vec_chunks WHERE chunk_id = ?1")?;
        let mut hits: Vec<DenseHit> = candidates
            .into_iter()
            .map(|chunk_id| {
                let bytes: Vec<u8> = fetch.query_row([chunk_id], |row| row.get(0))?;
                Ok(DenseHit {
                    chunk_id,
                    distance: cosine_distance(query, &bytes_to_vector(&bytes)),
                })
            })
            .collect::<Result<_>>()?;
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        hits.truncate(k);
        Ok(hits)
    }

    pub fn lexical_search(
        &self,
        repo_id: i64,
        fts_query: &str,
        k: usize,
        path_prefix: Option<&str>,
    ) -> Result<Vec<LexicalHit>> {
        let prefix_pattern = path_prefix.map(|p| format!("{}%", p.trim_end_matches('/')));
        let mut stmt = self.conn.prepare(
            "SELECT c.id, chunks_fts.rank FROM chunks_fts
             JOIN chunks c ON c.id = chunks_fts.rowid
             JOIN files f ON f.id = c.file_id
             WHERE chunks_fts MATCH ?1 AND f.repo_id = ?2
               AND (?3 IS NULL OR f.relpath LIKE ?3)
             ORDER BY chunks_fts.rank LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![fts_query, repo_id, prefix_pattern, k as i64],
            |row| {
                Ok(LexicalHit {
                    chunk_id: row.get(0)?,
                    rank: row.get(1)?,
                })
            },
        )?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn hydrate_chunks(&self, chunk_ids: &[i64]) -> Result<Vec<ChunkRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, f.relpath, c.start_line, c.end_line, c.symbol, c.content, f.mtime
             FROM chunks c JOIN files f ON f.id = c.file_id
             WHERE c.id = ?1",
        )?;
        let mut rows = Vec::with_capacity(chunk_ids.len());
        for chunk_id in chunk_ids {
            let row = stmt.query_row([chunk_id], |row| {
                Ok(ChunkRow {
                    id: row.get(0)?,
                    relpath: row.get(1)?,
                    start_line: row.get(2)?,
                    end_line: row.get(3)?,
                    symbol: row.get(4)?,
                    content: row.get(5)?,
                    file_mtime: row.get(6)?,
                })
            })?;
            rows.push(row);
        }
        Ok(rows)
    }

    pub fn symbols(&self, repo_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT c.symbol FROM chunks c
             JOIN files f ON f.id = c.file_id
             WHERE f.repo_id = ?1 AND c.symbol IS NOT NULL",
        )?;
        let rows = stmt.query_map([repo_id], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}
