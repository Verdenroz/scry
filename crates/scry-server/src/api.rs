//! Wire types shared by the server routes and the CLI client.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub repo_key: String,
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub path_prefix: Option<String>,
}

fn default_limit() -> usize {
    10
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<Hit>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Hit {
    pub relpath: String,
    pub start_line: u32,
    pub end_line: u32,
    pub symbol: Option<String>,
    pub score: f64,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestRequest {
    pub repo_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestResponse {
    pub files: Vec<ManifestEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub relpath: String,
    pub xxh64: String,
    pub size: u64,
    pub mtime_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequest {
    pub repo_key: String,
    #[serde(default)]
    pub upserts: Vec<FileUpload>,
    #[serde(default)]
    pub deletes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileUpload {
    pub relpath: String,
    pub content: String,
    pub xxh64: String,
    pub size: u64,
    pub mtime_ms: i64,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SyncResponse {
    pub indexed_files: usize,
    pub deleted_files: usize,
    pub embedded_chunks: usize,
    pub reused_chunks: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub repos: i64,
    pub files: i64,
    pub chunks: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}
