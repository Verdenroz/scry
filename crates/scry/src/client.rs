use anyhow::{Context, Result, anyhow};
use scry_core::config::ClientConfig;
use scry_server::api::{
    ErrorResponse, ManifestRequest, ManifestResponse, SearchRequest, SearchResponse,
    StatusResponse, SyncRequest, SyncResponse,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(config: &ClientConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: config.server_url.trim_end_matches('/').to_string(),
            token: config.token.clone(),
        }
    }

    async fn post<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        path: &str,
        request: &Req,
    ) -> Result<Resp> {
        let mut builder = self
            .http
            .post(format!("{}{path}", self.base_url))
            .json(request);
        if let Some(token) = &self.token {
            builder = builder.bearer_auth(token);
        }
        let response = builder
            .send()
            .await
            .with_context(|| format!("cannot reach scry server at {}", self.base_url))?;
        if !response.status().is_success() {
            let status = response.status();
            let message = response
                .json::<ErrorResponse>()
                .await
                .map(|e| e.error)
                .unwrap_or_else(|_| status.to_string());
            return Err(anyhow!("{message}"));
        }
        Ok(response.json().await?)
    }

    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        self.post("/v1/search", request).await
    }

    pub async fn manifest(&self, repo_key: &str) -> Result<ManifestResponse> {
        self.post(
            "/v1/manifest",
            &ManifestRequest {
                repo_key: repo_key.to_string(),
            },
        )
        .await
    }

    pub async fn sync(&self, request: &SyncRequest) -> Result<SyncResponse> {
        self.post("/v1/sync", request).await
    }

    pub async fn status(&self) -> Result<StatusResponse> {
        let mut builder = self.http.get(format!("{}/v1/status", self.base_url));
        if let Some(token) = &self.token {
            builder = builder.bearer_auth(token);
        }
        Ok(builder.send().await?.error_for_status()?.json().await?)
    }
}
