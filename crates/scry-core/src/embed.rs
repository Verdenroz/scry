use async_trait::async_trait;
use serde::Deserialize;

use crate::config::EmbeddingConfig;
use crate::{Error, Result};

#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embeds stored content: chunks, memories, HyDE snippets.
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Embeds search and recall queries, which some models expect to carry
    /// an instruction the documents do not.
    async fn embed_queries(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed(inputs).await
    }

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

    async fn embed_prefixed(&self, prefix: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(self.config.batch_size.max(1)) {
            let batch: Vec<String> = batch
                .iter()
                .map(|input| format!("{prefix}{input}"))
                .collect();
            vectors.extend(self.embed_batch(&batch).await?);
        }
        Ok(vectors)
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
        self.embed_prefixed(&self.config.document_prefix, inputs)
            .await
    }

    async fn embed_queries(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed_prefixed(&self.config.query_prefix, inputs).await
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

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use super::*;

    /// Answers every `/embeddings` request with unit vectors and reports the
    /// `input` list it received.
    fn stub_endpoint() -> (String, mpsc::Receiver<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let (sent, received) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let inputs: Vec<String> = request["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                let data: Vec<_> = (0..inputs.len())
                    .map(|i| serde_json::json!({ "index": i, "embedding": [1.0, 0.0] }))
                    .collect();
                sent.send(inputs).unwrap();
                let reply = serde_json::json!({ "data": data }).to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                )
                .unwrap();
            }
        });
        (base_url, received)
    }

    #[tokio::test]
    async fn documents_and_queries_carry_their_own_prefix() {
        let (base_url, received) = stub_endpoint();
        let embedder = HttpEmbedder::new(EmbeddingConfig {
            base_url,
            dim: 2,
            document_prefix: "doc: ".to_string(),
            query_prefix: "query: ".to_string(),
            ..EmbeddingConfig::default()
        });
        embedder.embed(&["fn main".to_string()]).await.unwrap();
        embedder
            .embed_queries(&["where is main".to_string()])
            .await
            .unwrap();
        assert_eq!(received.recv().unwrap(), ["doc: fn main"]);
        assert_eq!(received.recv().unwrap(), ["query: where is main"]);
    }

    #[tokio::test]
    async fn an_empty_query_prefix_sends_queries_unchanged() {
        let (base_url, received) = stub_endpoint();
        let embedder = HttpEmbedder::new(EmbeddingConfig {
            base_url,
            dim: 2,
            ..EmbeddingConfig::default()
        });
        embedder
            .embed_queries(&["where is main".to_string()])
            .await
            .unwrap();
        assert_eq!(received.recv().unwrap(), ["where is main"]);
    }
}
