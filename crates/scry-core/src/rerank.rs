//! Cross-encoder rerank of the fused candidates. The endpoint is the
//! Jina/Cohere shape llama-server exposes at `/v1/rerank`.

use serde::Deserialize;

use crate::config::{RerankConfig, RerankGate};
use crate::search::SearchHit;
use crate::{Error, Result};

pub struct RerankClient {
    client: reqwest::Client,
    config: RerankConfig,
}

#[derive(Deserialize)]
struct RerankResponse {
    results: Vec<RerankResult>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct RerankResult {
    pub index: usize,
    pub relevance_score: f64,
}

impl RerankClient {
    pub fn new(config: RerankConfig) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("client build"),
            config,
        }
    }

    pub fn top_n(&self) -> usize {
        self.config.top_n
    }

    pub fn gate(&self) -> RerankGate {
        self.config.gate
    }

    /// Scores every document against the query; results come back best
    /// first with the index into `documents`.
    pub async fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<RerankResult>> {
        let url = format!("{}/rerank", self.config.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.config.model,
            "query": query,
            "documents": documents,
            "top_n": documents.len(),
        });
        let mut request = self.client.post(&url).json(&body);
        if !self.config.api_key.is_empty() {
            request = request.bearer_auth(&self.config.api_key);
        }
        let response = request.send().await?.error_for_status()?;
        let parsed: RerankResponse = response.json().await?;
        let mut results = parsed.results;
        if results.iter().any(|r| r.index >= documents.len()) {
            return Err(Error::Rerank("index out of range in response".to_string()));
        }
        results.sort_by(|a, b| b.relevance_score.total_cmp(&a.relevance_score));
        Ok(results)
    }
}

/// Reorders `hits` by the reranker's ranking, reports its relevance clamped
/// to [0, 1] as the score, and keeps `limit`. Hits the reranker did not
/// score are dropped.
pub fn reorder(hits: Vec<SearchHit>, ranked: &[RerankResult], limit: usize) -> Vec<SearchHit> {
    let mut slots: Vec<Option<SearchHit>> = hits.into_iter().map(Some).collect();
    ranked
        .iter()
        .filter_map(|r| {
            slots[r.index].take().map(|hit| SearchHit {
                score: r.relevance_score.clamp(0.0, 1.0),
                ..hit
            })
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(relpath: &str) -> SearchHit {
        SearchHit {
            repo_key: "r".to_string(),
            relpath: relpath.to_string(),
            start_line: 1,
            end_line: 1,
            symbol: None,
            score: 0.5,
            content: String::new(),
        }
    }

    #[test]
    fn reorder_follows_the_reranker_and_truncates() {
        let hits = vec![hit("a"), hit("b"), hit("c")];
        let ranked = [
            RerankResult {
                index: 2,
                relevance_score: 3.5,
            },
            RerankResult {
                index: 0,
                relevance_score: 0.4,
            },
            RerankResult {
                index: 1,
                relevance_score: -1.0,
            },
        ];
        let out = reorder(hits, &ranked, 2);
        let paths: Vec<&str> = out.iter().map(|h| h.relpath.as_str()).collect();
        assert_eq!(paths, vec!["c", "a"]);
        assert_eq!(out[0].score, 1.0);
        assert_eq!(out[1].score, 0.4);
    }
}
