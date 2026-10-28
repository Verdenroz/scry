use anyhow::{Context, Result, anyhow};
use scry_core::config::ClientConfig;
use scry_server::api::{
    AnswerRequest, AnswerResponse, BriefRequest, BriefResponse, ErrorResponse, FeedbackRequest,
    FeedbackResponse, ManifestRequest, ManifestResponse, MemoryStatsResponse, PruneRequest,
    PruneResponse, RecallRequest, RecallResponse, RememberRequest, RememberResponse, SearchRequest,
    SearchResponse, StatusResponse, SupersedeRequest, SupersedeResponse, SyncRequest, SyncResponse,
    VerifyRequest, VerifyResponse, WebSearchRequest, WebSearchResponse,
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
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .expect("client build"),
            base_url: config.server_url.trim_end_matches('/').to_string(),
            token: config.token.clone(),
        }
    }

    async fn get<Resp: DeserializeOwned>(&self, path: &str) -> Result<Resp> {
        let mut builder = self.http.get(format!("{}{path}", self.base_url));
        if let Some(token) = &self.token {
            builder = builder.bearer_auth(token);
        }
        Ok(builder.send().await?.error_for_status()?.json().await?)
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

    pub async fn prune(&self, request: &PruneRequest) -> Result<PruneResponse> {
        self.post("/v1/repos/prune", request).await
    }

    pub async fn remember(&self, request: &RememberRequest) -> Result<RememberResponse> {
        self.post("/v1/memories/remember", request).await
    }

    pub async fn recall(&self, request: &RecallRequest) -> Result<RecallResponse> {
        self.post("/v1/memories/recall", request).await
    }

    pub async fn feedback(&self, request: &FeedbackRequest) -> Result<FeedbackResponse> {
        self.post("/v1/memories/feedback", request).await
    }

    pub async fn brief(&self, request: &BriefRequest) -> Result<BriefResponse> {
        self.post("/v1/memories/brief", request).await
    }

    pub async fn supersede(&self, request: &SupersedeRequest) -> Result<SupersedeResponse> {
        self.post("/v1/memories/supersede", request).await
    }

    pub async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse> {
        self.post("/v1/memories/verify", request).await
    }

    pub async fn web_search(&self, request: &WebSearchRequest) -> Result<WebSearchResponse> {
        self.post("/v1/web/search", request).await
    }

    pub async fn answer(&self, request: &AnswerRequest) -> Result<AnswerResponse> {
        self.post("/v1/answer", request).await
    }

    pub async fn status(&self) -> Result<StatusResponse> {
        self.get("/v1/status").await
    }

    pub async fn memory_stats(&self) -> Result<MemoryStatsResponse> {
        self.get("/v1/memories/stats").await
    }
}
