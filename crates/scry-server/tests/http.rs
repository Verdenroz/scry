use std::sync::Arc;

use scry_core::config::Config;
use scry_core::config::{RerankConfig, VerifyConfig};
use scry_core::embed::HashEmbedder;
use scry_core::rerank::RerankClient;
use scry_core::store::Store;
use scry_core::verify::Verifier;
use scry_server::api::{
    BriefResponse, FeedbackResponse, FileUpload, GcResponse, ManifestRequest, MemoryStatsResponse,
    RecallResponse, RememberResponse, RestoreResponse, SearchRequest, SearchResponse,
    StaleMemoryDto, StaleResponse, StatusResponse, SupersedeResponse, SyncRequest, SyncResponse,
    VerifyResponse,
};
use scry_server::api::{WorkItemResponse, WorkListResponse};
use scry_server::{AppState, QueryCache, StoreHandle, router};

const TOKEN: &str = "test-token";
const REPO: &str = "github.com/test/http";

async fn spawn_server() -> String {
    spawn_server_with(None, None).await
}

async fn spawn_server_with(rerank: Option<RerankClient>, verify: Option<Verifier>) -> String {
    let store = Store::open_in_memory("hash-test", 128).unwrap();
    let config: Config = toml::from_str(&format!(
        "[server]\nauth_token = \"{TOKEN}\"\n[embedding]\ndim = 128\nmodel = \"hash-test\"\n"
    ))
    .unwrap();
    let state = AppState {
        store: StoreHandle::spawn(store),
        embedder: Box::new(HashEmbedder { dim: 128 }),
        chat: None,
        hyde: config.search.hyde,
        auth_token: config.server.auth_token.clone(),
        index_config: config.index.clone(),
        memory_config: config.memory.clone(),
        tavily: None,
        rerank,
        verify,
        query_cache: QueryCache::new(8),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, router(Arc::new(state)))
            .await
            .unwrap();
    });
    base
}

fn upload(relpath: &str, content: &str) -> FileUpload {
    FileUpload {
        relpath: relpath.to_string(),
        content: content.to_string(),
        xxh64: scry_core::hashing::hex(scry_core::hashing::hash_bytes(content.as_bytes())),
        size: content.len() as u64,
        mtime_ms: 1_700_000_000_000,
    }
}

