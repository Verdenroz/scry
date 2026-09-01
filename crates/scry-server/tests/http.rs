use std::sync::Arc;

use scry_core::config::Config;
use scry_core::embed::HashEmbedder;
use scry_core::store::Store;
use scry_server::api::{
    FileUpload, ManifestRequest, SearchRequest, SearchResponse, StatusResponse, SyncRequest,
    SyncResponse,
};
use scry_server::{AppState, StoreHandle, router};

const TOKEN: &str = "test-token";
const REPO: &str = "github.com/test/http";

async fn spawn_server() -> String {
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
            repo_key: REPO.to_string(),
            query: "load configuration".to_string(),
            limit: 5,
            path_prefix: None,
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

    let missing_repo = http
        .post(format!("{base}/v1/search"))
        .bearer_auth(TOKEN)
        .json(&SearchRequest {
            repo_key: "github.com/none/none".to_string(),
            query: "x".to_string(),
            limit: 5,
            path_prefix: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(missing_repo.status(), 404);
}
