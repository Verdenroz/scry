//! Wire types shared by the server routes and the CLI client.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    /// None searches every indexed repo.
    #[serde(default)]
    pub repo_key: Option<String>,
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
    pub repo_key: String,
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
    pub memories: i64,
    pub stale_memories: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RememberRequest {
    pub repo_key: Option<String>,
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub pain: Option<f64>,
    #[serde(default)]
    pub cost: Option<f64>,
    #[serde(default)]
    pub anchors: Vec<AnchorDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnchorDto {
    pub relpath: String,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub end_line: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RememberResponse {
    pub id: i64,
    pub salience: f64,
    pub surprise: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecallRequest {
    pub repo_key: Option<String>,
    pub query: String,
    #[serde(default = "default_recall_limit")]
    pub limit: usize,
}

fn default_recall_limit() -> usize {
    5
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecallResponse {
    pub memories: Vec<MemoryDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryDto {
    pub id: i64,
    pub kind: String,
    pub content: String,
    pub score: f64,
    pub stale: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FeedbackRequest {
    pub id: i64,
    pub helpful: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FeedbackResponse {
    pub updated: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PruneRequest {
    pub repo_key: String,
    #[serde(default)]
    pub migrate_memories_to: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PruneResponse {
    pub deleted_files: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WebSearchRequest {
    pub query: String,
    #[serde(default = "default_web_limit")]
    pub limit: usize,
}

fn default_web_limit() -> usize {
    5
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WebSearchResponse {
    pub results: Vec<WebHit>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WebHit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnswerRequest {
    pub query: String,
    #[serde(default)]
    pub repo_key: Option<String>,
    #[serde(default)]
    pub web: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnswerResponse {
    pub answer: String,
    pub citations: Vec<Citation>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Citation {
    pub n: usize,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}