#[tokio::test]
async fn auth_sync_search_roundtrip() {
    let base = spawn_server().await;
    let http = reqwest::Client::new();

    let health = http.get(format!("{base}/health")).send().await.unwrap();
    assert!(health.status().is_success());

    let unauthorized = http.get(format!("{base}/v1/status")).send().await.unwrap();
    assert_eq!(unauthorized.status(), 401);

    let sync: SyncResponse = http
        .post(format!("{base}/v1/sync"))
        .bearer_auth(TOKEN)
        .json(&SyncRequest {
            repo_key: REPO.to_string(),
            upserts: vec![
                upload(
                    "src/config.rs",
                    "pub fn load_configuration(path: &str) -> u16 {\n    path.len() as u16\n}\n",
                ),
                upload(
                    "src/net.rs",
                    "pub fn send_request(url: &str) -> usize {\n    url.len()\n}\n",
                ),
            ],
            deletes: vec![],
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sync.indexed_files, 2);
    assert!(sync.embedded_chunks > 0);

    let status: StatusResponse = http
        .get(format!("{base}/v1/status"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status.repos, 1);
    assert_eq!(status.files, 2);

    let manifest: scry_server::api::ManifestResponse = http
        .post(format!("{base}/v1/manifest"))
        .bearer_auth(TOKEN)
        .json(&ManifestRequest {
            repo_key: REPO.to_string(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(manifest.files.len(), 2);

    let search: SearchResponse = http
        .post(format!("{base}/v1/search"))
        .bearer_auth(TOKEN)
        .json(&SearchRequest {
            repo_key: Some(REPO.to_string()),
            query: "load configuration".to_string(),
            limit: 5,
            path_prefix: None,
            rerank: true,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(search.hits[0].relpath, "src/config.rs");

    let global: SearchResponse = http
        .post(format!("{base}/v1/search"))
        .bearer_auth(TOKEN)
        .json(&SearchRequest {
            repo_key: None,
            query: "load configuration".to_string(),
            limit: 5,
            path_prefix: None,
            rerank: true,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(global.hits[0].repo_key, REPO);

    let missing_repo = http
        .post(format!("{base}/v1/search"))
        .bearer_auth(TOKEN)
        .json(&SearchRequest {
            repo_key: Some("github.com/none/none".to_string()),
            query: "x".to_string(),
            limit: 5,
            path_prefix: None,
            rerank: true,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(missing_repo.status(), 404);
}

/// Scores each document by its position, so the last candidate wins.
async fn spawn_mock_reranker() -> String {
    let app = axum::Router::new().route(
        "/v1/rerank",
        axum::routing::post(
            |axum::Json(body): axum::Json<serde_json::Value>| async move {
                let n = body["documents"].as_array().map_or(0, |d| d.len());
                let results: Vec<serde_json::Value> = (0..n)
                    .map(|i| serde_json::json!({ "index": i, "relevance_score": i as f64 }))
                    .collect();
                axum::Json(serde_json::json!({ "results": results }))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

async fn search(http: &reqwest::Client, base: &str, rerank: bool) -> Vec<String> {
    let response: SearchResponse = http
        .post(format!("{base}/v1/search"))
        .bearer_auth(TOKEN)
        .json(&SearchRequest {
            repo_key: Some(REPO.to_string()),
            query: "load configuration".to_string(),
            limit: 2,
            path_prefix: None,
            rerank,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    response.hits.into_iter().map(|h| h.relpath).collect()
}

#[tokio::test]
async fn rerank_stage_reorders_and_no_rerank_bypasses_it() {
    let reranker = spawn_mock_reranker().await;
    let config: RerankConfig = toml::from_str(&format!(
        "base_url = \"{reranker}\"\nmodel = \"mock\"\ntop_n = 2\nweight = 2.0\n"
    ))
    .unwrap();
    let base = spawn_server_with(Some(RerankClient::new(config)), None).await;
    let http = reqwest::Client::new();
    http.post(format!("{base}/v1/sync"))
        .bearer_auth(TOKEN)
        .json(&SyncRequest {
            repo_key: REPO.to_string(),
            upserts: vec![
                upload(
                    "src/config.rs",
                    "pub fn load_configuration(path: &str) -> u16 {\n    path.len() as u16\n}\n",
                ),
                upload(
                    "src/net.rs",
                    "pub fn send_request(url: &str) -> usize {\n    url.len()\n}\n",
                ),
            ],
            deletes: vec![],
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let plain = search(&http, &base, false).await;
    let reranked = search(&http, &base, true).await;
    assert_eq!(plain[0], "src/config.rs");
    assert_eq!(reranked.len(), 2);
    assert_eq!(reranked[0], plain[1]);
    assert_eq!(reranked[1], plain[0]);
}

#[tokio::test]
async fn memory_brief_supersede_and_stats() {
    let base = spawn_server().await;
    let http = reqwest::Client::new();
    let remember = |body: serde_json::Value| {
        http.post(format!("{base}/v1/memories/remember"))
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
    };

    let scope = "path:~/notes";
    let first: RememberResponse = remember(serde_json::json!({
        "repo_key": scope, "kind": "lesson", "content": "keep notes terse",
        "source": "capture", "evidence": "session 4"
    }))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let second: RememberResponse = remember(serde_json::json!({
        "repo_key": scope, "kind": "lesson", "content": "keep notes very terse"
    }))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let recall = |touch: bool| {
        http.post(format!("{base}/v1/memories/recall"))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({ "repo_key": scope, "query": "terse notes", "touch": touch }))
            .send()
    };
    recall(false).await.unwrap();
    let stats = memory_stats(&http, &base).await;
    assert_eq!(stats.sources[0].retrievals, 0);
    recall(true).await.unwrap();
    let stats = memory_stats(&http, &base).await;
    assert!(stats.sources[0].retrievals > 0);

    let bad = remember(serde_json::json!({
        "repo_key": scope, "kind": "lesson", "content": "x", "source": "oracle"
    }))
    .await
    .unwrap();
    assert_eq!(bad.status(), 404);

    let brief = |repo_key: Option<&str>| {
        http.post(format!("{base}/v1/memories/brief"))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({ "repo_key": repo_key }))
            .send()
    };
    let digest: BriefResponse = brief(Some(scope)).await.unwrap().json().await.unwrap();
    assert_eq!(digest.memories.len(), 2);
    assert!(digest.went_stale.is_empty());
    let elsewhere: BriefResponse = brief(Some("path:~/other"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(elsewhere.memories.is_empty());

    let feedback = |id: i64, helpful: bool| {
        http.post(format!("{base}/v1/memories/feedback"))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({ "id": id, "helpful": helpful, "repo_key": scope }))
            .send()
    };
    feedback(first.id, false).await.unwrap();
    let digest: BriefResponse = brief(Some(scope)).await.unwrap().json().await.unwrap();
    assert_eq!(digest.memories.len(), 1);
    feedback(first.id, true).await.unwrap();
    let digest: BriefResponse = brief(Some(scope)).await.unwrap().json().await.unwrap();
    assert_eq!(digest.memories.len(), 2);

    let supersede = |old_id: i64, new_id: i64| {
        http.post(format!("{base}/v1/memories/supersede"))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({ "old_id": old_id, "new_id": new_id }))
            .send()
    };
    let missing: SupersedeResponse = supersede(first.id, 999)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!missing.superseded);
    let done: SupersedeResponse = supersede(first.id, second.id)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(done.superseded);
    let digest: BriefResponse = brief(Some(scope)).await.unwrap().json().await.unwrap();
    assert_eq!(digest.memories.len(), 1);
    assert_eq!(digest.memories[0].id, second.id);

    let stats = memory_stats(&http, &base).await;
    assert_eq!(stats.sources.len(), 1);
    assert_eq!(stats.sources[0].source, "agent");
    assert_eq!(stats.sources[0].memories, 1);
}

#[tokio::test]
async fn feedback_on_a_missing_memory_creates_no_repo() {
    let base = spawn_server().await;
    let http = reqwest::Client::new();
    let response: FeedbackResponse = post_json(
        &http,
        &base,
        "/v1/memories/feedback",
        serde_json::json!({ "id": 999, "helpful": false, "repo_key": "github.com/test/phantom" }),
    )
    .await
    .json()
    .await
    .unwrap();
    assert!(!response.updated);
    let status: StatusResponse = http
        .get(format!("{base}/v1/status"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status.repos, 0);
}

#[tokio::test]
async fn a_helpful_vote_from_an_unseen_scope_creates_no_repo() {
    let base = spawn_server().await;
    let http = reqwest::Client::new();
    let remembered: RememberResponse = post_json(
        &http,
        &base,
        "/v1/memories/remember",
        serde_json::json!({ "kind": "lesson", "content": "global fact" }),
    )
    .await
    .json()
    .await
    .unwrap();
    let response: FeedbackResponse = post_json(
        &http,
        &base,
        "/v1/memories/feedback",
        serde_json::json!({
            "id": remembered.id, "helpful": true, "repo_key": "github.com/test/phantom"
        }),
    )
    .await
    .json()
    .await
    .unwrap();
    assert!(response.updated);
    let status: StatusResponse = http
        .get(format!("{base}/v1/status"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status.repos, 0);
}

async fn memory_stats(http: &reqwest::Client, base: &str) -> MemoryStatsResponse {
    http.get(format!("{base}/v1/memories/stats"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Answers "yes" with P=0.8 when the prompt mentions SOLID, else P=0.1.
async fn spawn_mock_verifier() -> String {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(
            |axum::Json(body): axum::Json<serde_json::Value>| async move {
                let prompt = body["messages"][0]["content"].as_str().unwrap_or_default();
                let yes: f64 = if prompt.contains("SOLID") { 0.8 } else { 0.1 };
                axum::Json(serde_json::json!({ "choices": [{
                    "message": { "content": "yes" },
                    "logprobs": { "content": [{ "top_logprobs": [
                        { "token": "yes", "logprob": yes.ln() },
                        { "token": "no", "logprob": (1.0 - yes).ln() },
                    ]}]},
                }]}))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

async fn verify(http: &reqwest::Client, base: &str, evidence: &str) -> VerifyResponse {
    http.post(format!("{base}/v1/memories/verify"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({ "claim": "c", "evidence": evidence }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn verify_scores_against_the_server_threshold() {
    let http = reqwest::Client::new();
    let unconfigured = verify(&http, &spawn_server().await, "x").await;
    assert!(!unconfigured.configured);
    assert!(unconfigured.score.is_none() && unconfigured.supported.is_none());

    let model = spawn_mock_verifier().await;
    let config: VerifyConfig = toml::from_str(&format!(
        "base_url = \"{model}\"\nmodel = \"mock\"\nthreshold = 0.5\n"
    ))
    .unwrap();
    let base = spawn_server_with(None, Some(Verifier::new(config))).await;
    let good = verify(&http, &base, "SOLID evidence").await;
    assert!((good.score.unwrap() - 0.8).abs() < 1e-9);
    assert_eq!(good.supported, Some(true));
    let bad = verify(&http, &base, "vague evidence").await;
    assert!((bad.score.unwrap() - 0.1).abs() < 1e-9);
    assert_eq!(bad.supported, Some(false));
}

async fn post_json(
    http: &reqwest::Client,
    base: &str,
    path: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    http.post(format!("{base}{path}"))
        .bearer_auth(TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn sync_file(http: &reqwest::Client, base: &str, content: &str) {
    http.post(format!("{base}/v1/sync"))
        .bearer_auth(TOKEN)
        .json(&SyncRequest {
            repo_key: REPO.to_string(),
            upserts: vec![upload("src/a.rs", content)],
            deletes: vec![],
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}

async fn stale_list(http: &reqwest::Client, base: &str, files: &[&str]) -> Vec<StaleMemoryDto> {
    let response: StaleResponse = post_json(
        http,
        base,
        "/v1/memories/stale",
        serde_json::json!({ "repo_key": REPO, "files": files }),
    )
    .await
    .json()
    .await
    .unwrap();
    response.memories
}

async fn gc(http: &reqwest::Client, base: &str, dry_run: bool) -> GcResponse {
    post_json(
        http,
        base,
        "/v1/memories/gc",
        serde_json::json!({ "dry_run": dry_run }),
    )
    .await
    .json()
    .await
    .unwrap()
}

#[tokio::test]
async fn reaffirm_with_anchors_requires_them_indexed() {
    let base = spawn_server().await;
    let http = reqwest::Client::new();
    sync_file(&http, &base, "pub fn a() {}\n").await;
    let remembered: RememberResponse = post_json(
        &http,
        &base,
        "/v1/memories/remember",
        serde_json::json!({
            "repo_key": REPO, "kind": "lesson", "content": "a is special",
            "anchors": [{ "relpath": "src/a.rs" }]
        }),
    )
    .await
    .json()
    .await
    .unwrap();
    sync_file(&http, &base, "pub fn a() { 1; }\n").await;

    let reaffirm = |relpath: &str| {
        post_json(
            &http,
            &base,
            "/v1/memories/reaffirm",
            serde_json::json!({
                "id": remembered.id, "evidence": "moved",
                "anchors": [{ "relpath": relpath, "start_line": 1, "end_line": 1 }]
            }),
        )
    };
    assert_eq!(reaffirm("src/missing.rs").await.status(), 404);
    assert_eq!(stale_list(&http, &base, &[]).await.len(), 1);
    assert_eq!(reaffirm("src/a.rs").await.status(), 200);
    assert!(stale_list(&http, &base, &[]).await.is_empty());
}

#[tokio::test]
async fn memory_stale_reaffirm_gc_and_restore() {
    let base = spawn_server().await;
    let http = reqwest::Client::new();
    sync_file(&http, &base, "pub fn a() {}\n").await;

    let remembered: RememberResponse = post_json(
        &http,
        &base,
        "/v1/memories/remember",
        serde_json::json!({
            "repo_key": REPO, "kind": "lesson", "content": "a is special",
            "anchors": [{ "relpath": "src/a.rs" }]
        }),
    )
    .await
    .json()
    .await
    .unwrap();
    assert!(stale_list(&http, &base, &[]).await.is_empty());

    let reaffirm = |id: i64| {
        post_json(
            &http,
            &base,
            "/v1/memories/reaffirm",
            serde_json::json!({ "id": id, "evidence": "read the new body" }),
        )
    };
    assert_eq!(reaffirm(remembered.id).await.status(), 404);

    sync_file(&http, &base, "pub fn a() { 1; }\n").await;
    let listed = stale_list(&http, &base, &[]).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].relpaths, vec!["src/a.rs".to_string()]);
    assert_eq!(stale_list(&http, &base, &["src/b.rs"]).await.len(), 0);
    assert_eq!(stale_list(&http, &base, &["src/a.rs"]).await.len(), 1);

    assert_eq!(reaffirm(remembered.id).await.status(), 200);
    assert!(stale_list(&http, &base, &[]).await.is_empty());

    let noisy = remembered.id;
    for scope in [REPO, "github.com/test/other"] {
        post_json(
            &http,
            &base,
            "/v1/memories/feedback",
            serde_json::json!({ "id": noisy, "helpful": false, "repo_key": scope }),
        )
        .await;
    }
    assert_eq!(gc(&http, &base, true).await.noise, 1);
    assert_eq!(memory_stats(&http, &base).await.sources[0].archived, 0);
    assert_eq!(gc(&http, &base, false).await.noise, 1);
    let stats = memory_stats(&http, &base).await;
    assert_eq!(
        (stats.sources[0].memories, stats.sources[0].archived),
        (0, 1)
    );

    let restored: RestoreResponse = post_json(
        &http,
        &base,
        "/v1/memories/restore",
        serde_json::json!({ "id": noisy }),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(restored.status, "live");
    let again = post_json(
        &http,
        &base,
        "/v1/memories/restore",
        serde_json::json!({ "id": noisy }),
    )
    .await;
    assert_eq!(again.status(), 404);
    let recalled: RecallResponse = post_json(
        &http,
        &base,
        "/v1/memories/recall",
        serde_json::json!({ "repo_key": REPO, "query": "a is special" }),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(recalled.memories.len(), 1);
}

#[tokio::test]
async fn work_lists_round_trip() {
    let base = spawn_server().await;
    let http = reqwest::Client::new();
    let lists = |all: bool| {
        let http = http.clone();
        let base = base.clone();
        async move {
            post_json(
                &http,
                &base,
                "/v1/work/lists",
                serde_json::json!({ "repo_key": REPO, "all": all }),
            )
            .await
            .json::<WorkListResponse>()
            .await
            .unwrap()
            .lists
        }
    };
    assert!(lists(false).await.is_empty());

    let added: WorkItemResponse = post_json(
        &http,
        &base,
        "/v1/work/add",
        serde_json::json!({ "repo_key": REPO, "list": "ship", "task": "write tests" }),
    )
    .await
    .json()
    .await
    .unwrap();
    let bad = post_json(
        &http,
        &base,
        "/v1/work/add",
        serde_json::json!({ "repo_key": REPO, "list": "ship", "task": "t", "status": "blocked" }),
    )
    .await;
    assert_eq!(bad.status(), 404);
    post_json(
        &http,
        &base,
        "/v1/work/state",
        serde_json::json!({ "repo_key": REPO, "list": "ship", "entries": [["pr", "#12"]] }),
    )
    .await;
    let open = lists(false).await;
    assert_eq!(open[0].items[0].status, "todo");
    assert_eq!(open[0].state, vec![("pr".to_string(), "#12".to_string())]);

    let set = post_json(
        &http,
        &base,
        "/v1/work/set",
        serde_json::json!({ "repo_key": REPO, "id": added.id, "status": "done", "note": "green" }),
    )
    .await;
    assert_eq!(set.status(), 200);
    assert!(lists(false).await[0].items.is_empty());

    let close = |list: &str| {
        post_json(
            &http,
            &base,
            "/v1/work/close",
            serde_json::json!({ "repo_key": REPO, "list": list }),
        )
    };
    assert_eq!(close("ship").await.status(), 200);
    assert_eq!(close("missing").await.status(), 404);
    assert!(lists(false).await.is_empty());
    assert_eq!(lists(true).await[0].items[0].note.as_deref(), Some("green"));
}
