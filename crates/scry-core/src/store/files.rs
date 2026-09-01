use rusqlite::params;

use super::Store;
use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    pub id: i64,
    pub relpath: String,
    pub xxh64: String,
    pub size: u64,
    pub mtime: i64,
}

impl Store {
    pub fn list_files(&self, repo_id: i64) -> Result<Vec<StoredFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, relpath, xxh64, size, mtime FROM files
             WHERE repo_id = ?1 ORDER BY relpath",
        )?;
        let rows = stmt.query_map([repo_id], |row| {
            Ok(StoredFile {
                id: row.get(0)?,
                relpath: row.get(1)?,
                xxh64: row.get(2)?,
                size: row.get::<_, i64>(3)? as u64,
                mtime: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn upsert_file(
        &self,
        repo_id: i64,
        relpath: &str,
        xxh64: &str,
        size: u64,
        mtime: i64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO files (repo_id, relpath, xxh64, size, mtime)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (repo_id, relpath)
             DO UPDATE SET xxh64 = ?3, size = ?4, mtime = ?5",
            params![repo_id, relpath, xxh64, size as i64, mtime],
        )?;
        let id = self.conn.query_row(
            "SELECT id FROM files WHERE repo_id = ?1 AND relpath = ?2",
            params![repo_id, relpath],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn delete_file(&mut self, repo_id: i64, relpath: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let file_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM files WHERE repo_id = ?1 AND relpath = ?2",
                params![repo_id, relpath],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })?;
        let Some(file_id) = file_id else {
            return Ok(());
        };
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
        tx.execute("DELETE FROM files WHERE id = ?1", [file_id])?;
        tx.commit()?;
        Ok(())
    }
}
