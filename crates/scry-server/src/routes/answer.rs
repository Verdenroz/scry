use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use scry_core::search::{SearchOptions, query_vector, search_with_vector};

use crate::AppState;
use crate::api::{AnswerRequest, AnswerResponse, Citation};
use crate::error::ApiError;

const LOCAL_SOURCES: usize = 8;
const WEB_SOURCES: usize = 5;
const SNIPPET_CHARS: usize = 1200;

struct Source {
    label: String,
    text: String,
}

pub async fn answer(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<AnswerResponse>, ApiError> {
    let Some(chat) = &state.chat else {
        return Err(ApiError::Unavailable(
            "answers require a [chat] endpoint in the server config".to_string(),
        ));
    };

    let mut sources: Vec<Source> = Vec::new();
    if let Some(repo_key) = &request.repo_key {
        let vector = query_vector(
            state.embedder.as_ref(),
            state.chat.as_ref(),
            state.hyde,
            &request.query,
        )
        .await?;
        let repo_key = repo_key.clone();
        let query = request.query.clone();
        let hits = state
            .store
            .call(move |store| {
                let Some(repo_id) = store.repo_id(&repo_key)? else {
                    return Ok(Vec::new());
                };
                let options = SearchOptions {
                    limit: LOCAL_SOURCES,
                    path_prefix: None,
                };
                search_with_vector(store, Some(repo_id), &query, &vector, &options)
            })
            .await?;
        sources.extend(hits.into_iter().map(|hit| Source {
            label: format!("{}:{}-{}", hit.relpath, hit.start_line, hit.end_line),
            text: truncate(&hit.content, SNIPPET_CHARS),
        }));
    }
    if request.web
        && let Some(tavily) = &state.tavily
    {
        let results = tavily.search(&request.query, WEB_SOURCES).await?;
        sources.extend(results.into_iter().map(|r| Source {
            label: r.url,
            text: truncate(&r.content, SNIPPET_CHARS),
        }));
    }
    if sources.is_empty() {
        return Err(ApiError::NotFound(
            "no sources found; index this repo or pass web: true".to_string(),
        ));
    }

    let mut prompt = String::from(
        "Answer the question using only the numbered sources below. Cite every \
         claim with [N] markers referring to source numbers. Be concise.\n\n",
    );
    for (i, source) in sources.iter().enumerate() {
        prompt.push_str(&format!(
            "Source {}: {}\n{}\n\n",
            i + 1,
            source.label,
            source.text
        ));
    }
    prompt.push_str(&format!("Question: {}\nAnswer:", request.query));

    let answer = chat
        .complete(&prompt, 700)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;

    Ok(Json(AnswerResponse {
        answer: answer.trim().to_string(),
        citations: sources
            .into_iter()
            .enumerate()
            .map(|(i, source)| Citation {
                n: i + 1,
                source: source.label,
            })
            .collect(),
    }))
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let cut = (0..=max)
        .rev()
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    text[..cut].to_string()
}
