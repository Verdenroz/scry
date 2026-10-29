//! Layered config: file (explicit path, `$SCRY_CONFIG`, or
//! `~/.config/scry/config.toml`), then env overrides. Secret-bearing keys
//! accept `env:VAR` indirection so tokens stay out of the file.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub chat: Option<ChatConfig>,
    #[serde(default)]
    pub verify: Option<VerifyConfig>,
    #[serde(default)]
    pub tavily: Option<TavilyConfig>,
    #[serde(default)]
    pub rerank: Option<RerankConfig>,
    #[serde(default)]
    pub client: ClientConfig,
    #[serde(default)]
    pub index: IndexConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    #[serde(default = "default_half_life_days")]
    pub half_life_days: f64,
    #[serde(default = "default_gc_stale_days")]
    pub gc_stale_days: u32,
    #[serde(default = "default_gc_episode_floor")]
    pub gc_episode_floor: f64,
}

fn default_half_life_days() -> f64 {
    29.0
}

fn default_gc_stale_days() -> u32 {
    30
}

fn default_gc_episode_floor() -> f64 {
    0.1
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            half_life_days: default_half_life_days(),
            gc_stale_days: default_gc_stale_days(),
            gc_episode_floor: default_gc_episode_floor(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SearchConfig {
    #[serde(default)]
    pub hyde: HydeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HydeMode {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
    #[serde(default)]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingConfig {
    #[serde(default = "default_embedding_url")]
    pub base_url: String,
    #[serde(default = "default_embedding_key")]
    pub api_key: String,
    #[serde(default = "default_embedding_model")]
    pub model: String,
    #[serde(default = "default_dim")]
    pub dim: usize,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatConfig {
    pub base_url: String,
    #[serde(default = "default_embedding_key")]
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub thinking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VerifyApi {
    #[default]
    Chat,
    SystemOne,
}

/// Claim verifier. `chat` asks an OpenAI-compatible model a yes/no question
/// at `{base_url}/chat/completions`; `systemone` posts to `{base_url}/systemone`.
/// Both take a base_url ending in `/v1`.
/// Absent means verification is not configured.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyConfig {
    #[serde(default)]
    pub api: VerifyApi,
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_embedding_key")]
    pub api_key: String,
    #[serde(default = "default_verify_threshold")]
    pub threshold: f64,
    /// Chat prompt override; must contain `{claim}` and `{evidence}`.
    #[serde(default)]
    pub prompt: Option<String>,
}

fn default_verify_threshold() -> f64 {
    0.5
}

impl VerifyConfig {
    fn validate(&self) -> Result<()> {
        let Some(prompt) = &self.prompt else {
            return Ok(());
        };
        for placeholder in ["{claim}", "{evidence}"] {
            if !prompt.contains(placeholder) {
                return Err(Error::Config(format!(
                    "[verify] prompt must contain {placeholder}"
                )));
            }
        }
        Ok(())
    }
}

/// Cross-encoder over the fused top `top_n`, reached through an
/// OpenAI/Jina-style `POST {base_url}/rerank`, fused back into the
/// ranking as a weighted reciprocal-rank leg. Absent means no rerank.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RerankConfig {
    pub base_url: String,
    /// Empty sends no Authorization header.
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_rerank_top_n")]
    pub top_n: usize,
    #[serde(default = "default_rerank_weight")]
    pub weight: f64,
    /// Each document is cut here before scoring; the header and the start
    /// of a chunk carry the signal the cross-encoder needs.
    #[serde(default = "default_rerank_max_chars")]
    pub max_chars: usize,
}

fn default_rerank_top_n() -> usize {
    20
}

fn default_rerank_max_chars() -> usize {
    3000
}

fn default_rerank_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TavilyConfig {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    #[serde(default = "default_server_url")]
    pub server_url: String,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexConfig {
    #[serde(default = "default_max_file_size", deserialize_with = "de_byte_size")]
    pub max_file_size: u64,
    #[serde(default = "default_max_file_count")]
    pub max_file_count: usize,
}

fn default_listen() -> String {
    "127.0.0.1:7345".to_string()
}

fn default_db_path() -> PathBuf {
    PathBuf::from("~/.local/share/scry/index.db")
}

fn default_embedding_url() -> String {
    "http://localhost:12434/v1".to_string()
}

fn default_embedding_key() -> String {
    "ollama".to_string()
}

fn default_embedding_model() -> String {
    "harrier-oss:0.6b".to_string()
}

fn default_dim() -> usize {
    1024
}

fn default_batch_size() -> usize {
    8
}

fn default_server_url() -> String {
    "http://127.0.0.1:7345".to_string()
}

fn default_max_file_size() -> u64 {
    1024 * 1024
}

fn default_max_file_count() -> usize {
    10_000
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            db_path: default_db_path(),
            auth_token: None,
        }
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            base_url: default_embedding_url(),
            api_key: default_embedding_key(),
            model: default_embedding_model(),
            dim: default_dim(),
            batch_size: default_batch_size(),
        }
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_url: default_server_url(),
            token: None,
        }
    }
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            max_file_size: default_max_file_size(),
            max_file_count: default_max_file_count(),
        }
    }
}

