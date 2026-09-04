use std::collections::HashMap;

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
    pub repo_key: String,
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

fn json_id_list(ids: &[i64]) -> String {
    let inner: Vec<String> = ids.iter().map(i64::to_string).collect();
    format!("[{}]", inner.join(","))
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

    pub fn dense_search(
        &self,
        repo_id: Option<i64>,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<DenseHit>> {
        let chunk_count: i64 = self.conn.query_row(
            "SELECT count(*) FROM chunks c JOIN files f ON f.id = c.file_id
             WHERE ?1 IS NULL OR f.repo_id = ?1",
            [repo_id],
            |row| row.get(0),
        )?;
        if chunk_count > BINARY_COARSE_THRESHOLD {
            self.dense_search_coarse(repo_id, query, k, (k * COARSE_FACTOR).max(200))
        } else {
            self.dense_search_exact(repo_id, query, k)
        }
    }

    pub fn dense_search_exact(
        &self,
        repo_id: Option<i64>,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<DenseHit>> {
        // vec0 KNN cannot take an optional partition constraint in one
        // statement; the filter must be present or absent in the SQL.
        let sql = match repo_id {
            Some(_) => {
                "SELECT chunk_id, distance FROM vec_chunks
                 WHERE embedding MATCH ?1 AND k = ?2 AND repo_id = ?3
                 ORDER BY distance"
            }
            None => {
                "SELECT chunk_id, distance FROM vec_chunks
                 WHERE embedding MATCH ?1 AND k = ?2
                 ORDER BY distance"
            }
        };
        let mut stmt = self.conn.prepare(sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            Ok(DenseHit {
                chunk_id: row.get(0)?,
                distance: row.get(1)?,
            })
        };
        let rows = match repo_id {
            Some(repo_id) => {
                stmt.query_map(params![vector_bytes(query), k as i64, repo_id], map)?
            }
            None => stmt.query_map(params![vector_bytes(query), k as i64], map)?,
        };
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Hamming pass over `vec_chunks_bit` keeps `coarse_k` candidates,
    /// then the float table rescores them by cosine and keeps `k`.
    pub fn dense_search_coarse(
        &self,
        repo_id: Option<i64>,
        query: &[f32],
        k: usize,
        coarse_k: usize,
    ) -> Result<Vec<DenseHit>> {
        let sql = match repo_id {
            Some(_) => {
                "SELECT chunk_id FROM vec_chunks_bit
                 WHERE embedding MATCH vec_quantize_binary(?1) AND k = ?2 AND repo_id = ?3"
            }
            None => {
                "SELECT chunk_id FROM vec_chunks_bit
                 WHERE embedding MATCH vec_quantize_binary(?1) AND k = ?2"
            }
        };
        let mut stmt = self.conn.prepare(sql)?;
        let candidates: Vec<i64> = match repo_id {
            Some(repo_id) => stmt
                .query_map(
                    params![vector_bytes(query), coarse_k as i64, repo_id],
                    |row| row.get(0),
                )?
                .collect::<std::result::Result<_, _>>()?,
            None => stmt
                .query_map(params![vector_bytes(query), coarse_k as i64], |row| {
                    row.get(0)
                })?
                .collect::<std::result::Result<_, _>>()?,
        };

        let ids = json_id_list(&candidates);
        let mut rescore = self.conn.prepare(
            "SELECT chunk_id, distance FROM vec_chunks
             WHERE embedding MATCH ?1 AND k = ?2
               AND chunk_id IN (SELECT value FROM json_each(?3))
             ORDER BY distance",
        )?;
        let rows = rescore.query_map(params![vector_bytes(query), k as i64, ids], |row| {
            Ok(DenseHit {
                chunk_id: row.get(0)?,
                distance: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn sample_chunk_vectors(
        &self,
        repo_id: Option<i64>,
        n: usize,
    ) -> Result<Vec<(i64, Vec<f32>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.chunk_id, v.embedding FROM vec_chunks v
             JOIN chunks c ON c.id = v.chunk_id
             JOIN files f ON f.id = c.file_id
             WHERE ?1 IS NULL OR f.repo_id = ?1
             ORDER BY random() LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![repo_id, n as i64], |row| {
            let bytes: Vec<u8> = row.get(1)?;
            Ok((row.get(0)?, bytes_to_vector(&bytes)))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn lexical_search(
        &self,
        repo_id: Option<i64>,
        fts_query: &str,
        k: usize,
        path_prefix: Option<&str>,
    ) -> Result<Vec<LexicalHit>> {
        let prefix_pattern = path_prefix.map(|p| format!("{}%", p.trim_end_matches('/')));
        let mut stmt = self.conn.prepare(
            "SELECT c.id, bm25(chunks_fts, 1.0, 2.0, 2.5) AS score FROM chunks_fts
             JOIN chunks c ON c.id = chunks_fts.rowid
             JOIN files f ON f.id = c.file_id
             WHERE chunks_fts MATCH ?1 AND (?2 IS NULL OR f.repo_id = ?2)
               AND (?3 IS NULL OR f.relpath LIKE ?3)
             ORDER BY score LIMIT ?4",
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

    /// Rows come back in `chunk_ids` order.
    pub fn hydrate_chunks(&self, chunk_ids: &[i64]) -> Result<Vec<ChunkRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, f.relpath, c.start_line, c.end_line, c.symbol, c.content, f.mtime, r.key
             FROM chunks c
             JOIN files f ON f.id = c.file_id
             JOIN repos r ON r.id = f.repo_id
             WHERE c.id IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = stmt.query_map([json_id_list(chunk_ids)], |row| {
            Ok(ChunkRow {
                id: row.get(0)?,
                relpath: row.get(1)?,
                start_line: row.get(2)?,
                end_line: row.get(3)?,
                symbol: row.get(4)?,
                content: row.get(5)?,
                file_mtime: row.get(6)?,
                repo_key: row.get(7)?,
            })
        })?;
        let mut by_id: HashMap<i64, ChunkRow> = rows
            .map(|row| row.map(|r| (r.id, r)))
            .collect::<std::result::Result<_, _>>()?;
        Ok(chunk_ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    pub fn symbols(&self, repo_id: Option<i64>) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT c.symbol FROM chunks c
             JOIN files f ON f.id = c.file_id
             WHERE (?1 IS NULL OR f.repo_id = ?1) AND c.symbol IS NOT NULL",
        )?;
        let rows = stmt.query_map([repo_id], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIM: usize = 16;

    fn vector(seed: u64) -> Vec<f32> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..DIM)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                ((x >> 40) as f32 / (1u64 << 24) as f32) - 0.5
            })
            .collect()
    }

    fn store_with_chunks(n: usize) -> (Store, i64) {
        let mut store = Store::open_in_memory("test", DIM).unwrap();
        let repo_id = store.upsert_repo("test/repo").unwrap();
        for file in 0..n.div_ceil(100) {
            let file_id = store
                .upsert_file(repo_id, &format!("f{file}.rs"), "0", 1, 0)
                .unwrap();
            let chunks: Vec<NewChunk> = (file * 100..((file + 1) * 100).min(n))
                .map(|i| NewChunk {
                    start_line: 1,
                    end_line: 1,
                    symbol: None,
                    content: format!("chunk {i}"),
                    content_hash: format!("{i:016x}"),
                    embedding: vector(i as u64 + 1),
                })
                .collect();
            store
                .replace_file_chunks(file_id, repo_id, &chunks)
                .unwrap();
        }
        (store, repo_id)
    }

    #[test]
    fn coarse_path_above_threshold_finds_the_exact_nearest() {
        let n = BINARY_COARSE_THRESHOLD as usize + 100;
        let (store, repo_id) = store_with_chunks(n);
        let query = vector(4242);
        let exact = store.dense_search_exact(Some(repo_id), &query, 10).unwrap();
        let hits = store.dense_search(Some(repo_id), &query, 10).unwrap();
        assert_eq!(hits.len(), 10);
        assert_eq!(hits[0].chunk_id, exact[0].chunk_id);
        assert!(hits[0].distance < 1e-6);
        assert!(hits.windows(2).all(|w| w[0].distance <= w[1].distance));
    }

    #[test]
    fn hydrate_keeps_requested_order() {
        let (store, _) = store_with_chunks(5);
        let rows = store.hydrate_chunks(&[4, 2, 5]).unwrap();
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![4, 2, 5]);
    }
}
