use std::path::Path;

use scry_core::config::IndexConfig;
use scry_core::embed::{Embedder, HashEmbedder};
use scry_core::index::index_repo;
use scry_core::memory::{AnchorSpec, MemoryDraft, recall_with_vector, remember};
use scry_core::store::Store;

const REPO_KEY: &str = "github.com/test/memory";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn draft(repo_id: i64, content: &str, pain: Option<f64>, anchor: Option<&str>) -> MemoryDraft {
    MemoryDraft {
        repo_id: Some(repo_id),
        kind: "lesson".to_string(),
        content: content.to_string(),
        pain,
        cost: None,
        anchors: anchor
            .map(|relpath| AnchorSpec {
                relpath: relpath.to_string(),
                start_line: None,
                end_line: None,
            })
            .into_iter()
            .collect(),
    }
}

#[tokio::test]
async fn remember_recall_staleness_and_feedback() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "src/parser.rs",
        "pub fn parse_settings(text: &str) -> u16 {\n    text.trim().parse().unwrap()\n}\n",
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

    let (id, salience, _surprise) = remember(
        &mut store,
        &embedder,
        draft(
            repo_id,
            "parse_settings panics on non-numeric config files; validate before parsing",
            Some(8.0),
            Some("src/parser.rs"),
        ),
    )
    .await
    .unwrap();
    assert!(salience > 0.5);

    let query_vec = embedder
        .embed(&["parse settings config panics".to_string()])
        .await
        .unwrap()
        .pop()
        .unwrap();
    let hits = recall_with_vector(&store, Some(repo_id), &query_vec, 5, 29.0).unwrap();
    assert_eq!(hits[0].id, id);
    assert!(!hits[0].stale);
    let live_score = hits[0].score;

    write(
        dir.path(),
        "src/parser.rs",
        "pub fn parse_settings(text: &str) -> u16 {\n    text.trim().parse().unwrap_or(0)\n}\n",
    );
    index_repo(
        &mut store,
        &embedder,
        REPO_KEY,
        dir.path(),
        &IndexConfig::default(),
    )
    .await
    .unwrap();

    let hits = recall_with_vector(&store, Some(repo_id), &query_vec, 5, 29.0).unwrap();
    assert_eq!(hits[0].id, id);
    assert!(hits[0].stale);
    assert!(hits[0].score < live_score);

    assert!(store.memory_feedback(id, true).unwrap());
    let hits = recall_with_vector(&store, Some(repo_id), &query_vec, 5, 29.0).unwrap();
    assert!(hits[0].score > 0.0);

    let (total, stale) = store.memory_counts().unwrap();
    assert_eq!((total, stale), (1, 1));
}

#[tokio::test]
async fn surprise_drops_for_near_duplicates() {
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    let content = "always run migrations before deploying the finance service";
    let first = MemoryDraft {
        repo_id: None,
        kind: "convention".to_string(),
        content: content.to_string(),
        pain: None,
        cost: None,
        anchors: vec![],
    };
    let (_, _, surprise_first) = remember(&mut store, &embedder, first.clone())
        .await
        .unwrap();
    let (_, _, surprise_dup) = remember(&mut store, &embedder, first).await.unwrap();
    assert!(surprise_first > 0.9);
    assert!(surprise_dup < 0.1);
}

#[tokio::test]
async fn rejects_unknown_kind_and_unindexed_anchor() {
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    let bad_kind = MemoryDraft {
        repo_id: None,
        kind: "vibe".to_string(),
        content: "x".to_string(),
        pain: None,
        cost: None,
        anchors: vec![],
    };
    assert!(remember(&mut store, &embedder, bad_kind).await.is_err());

    let repo_id = store.upsert_repo(REPO_KEY).unwrap();
    let bad_anchor = draft(repo_id, "x", None, Some("src/missing.rs"));
    let err = remember(&mut store, &embedder, bad_anchor)
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("not in the index"));
}
