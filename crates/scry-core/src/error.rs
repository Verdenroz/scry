#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Config(String),
    #[error("invalid config file: {0}")]
    ConfigParse(#[from] toml::de::Error),
    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("embedding: {0}")]
    Embedding(String),
    #[error("chat: {0}")]
    Chat(String),
    #[error("rerank: {0}")]
    Rerank(String),
}

pub type Result<T> = std::result::Result<T, Error>;
