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
    /// Applies the server's `[rerank]` stage when one is configured.
    #[serde(default = "default_true")]
    pub rerank: bool,
}

fn default_limit() -> usize {
    10
}

fn default_true() -> bool {
    true
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
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub anchors: Vec<AnchorDto>,
}

fn default_source() -> String {
    "agent".to_string()
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
    #[serde(default = "default_touch")]
    pub touch: bool,
}

fn default_recall_limit() -> usize {
    5
}

fn default_touch() -> bool {
    true
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
    /// Scope the vote was cast in; noise suppresses the memory there.
    #[serde(default)]
    pub repo_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FeedbackResponse {
    pub updated: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BriefRequest {
    pub repo_key: Option<String>,
    #[serde(default = "default_brief_limit")]
    pub limit: usize,
    #[serde(default = "default_stale_days")]
    pub stale_days: u32,
}

fn default_brief_limit() -> usize {
    8
}

fn default_stale_days() -> u32 {
    7
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BriefResponse {
    pub memories: Vec<MemoryDto>,
    pub went_stale: Vec<MemoryDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SupersedeRequest {
    pub old_id: i64,
    pub new_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SupersedeResponse {
    pub superseded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerifyRequest {
    pub claim: String,
    pub evidence: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerifyResponse {
    pub configured: bool,
    pub score: Option<f64>,
    pub supported: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryStatsResponse {
    pub sources: Vec<SourceStatsDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SourceStatsDto {
    pub source: String,
    pub memories: i64,
    pub stale: i64,
    pub retrievals: i64,
    pub helpful: i64,
    pub noise: i64,
    pub archived: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReaffirmRequest {
    pub id: i64,
    pub evidence: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReaffirmResponse {
    pub id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StaleRequest {
    #[serde(default)]
    pub repo_key: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StaleMemoryDto {
    pub id: i64,
    pub kind: String,
    pub content: String,
    pub relpaths: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StaleResponse {
    pub memories: Vec<StaleMemoryDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GcRequest {
    #[serde(default)]
    pub dry_run: bool,
}

pub type GcResponse = scry_core::memory_gc::GcReport;

#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreRequest {
    pub id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreResponse {
    pub id: i64,
    pub status: String,
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
    /// Applies the server's `[rerank]` stage to the local sources.
    #[serde(default = "default_true")]
    pub rerank: bool,
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
