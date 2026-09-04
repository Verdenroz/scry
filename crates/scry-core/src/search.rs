//! Routed hybrid retrieval: dense KNN and BM25 fused with weighted RRF,
//! symbol-aware query expansion, optional HyDE, recency boost, and greedy
//! near-duplicate filtering.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::chat::ChatClient;
use crate::config::HydeMode;
use crate::embed::Embedder;
use crate::store::Store;
use crate::{Error, Result};

pub const RRF_K: f64 = 60.0;
const CANDIDATES: usize = 50;
const JACCARD_DEDUP: f64 = 0.8;
const RECENCY_BOOST: f64 = 0.1;
const RECENCY_DECAY_DAYS: f64 = 14.0;
const MAX_SYMBOL_EXPANSIONS: usize = 8;
const HYDE_TIMEOUT: Duration = Duration::from_secs(6);
const HYDE_MAX_TOKENS: u32 = 120;

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub limit: usize,
    pub path_prefix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub repo_key: String,
    pub relpath: String,
    pub start_line: u32,
    pub end_line: u32,
    pub symbol: Option<String>,
    pub score: f64,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Route {
    pub dense_weight: f64,
    pub lexical_weight: f64,
    pub natural_language: bool,
}

/// Short keyword and identifier-shaped queries collapse dense retrieval,
/// so they weight the lexical leg up; question-shaped queries weight dense.
pub fn route_query(query: &str) -> Route {
    let tokens = query_tokens(query);
    let identifier_like = tokens
        .iter()
        .any(|t| t.contains('_') || t.contains("::") || has_inner_uppercase(t));
    if tokens.len() <= 3 || (identifier_like && tokens.len() <= 6) {
        Route {
            dense_weight: 0.35,
            lexical_weight: 0.65,
            natural_language: false,
        }
    } else {
        Route {
            dense_weight: 0.7,
            lexical_weight: 0.3,
            natural_language: true,
        }
    }
}

fn has_inner_uppercase(token: &str) -> bool {
    token.chars().next().is_some_and(char::is_lowercase)
        && token.chars().skip(1).any(char::is_uppercase)
}

fn query_tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != ':')
        .filter(|t| t.len() >= 2)
        .map(str::to_string)
        .collect()
}

fn subtokens(identifier: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for word in identifier.split(|c: char| !c.is_alphanumeric()) {
        let mut current = String::new();
        for c in word.chars() {
            if c.is_uppercase() && !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            current.push(c.to_ascii_lowercase());
        }
        if !current.is_empty() {
            parts.push(current);
        }
    }
    parts
}

/// Either side being a >= 4 char prefix of the other counts, so
/// "cryptocurrency" reaches `crypto` and "domain" reaches `domains`.
fn tokens_match(wanted: &str, subtoken: &str) -> bool {
    if wanted == subtoken {
        return true;
    }
    let stem = wanted.len().min(subtoken.len());
    stem >= 4 && (wanted.starts_with(subtoken) || subtoken.starts_with(wanted))
}

/// Repo identifiers whose subtokens match a query token, e.g. query
/// "auth" pulls in `AuthLayer` and `require_auth` for the lexical leg.
pub fn expand_symbols(symbols: &[String], query: &str) -> Vec<String> {
    let wanted: Vec<String> = query_tokens(query)
        .iter()
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_lowercase())
        .collect();
    let mut seen = HashSet::new();
    let mut expansions = Vec::new();
    for symbol in symbols {
        for identifier in symbol.split(" > ") {
            let lower = identifier.to_lowercase();
            if wanted.contains(&lower) || seen.contains(&lower) {
                continue;
            }
            let subs = subtokens(identifier);
            if wanted
                .iter()
                .any(|w| subs.iter().any(|sub| tokens_match(w, sub)))
            {
                seen.insert(lower);
                expansions.push(identifier.to_string());
                if expansions.len() >= MAX_SYMBOL_EXPANSIONS {
                    return expansions;
                }
            }
        }
    }
    expansions
}