pub fn parse_byte_size(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Ok(n) = text.parse::<u64>() {
        return Some(n);
    }
    let split = text.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = text.split_at(split);
    let num: u64 = num.trim().parse().ok()?;
    let factor = match unit.to_ascii_uppercase().as_str() {
        "B" => 1,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    Some(num * factor)
}

fn de_byte_size<'de, D: serde::Deserializer<'de>>(de: D) -> std::result::Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Int(u64),
        Text(String),
    }
    match Raw::deserialize(de)? {
        Raw::Int(n) => Ok(n),
        Raw::Text(text) => parse_byte_size(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid byte size {text:?}"))),
    }
}

fn expand_home(path: &Path) -> PathBuf {
    let Ok(stripped) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(stripped),
        None => path.to_path_buf(),
    }
}

fn resolve_secret(value: Option<String>) -> Option<String> {
    let value = value?;
    match value.strip_prefix("env:") {
        Some(var) => std::env::var(var).ok(),
        None => Some(value),
    }
}

impl Config {
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let env_path = std::env::var("SCRY_CONFIG").ok().map(PathBuf::from);
        let default_path = std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".config/scry/config.toml"));
        let path = explicit
            .map(Path::to_path_buf)
            .or(env_path)
            .or_else(|| default_path.filter(|p| p.exists()));

        let mut config = match path {
            Some(path) => {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| Error::Config(format!("cannot read {}: {e}", path.display())))?;
                toml::from_str(&text)?
            }
            None => Self::default(),
        };
        config.apply_env();
        if let Some(verify) = &config.verify {
            verify.validate()?;
        }
        config.server.db_path = expand_home(&config.server.db_path);
        Ok(config)
    }

    fn apply_env(&mut self) {
        if let Ok(listen) = std::env::var("SCRY_LISTEN") {
            self.server.listen = listen;
        }
        if let Ok(db_path) = std::env::var("SCRY_DB_PATH") {
            self.server.db_path = PathBuf::from(db_path);
        }
        if let Ok(url) = std::env::var("SCRY_SERVER_URL") {
            self.client.server_url = url;
        }
        if let Ok(token) = std::env::var("SCRY_TOKEN") {
            self.server.auth_token = Some(token.clone());
            self.client.token = Some(token);
        }
        if let Ok(key) = std::env::var("TAVILY_API_KEY") {
            self.tavily = Some(TavilyConfig { api_key: key });
        }
        self.server.auth_token = resolve_secret(self.server.auth_token.take());
        self.client.token = resolve_secret(self.client.token.take());
        resolve_key(&mut self.embedding.api_key);
        if let Some(chat) = self.chat.as_mut() {
            resolve_key(&mut chat.api_key);
        }
        if let Some(verify) = self.verify.as_mut() {
            resolve_key(&mut verify.api_key);
        }
        if let Some(tavily) = self.tavily.as_mut() {
            resolve_key(&mut tavily.api_key);
        }
        if let Some(rerank) = self.rerank.as_mut() {
            resolve_key(&mut rerank.api_key);
        }
    }
}

