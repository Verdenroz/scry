use std::path::Path;

use scry_core::config::IndexConfig;
use scry_core::embed::{Embedder, HashEmbedder};
use scry_core::index::index_repo;
use scry_core::memory::{AnchorSpec, MemoryDraft, brief, recall_with_vector, remember};
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
        source: "agent".to_string(),
        evidence: None,
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
    let hits = recall_with_vector(&store, Some(repo_id), &query_vec, 5, 29.0, true).unwrap();
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

    let hits = recall_with_vector(&store, Some(repo_id), &query_vec, 5, 29.0, true).unwrap();
    assert_eq!(hits[0].id, id);
    assert!(hits[0].stale);
    assert!(hits[0].score < live_score);

    assert!(store.memory_feedback(id, true, Some(repo_id)).unwrap());
    let hits = recall_with_vector(&store, Some(repo_id), &query_vec, 5, 29.0, true).unwrap();
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
        source: "agent".to_string(),
        evidence: None,
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
        source: "agent".to_string(),
        evidence: None,
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

#[tokio::test]
async fn brief_ranks_scope_and_globals_without_touching_access() {
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    let repo_id = store.upsert_repo(REPO_KEY).unwrap();
    let other = store.upsert_repo("github.com/test/other").unwrap();
    let strong = draft(repo_id, "strong lesson about retries", Some(10.0), None);
    let weak = draft(repo_id, "weak note about naming", Some(0.0), None);
    let foreign = draft(other, "lesson from another repo", Some(10.0), None);
    let global = MemoryDraft {
        repo_id: None,
        ..draft(repo_id, "global convention about commits", Some(5.0), None)
    };
    let mut ids = Vec::new();
    for d in [strong, weak, foreign, global] {
        ids.push(remember(&mut store, &embedder, d).await.unwrap().0);
    }

    let digest = brief(&store, Some(repo_id), true, 8, 7, 29.0).unwrap();
    let got: Vec<i64> = digest.top.iter().map(|h| h.id).collect();
    assert_eq!(got.len(), 3);
    assert!(!got.contains(&ids[2]));
    assert_eq!(got[0], ids[0]);
    assert!(digest.went_stale.is_empty());
    assert_eq!(
        brief(&store, Some(repo_id), true, 1, 7, 29.0)
            .unwrap()
            .top
            .len(),
        1
    );

    let rows = store.live_memories(Some(repo_id)).unwrap();
    assert!(rows.iter().all(|row| row.access_count == 0));
}

#[tokio::test]
async fn brief_lists_recently_stale_and_rejects_unknown_source() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "pub fn a() {}\n");
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
    let id = remember(
        &mut store,
        &embedder,
        draft(repo_id, "a is special", None, Some("src/a.rs")),
    )
    .await
    .unwrap()
    .0;
    write(dir.path(), "src/a.rs", "pub fn a() { 1; }\n");
    index_repo(
        &mut store,
        &embedder,
        REPO_KEY,
        dir.path(),
        &IndexConfig::default(),
    )
    .await
    .unwrap();

    let digest = brief(&store, Some(repo_id), true, 8, 7, 29.0).unwrap();
    assert!(digest.top.is_empty());
    assert_eq!(digest.went_stale.len(), 1);
    assert_eq!(digest.went_stale[0].id, id);
    assert!(
        brief(&store, Some(repo_id), true, 8, 0, 29.0)
            .unwrap()
            .went_stale
            .len()
            <= 1
    );

    let bad = MemoryDraft {
        source: "oracle".to_string(),
        ..draft(repo_id, "x", None, None)
    };
    assert!(remember(&mut store, &embedder, bad).await.is_err());
}

#[tokio::test]
async fn untouched_recall_leaves_access_count_alone() {
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    let repo_id = store.upsert_repo(REPO_KEY).unwrap();
    remember(
        &mut store,
        &embedder,
        draft(repo_id, "retry with backoff", None, None),
    )
    .await
    .unwrap();
    let query_vec = embedder
        .embed(&["retry backoff".to_string()])
        .await
        .unwrap()
        .pop()
        .unwrap();

    recall_with_vector(&store, Some(repo_id), &query_vec, 5, 29.0, false).unwrap();
    assert_eq!(
        store.live_memories(Some(repo_id)).unwrap()[0].access_count,
        0
    );
    recall_with_vector(&store, Some(repo_id), &query_vec, 5, 29.0, true).unwrap();
    assert_eq!(
        store.live_memories(Some(repo_id)).unwrap()[0].access_count,
        1
    );
}

#[tokio::test]
async fn scoped_brief_caps_globals() {
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    let repo_id = store.upsert_repo(REPO_KEY).unwrap();
    let mut global_ids = Vec::new();
    for i in 0..3 {
        let d = MemoryDraft {
            repo_id: None,
            ..draft(repo_id, &format!("global lesson {i}"), Some(10.0), None)
        };
        global_ids.push(remember(&mut store, &embedder, d).await.unwrap().0);
    }
    for i in 0..3 {
        let d = draft(repo_id, &format!("scope lesson {i}"), Some(0.0), None);
        remember(&mut store, &embedder, d).await.unwrap();
    }

    let scoped = brief(&store, Some(repo_id), true, 4, 7, 29.0).unwrap();
    let globals = scoped
        .top
        .iter()
        .filter(|h| global_ids.contains(&h.id))
        .count();
    assert_eq!((scoped.top.len(), globals), (4, 2));
    let unscoped = brief(&store, None, false, 4, 7, 29.0).unwrap();
    assert_eq!(unscoped.top.len(), 3);
}

#[tokio::test]
async fn brief_for_a_scope_without_rows_still_caps_globals() {
    let mut store = Store::open_in_memory("hash-test", 256).unwrap();
    let embedder = HashEmbedder { dim: 256 };
    for i in 0..3 {
        let d = MemoryDraft {
            repo_id: None,
            ..draft(0, &format!("global lesson {i}"), None, None)
        };
        remember(&mut store, &embedder, d).await.unwrap();
    }
    assert_eq!(brief(&store, None, true, 8, 7, 29.0).unwrap().top.len(), 2);
    assert_eq!(brief(&store, None, false, 8, 7, 29.0).unwrap().top.len(), 3);
}
