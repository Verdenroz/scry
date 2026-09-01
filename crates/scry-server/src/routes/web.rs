use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::AppState;
use crate::api::{WebHit, WebSearchRequest, WebSearchResponse};
use crate::error::ApiError;

pub async fn web_search(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WebSearchRequest>,
) -> Result<Json<WebSearchResponse>, ApiError> {
    let Some(tavily) = &state.tavily else {
        return Err(ApiError::Unavailable(
            "web search requires [tavily] api_key (or TAVILY_API_KEY) in the server config"
                .to_string(),
        ));
    };
    let results = tavily.search(&request.query, request.limit).await?;
    Ok(Json(WebSearchResponse {
        results: results
            .into_iter()
            .map(|r| WebHit {
                url: r.url,
                title: r.title,
                snippet: r.content,
                score: r.score,
            })
            .collect(),
    }))
}