fn resolve_key(key: &mut String) {
    *key = resolve_secret(Some(std::mem::take(key))).unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() {
        let config = Config::default();
        assert_eq!(config.server.listen, "127.0.0.1:7345");
        assert_eq!(config.client.server_url, "http://127.0.0.1:7345");
        assert_eq!(config.embedding.dim, 1024);
        assert_eq!(config.index.max_file_size, 1024 * 1024);
        assert!(config.chat.is_none());
        assert!(config.tavily.is_none());
    }

    #[test]
    fn parses_full_config() {
        let config: Config = toml::from_str(
            r#"
            [server]
            listen = "0.0.0.0:7345"
            db_path = "/var/lib/scry/index.db"
            auth_token = "secret"

            [embedding]
            base_url = "http://embed:8080/v1"
            model = "custom"
            dim = 768
            batch_size = 16

            [chat]
            base_url = "http://chat:8080/v1"
            model = "qwen3-4b"

            [tavily]
            api_key = "tvly-key"

            [client]
            server_url = "https://scry.example.com"
            token = "secret"

            [index]
            max_file_size = "2MB"
            max_file_count = 500
            "#,
        )
        .unwrap();
        assert_eq!(config.server.auth_token.as_deref(), Some("secret"));
        assert_eq!(config.embedding.dim, 768);
        assert_eq!(config.chat.unwrap().model, "qwen3-4b");
        assert_eq!(config.index.max_file_size, 2 * 1024 * 1024);
        assert_eq!(config.index.max_file_count, 500);
    }

    #[test]
    fn parses_rerank_section() {
        let config: Config = toml::from_str(
            "[rerank]\nbase_url = \"http://r:8080/v1\"\nmodel = \"bge\"\nweight = 2.0\n",
        )
        .unwrap();
        let rerank = config.rerank.unwrap();
        assert_eq!(rerank.top_n, 20);
        assert_eq!(rerank.weight, 2.0);
        assert!(Config::default().rerank.is_none());
    }

    #[test]
    fn parses_verify_section_with_neutral_defaults() {
        let config: Config =
            toml::from_str("[verify]\nbase_url = \"http://v:8080/v1\"\nmodel = \"m\"\n").unwrap();
        let verify = config.verify.unwrap();
        assert_eq!(verify.api, VerifyApi::Chat);
        assert_eq!(verify.threshold, 0.5);
        assert!(Config::default().verify.is_none());
    }

    #[test]
    fn verify_prompt_needs_both_placeholders() {
        let verify = |prompt: &str| VerifyConfig {
            api: VerifyApi::Chat,
            base_url: String::new(),
            model: String::new(),
            api_key: String::new(),
            threshold: 0.5,
            prompt: Some(prompt.to_string()),
        };
        assert!(verify("{claim} vs {evidence}").validate().is_ok());
        assert!(verify("{claim} only").validate().is_err());
        assert!(verify("{evidence} only").validate().is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<Config>("[server]\nlissten = \"x\"\n").is_err());
    }

    #[test]
    fn byte_sizes_accept_int_and_suffixed() {
        assert_eq!(parse_byte_size("1048576"), Some(1024 * 1024));
        assert_eq!(parse_byte_size("1MB"), Some(1024 * 1024));
        assert_eq!(parse_byte_size("512kb"), Some(512 * 1024));
        assert_eq!(parse_byte_size("bogus"), None);
    }
}
