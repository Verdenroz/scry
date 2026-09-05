use std::path::Path;

use scry_core::config::{HydeMode, IndexConfig};
use scry_core::embed::HashEmbedder;
use scry_core::index::index_repo;
use scry_core::search::{SearchOptions, hybrid_search};
use scry_core::store::Store;

const REPO_KEY: &str = "github.com/test/fixture";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn fixture_repo(root: &Path) {
    write(
        root,
        "src/config.rs",
        "pub struct Settings {\n    pub port: u16,\n    pub host: String,\n    pub verbose: bool,\n}\n\npub fn load_configuration(path: &str) -> Settings {\n    let text = std::fs::read_to_string(path).unwrap();\n    parse_settings(&text)\n}\n\nfn parse_settings(text: &str) -> Settings {\n    let port = text.trim().parse().unwrap();\n    let host = String::from(\"localhost\");\n    Settings { port, host, verbose: false }\n}\n",
    );
    write(
        root,
        "src/net.rs",
        "pub async fn send_request(url: &str) -> String {\n    reqwest::get(url).await.unwrap().text().await.unwrap()\n}\n\npub fn retry_delay(attempt: u32) -> u64 {\n    500 << attempt\n}\n",
    );
}

#[tokio::test]
async fn index_search_and_incremental_reindex() {
    let dir = tempfile::tempdir().unwrap();
    fixture_repo(dir.path());
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    let config = IndexConfig::default();

    let first = index_repo(&mut store, &embedder, REPO_KEY, dir.path(), &config)
        .await
        .unwrap();
    assert_eq!(first.indexed_files, 2);
    assert!(first.embedded_chunks > 0);

    let repo_id = store.repo_id(REPO_KEY).unwrap().unwrap();
    let options = SearchOptions {
        limit: 3,
        path_prefix: None,
    };
    let hits = hybrid_search(
        &store,
        &embedder,
        None,
        HydeMode::Off,
        Some(repo_id),
        "how is the configuration file loaded",
        &options,
    )
    .await
    .unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].relpath, "src/config.rs");
    assert!(hits[0].score > 0.0 && hits[0].score <= 1.0);
    assert!(hits[0].start_line >= 1);

    let second = index_repo(&mut store, &embedder, REPO_KEY, dir.path(), &config)
        .await
        .unwrap();
    assert_eq!(second.indexed_files, 0);
    assert_eq!(second.unchanged_files, 2);
    assert_eq!(second.embedded_chunks, 0);

    write(
        dir.path(),
        "src/config.rs",
        "pub struct Settings {\n    pub port: u16,\n    pub host: String,\n    pub verbose: bool,\n}\n\npub fn load_configuration(path: &str) -> Settings {\n    let text = std::fs::read_to_string(path).unwrap();\n    parse_settings(&text)\n}\n\nfn parse_settings(text: &str) -> Settings {\n    let port = text.trim().parse().unwrap_or(8080);\n    let host = String::from(\"localhost\");\n    Settings { port, host, verbose: false }\n}\n",
    );
    let third = index_repo(&mut store, &embedder, REPO_KEY, dir.path(), &config)
        .await
        .unwrap();
    assert_eq!(third.indexed_files, 1);
    assert!(third.reused_chunks > 0);
    assert!(third.embedded_chunks < first.embedded_chunks);
}

#[tokio::test]
async fn path_prefix_scopes_results() {
    let dir = tempfile::tempdir().unwrap();
    fixture_repo(dir.path());
    write(
        dir.path(),
        "docs/net.md",
        "sending requests with retries and delays\n",
    );
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    index_repo(
        &mut store,
        &embedder,
        REPO_KEY,
        dir.path(),
        &IndexConfig::default(),
    )
    .await
    .unwrap();

    let repo_id = store.repo_id(REPO_KEY).unwrap().unwrap();
    let options = SearchOptions {
        limit: 5,
        path_prefix: Some("src/".to_string()),
    };
    let hits = hybrid_search(
        &store,
        &embedder,
        None,
        HydeMode::Off,
        Some(repo_id),
        "send request retry delay",
        &options,
    )
    .await
    .unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit.relpath.starts_with("src/")));
}

#[tokio::test]
async fn deleting_a_file_removes_its_hits() {
    let dir = tempfile::tempdir().unwrap();
    fixture_repo(dir.path());
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    let config = IndexConfig::default();
    index_repo(&mut store, &embedder, REPO_KEY, dir.path(), &config)
        .await
        .unwrap();

    std::fs::remove_file(dir.path().join("src/net.rs")).unwrap();
    let outcome = index_repo(&mut store, &embedder, REPO_KEY, dir.path(), &config)
        .await
        .unwrap();
    assert_eq!(outcome.deleted_files, 1);

    let repo_id = store.repo_id(REPO_KEY).unwrap().unwrap();
    let hits = hybrid_search(
        &store,
        &embedder,
        None,
        HydeMode::Off,
        Some(repo_id),
        "send request retry delay",
        &SearchOptions {
            limit: 5,
            path_prefix: None,
        },
    )
    .await
    .unwrap();
    assert!(hits.iter().all(|hit| hit.relpath != "src/net.rs"));
}

#[tokio::test]
async fn global_search_spans_repos() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    fixture_repo(dir_a.path());
    write(
        dir_b.path(),
        "src/billing.rs",
        "pub fn calculate_invoice_total(items: &[u64]) -> u64 {\n    items.iter().sum()\n}\n",
    );
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    let config = IndexConfig::default();
    index_repo(&mut store, &embedder, REPO_KEY, dir_a.path(), &config)
        .await
        .unwrap();
    index_repo(
        &mut store,
        &embedder,
        "github.com/test/billing",
        dir_b.path(),
        &config,
    )
    .await
    .unwrap();

    let options = SearchOptions {
        limit: 3,
        path_prefix: None,
    };
    let hits = hybrid_search(
        &store,
        &embedder,
        None,
        HydeMode::Off,
        None,
        "calculate invoice total",
        &options,
    )
    .await
    .unwrap();
    assert_eq!(hits[0].repo_key, "github.com/test/billing");
    assert_eq!(hits[0].relpath, "src/billing.rs");

    let hits = hybrid_search(
        &store,
        &embedder,
        None,
        HydeMode::Off,
        None,
        "how is the configuration file loaded",
        &options,
    )
    .await
    .unwrap();
    assert_eq!(hits[0].repo_key, REPO_KEY);
}