/// Tokens of >= 4 chars become FTS prefix terms so "domain" also matches
/// "domains" in path and content.
pub fn fts_query(query: &str, expansions: &[String]) -> String {
    let mut terms: Vec<String> = query_tokens(query)
        .iter()
        .take(12)
        .map(|t| {
            let quoted = format!("\"{}\"", t.replace('"', ""));
            if t.len() >= 4 {
                format!("{quoted}*")
            } else {
                quoted
            }
        })
        .collect();
    terms.extend(
        expansions
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', ""))),
    );
    terms.join(" OR ")
}

pub async fn query_vector(
    embedder: &dyn Embedder,
    chat: Option<&ChatClient>,
    hyde: HydeMode,
    query: &str,
) -> Result<Vec<f32>> {
    let route = route_query(query);
    let use_hyde = match hyde {
        HydeMode::Off => false,
        HydeMode::On => chat.is_some(),
        HydeMode::Auto => chat.is_some() && route.natural_language,
    };
    if use_hyde {
        let chat = chat.expect("checked above");
        let prompt = format!(
            "Write a short code snippet with a brief comment that would plausibly \
             appear in a codebase answering: {query}\nOnly output the snippet."
        );
        if let Ok(Ok(snippet)) =
            tokio::time::timeout(HYDE_TIMEOUT, chat.complete(&prompt, HYDE_MAX_TOKENS)).await
        {
            let vectors = embedder.embed(&[query.to_string(), snippet]).await?;
            return Ok(average(&vectors));
        }
    }
    let mut vectors = embedder.embed(&[query.to_string()]).await?;
    vectors
        .pop()
        .ok_or_else(|| Error::Embedding("empty embedding response".to_string()))
}

fn average(vectors: &[Vec<f32>]) -> Vec<f32> {
    let dim = vectors[0].len();
    let mut avg = vec![0f32; dim];
    for vector in vectors {
        for (slot, v) in avg.iter_mut().zip(vector) {
            *slot += v / vectors.len() as f32;
        }
    }
    let norm = avg.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut avg {
            *v /= norm;
        }
    }
    avg
}

fn token_set(content: &str) -> HashSet<String> {
    content
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let intersection = a.intersection(b).count() as f64;
    let union = (a.len() + b.len()) as f64 - intersection;
    if union <= 0.0 {
        1.0
    } else {
        intersection / union
    }
}

pub async fn hybrid_search(
    store: &Store,
    embedder: &dyn Embedder,
    chat: Option<&ChatClient>,
    hyde: HydeMode,
    repo_id: Option<i64>,
    query: &str,
    options: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    let vector = query_vector(embedder, chat, hyde, query).await?;
    search_with_vector(store, repo_id, query, &vector, options)
}

