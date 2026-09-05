//! Cross-encoder rerank of the fused candidates. The endpoint is the
//! Jina/Cohere shape llama-server exposes at `/v1/rerank`.

use serde::Deserialize;

use crate::config::RerankConfig;
use crate::search::{RRF_K, SearchHit};
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

    pub fn weight(&self) -> f64 {
        self.config.weight
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
        let mut seen = std::collections::HashSet::with_capacity(results.len());
        if results
            .iter()
            .any(|r| r.index >= documents.len() || !seen.insert(r.index))
        {
            return Err(Error::Rerank(
                "bad or repeated index in response".to_string(),
            ));
        }
        results.sort_by(|a, b| b.relevance_score.total_cmp(&a.relevance_score));
        Ok(results)
    }
}

/// Fuses the reranker's ranking into the pool as a reciprocal-rank leg
/// weighted by `weight` against the pool's own order, then keeps
/// `limit`. Scores are left as they were; the reranker changes order only.
pub fn fuse(
    hits: Vec<SearchHit>,
    ranked: &[RerankResult],
    weight: f64,
    limit: usize,
) -> Vec<SearchHit> {
    let mut score: Vec<f64> = (0..hits.len())
        .map(|rank| 1.0 / (RRF_K + rank as f64 + 1.0))
        .collect();
    for (rank, r) in ranked.iter().enumerate() {
        score[r.index] += weight / (RRF_K + rank as f64 + 1.0);
    }
    let mut order: Vec<usize> = (0..hits.len()).collect();
    order.sort_by(|a, b| score[*b].total_cmp(&score[*a]));
    let mut slots: Vec<Option<SearchHit>> = hits.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|i| slots[i].take())
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
    fn fuse_promotes_by_reranker_rank_and_keeps_scores() {
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
        let out = fuse(hits, &ranked, 2.0, 2);
        let paths: Vec<&str> = out.iter().map(|h| h.relpath.as_str()).collect();
        assert_eq!(paths, vec!["c", "a"]);
        assert_eq!(out[0].score, 0.5);
    }

    #[test]
    fn fuse_with_zero_weight_keeps_the_pool_order() {
        let hits = vec![hit("a"), hit("b")];
        let ranked = [RerankResult {
            index: 1,
            relevance_score: 9.0,
        }];
        let out = fuse(hits, &ranked, 0.0, 2);
        assert_eq!(out[0].relpath, "a");
    }
}
