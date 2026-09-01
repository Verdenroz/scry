use scry_core::config::TavilyConfig;
use scry_core::{Error, Result};
use serde::Deserialize;

pub struct TavilyClient {
    http: reqwest::Client,
    api_key: String,
}

#[derive(Debug, Deserialize)]
pub struct TavilyResult {
    pub title: String,
    pub url: String,
    pub content: String,
    pub score: f64,
}

#[derive(Deserialize)]
struct TavilyResponse {
    results: Vec<TavilyResult>,
}

impl TavilyClient {
    pub fn new(config: TavilyConfig) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("client build"),
            api_key: config.api_key,
        }
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<TavilyResult>> {
        let response = self
            .http
            .post("https://api.tavily.com/search")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "query": query, "max_results": limit }))
            .send()
            .await?
            .error_for_status()
            .map_err(|e| Error::Config(format!("tavily search failed: {e}")))?;
        let parsed: TavilyResponse = response.json().await?;
        Ok(parsed.results)
    }
}