/// The synchronous half of retrieval; the query vector comes from
/// [`query_vector`] so no await ever holds the store. `repo_id` None
/// searches every indexed repo.
pub fn search_with_vector(
    store: &Store,
    repo_id: Option<i64>,
    query: &str,
    vector: &[f32],
    options: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    let route = route_query(query);
    let limit = options.limit.max(1);
    let fetch = if options.path_prefix.is_some() {
        CANDIDATES * 4
    } else {
        CANDIDATES
    };

    let dense = store.dense_search(repo_id, vector, fetch)?;

    let expansions = expand_symbols(&store.symbols(repo_id)?, query);
    let fts = fts_query(query, &expansions);
    let lexical = if fts.is_empty() {
        Vec::new()
    } else {
        store.lexical_search(repo_id, &fts, fetch, options.path_prefix.as_deref())?
    };

    let mut fused: HashMap<i64, f64> = HashMap::new();
    let mut similarity: HashMap<i64, f64> = HashMap::new();
    for (rank, hit) in dense.iter().enumerate() {
        *fused.entry(hit.chunk_id).or_default() += route.dense_weight / (RRF_K + rank as f64 + 1.0);
        similarity.insert(hit.chunk_id, (1.0 - hit.distance).clamp(0.0, 1.0));
    }
    for (rank, hit) in lexical.iter().enumerate() {
        *fused.entry(hit.chunk_id).or_default() +=
            route.lexical_weight / (RRF_K + rank as f64 + 1.0);
    }

    let mut candidates: Vec<(i64, f64)> = fused.into_iter().collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    candidates.truncate(CANDIDATES);

    let ids: Vec<i64> = candidates.iter().map(|(id, _)| *id).collect();
    let rows = store.hydrate_chunks(&ids)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);

    let mut scored: Vec<(f64, crate::store::ChunkRow)> = candidates
        .iter()
        .zip(rows)
        .filter(|(_, row)| match &options.path_prefix {
            Some(prefix) => row.relpath.starts_with(prefix.trim_start_matches("./")),
            None => true,
        })
        .map(|((_, fused_score), row)| {
            let age_days = ((now_ms - row.file_mtime).max(0) as f64) / 86_400_000.0;
            let boost = 1.0 + RECENCY_BOOST * (-age_days / RECENCY_DECAY_DAYS).exp();
            (fused_score * boost, row)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    let mut kept: Vec<(f64, crate::store::ChunkRow)> = Vec::new();
    let mut kept_tokens: Vec<HashSet<String>> = Vec::new();
    for (score, row) in scored {
        let tokens = token_set(&row.content);
        if kept_tokens
            .iter()
            .any(|k| jaccard(k, &tokens) >= JACCARD_DEDUP)
        {
            continue;
        }
        kept_tokens.push(tokens);
        kept.push((score, row));
        if kept.len() >= limit {
            break;
        }
    }

    let floor = kept
        .iter()
        .filter_map(|(_, row)| similarity.get(&row.id))
        .fold(f64::INFINITY, |a, b| a.min(*b));
    let floor = if floor.is_finite() { floor } else { 0.5 };

    Ok(kept
        .into_iter()
        .enumerate()
        .map(|(position, (_, row))| {
            let score = similarity
                .get(&row.id)
                .copied()
                .unwrap_or_else(|| floor * 0.95f64.powi(position as i32 + 1));
            SearchHit {
                repo_key: row.repo_key,
                relpath: row.relpath,
                start_line: row.start_line,
                end_line: row.end_line,
                symbol: row.symbol,
                score,
                content: row.content,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_queries_route_lexical() {
        let route = route_query("auth flow");
        assert!(route.lexical_weight > route.dense_weight);
        assert!(!route.natural_language);
    }

    #[test]
    fn identifier_queries_route_lexical() {
        let route = route_query("where is replace_file_chunks called from");
        assert!(route.lexical_weight > route.dense_weight);
    }

    #[test]
    fn questions_route_dense() {
        let route = route_query("how does the sync protocol decide what to upload");
        assert!(route.dense_weight > route.lexical_weight);
        assert!(route.natural_language);
    }

    #[test]
    fn symbol_expansion_matches_subtokens() {
        let symbols = vec![
            "AuthLayer".to_string(),
            "Store > replace_file_chunks".to_string(),
            "unrelated".to_string(),
        ];
        let expansions = expand_symbols(&symbols, "auth middleware chunks");
        assert!(expansions.contains(&"AuthLayer".to_string()));
        assert!(expansions.contains(&"replace_file_chunks".to_string()));
        assert!(!expansions.contains(&"unrelated".to_string()));
    }

    #[test]
    fn fts_query_quotes_prefixes_and_ors() {
        let q = fts_query("auth is flow", &["AuthLayer".to_string()]);
        assert_eq!(q, "\"auth\"* OR \"is\" OR \"flow\"* OR \"AuthLayer\"");
    }

    #[test]
    fn expansion_matches_prefixes_both_ways() {
        let symbols = vec!["get_crypto_quotes".to_string(), "domains".to_string()];
        let expansions = expand_symbols(&symbols, "cryptocurrency domain prices");
        assert!(expansions.contains(&"get_crypto_quotes".to_string()));
        assert!(expansions.contains(&"domains".to_string()));
        assert!(expand_symbols(&symbols, "cry dom").is_empty());
    }
}
