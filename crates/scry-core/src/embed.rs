use async_trait::async_trait;
use serde::Deserialize;

use crate::config::EmbeddingConfig;
use crate::{Error, Result};

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
}

pub struct HttpEmbedder {
    client: reqwest::Client,
    config: EmbeddingConfig,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

impl HttpEmbedder {
    pub fn new(config: EmbeddingConfig) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .expect("client build"),
            config,
        }
    }

    async fn embed_batch(&self, batch: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.config.base_url.trim_end_matches('/'));
        let mut last_err = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500 << attempt)).await;
            }
            let response = self
                .client
                .post(&url)
                .bearer_auth(&self.config.api_key)
                .json(&serde_json::json!({ "model": self.config.model, "input": batch }))
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            match response {
                Ok(response) => {
                    let mut parsed: EmbeddingResponse = response.json().await?;
                    parsed.data.sort_by_key(|item| item.index);
                    let vectors: Vec<Vec<f32>> =
                        parsed.data.into_iter().map(|item| item.embedding).collect();
                    if vectors.len() != batch.len()
                        || vectors.iter().any(|v| v.len() != self.config.dim)
                    {
                        return Err(Error::Embedding(format!(
                            "endpoint returned {} vectors (expected {}) or wrong dimension",
                            vectors.len(),
                            batch.len()
                        )));
                    }
                    return Ok(vectors);
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(Error::Embedding(format!(
            "embedding request failed after retries: {}",
            last_err.expect("at least one attempt")
        )))
    }
}

#[async_trait]
impl Embedder for HttpEmbedder {
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(self.config.batch_size.max(1)) {
            vectors.extend(self.embed_batch(batch).await?);
        }
        Ok(vectors)
    }

    fn dim(&self) -> usize {
        self.config.dim
    }
}

/// Deterministic bag-of-words feature hashing. Shares no space with any
/// real model; for tests and offline smoke runs only.
pub struct HashEmbedder {
    pub dim: usize,
}

#[async_trait]
impl Embedder for HashEmbedder {
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(inputs
            .iter()
            .map(|input| {
                let mut vector = vec![0f32; self.dim];
                for token in input
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|t| t.len() >= 2)
                {
                    let slot = crate::hashing::hash_bytes(token.to_lowercase().as_bytes());
                    vector[(slot % self.dim as u64) as usize] += 1.0;
                }
                let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for v in &mut vector {
                        *v /= norm;
                    }
                }
                vector
            })
            .collect())
    }

    fn dim(&self) -> usize {
        self.dim
    }
}
