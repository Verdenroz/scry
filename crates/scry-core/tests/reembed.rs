use std::path::Path;

use scry_core::config::{HydeMode, IndexConfig};
use scry_core::embed::{Embedder, HashEmbedder};
use scry_core::index::index_repo;
use scry_core::memory::{MemoryDraft, recall_with_vector, remember};
use scry_core::reembed;
use scry_core::search::{SearchOptions, hybrid_search};
use scry_core::store::{Scope, Store};

const REPO_KEY: &str = "github.com/test/reembed";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn lesson(repo_id: i64, content: &str) -> MemoryDraft {
    MemoryDraft {
        repo_id: Some(repo_id),
        kind: "lesson".to_string(),
        content: content.to_string(),
        pain: None,
        cost: None,
        source: "agent".to_string(),
        evidence: None,
        anchors: Vec::new(),
    }
}

async fn seeded_db(db: &Path, repo: &Path) -> (usize, i64) {
    write(
        repo,
        "src/config.rs",
        "pub fn load_configuration(path: &str) -> String {\n    std::fs::read_to_string(path).unwrap()\n}\n",
    );
    write(
        repo,
        "src/net.rs",
        "pub fn retry_delay(attempt: u32) -> u64 {\n    500 << attempt\n}\n",
    );
    let mut store = Store::open(db, "hash-old", 64).unwrap();
    let embedder = HashEmbedder { dim: 64 };
    index_repo(
        &mut store,
        &embedder,
        REPO_KEY,
        repo,
        &IndexConfig::default(),
    )
    .await
    .unwrap();
    let repo_id = store.repo_id(REPO_KEY).unwrap().unwrap();
    let (memory_id, _, _) = remember(
        &mut store,
        &embedder,
        lesson(repo_id, "retry delay doubles with every attempt"),
    )
    .await
    .unwrap();
    (store.counts().unwrap().2 as usize, memory_id)
}

#[tokio::test]
async fn reembed_moves_every_vector_to_the_new_model() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let backup = dir.path().join("index.db.bak");
    let (chunk_count, memory_id) = seeded_db(&db, &dir.path().join("repo")).await;

    let embedder = HashEmbedder { dim: 32 };
    let outcome = reembed::run(&db, &backup, &embedder, "hash-new", "", |_, _| ())
        .await
        .unwrap();
    assert_eq!(outcome.previous_model, "hash-old");
    assert_eq!(outcome.previous_dim, 64);
    assert_eq!(outcome.chunks, chunk_count);
    assert_eq!(outcome.memories, 1);

    let err = Store::open(&db, "hash-old", 64).err().unwrap();
    assert!(err.to_string().contains("embedding_model"));
    let store = Store::open(&db, "hash-new", 32).unwrap();
    assert_eq!(store.counts().unwrap().2 as usize, chunk_count);

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
        "retry delay attempt",
        &options,
    )
    .await
    .unwrap();
    assert_eq!(hits[0].relpath, "src/net.rs");

    let query = embedder
        .embed_queries(&["retry delay attempt".to_string()])
        .await
        .unwrap();
    let memories =
        recall_with_vector(&store, Scope::Repo(repo_id), &query[0], 5, 0.0, false).unwrap();
    assert_eq!(memories[0].id, memory_id);
}

#[tokio::test]
async fn reembed_keeps_a_backup_that_opens_under_the_old_model() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let backup = dir.path().join("index.db.bak");
    let (chunk_count, _) = seeded_db(&db, &dir.path().join("repo")).await;

    reembed::run(
        &db,
        &backup,
        &HashEmbedder { dim: 32 },
        "hash-new",
        "",
        |_, _| (),
    )
    .await
    .unwrap();

    let old = Store::open(&backup, "hash-old", 64).unwrap();
    assert_eq!(old.counts().unwrap().2 as usize, chunk_count);
}

#[tokio::test]
async fn reembed_refuses_to_overwrite_an_existing_backup() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let backup = dir.path().join("index.db.bak");
    seeded_db(&db, &dir.path().join("repo")).await;
    std::fs::write(&backup, b"keep me").unwrap();

    assert!(
        reembed::run(
            &db,
            &backup,
            &HashEmbedder { dim: 32 },
            "hash-new",
            "",
            |_, _| ()
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&backup).unwrap(), b"keep me");
    Store::open(&db, "hash-old", 64).unwrap();
}

#[tokio::test]
async fn reembed_backs_up_before_migrating_an_older_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let backup = dir.path().join("index.db.bak");
    seeded_db(&db, &dir.path().join("repo")).await;
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "ALTER TABLE memories DROP COLUMN contradicted_at;
             UPDATE meta SET value = '7' WHERE key = 'schema_version';",
        )
        .unwrap();

    reembed::run(
        &db,
        &backup,
        &HashEmbedder { dim: 32 },
        "hash-new",
        "",
        |_, _| (),
    )
    .await
    .unwrap();

    let version: String = rusqlite::Connection::open(&backup)
        .unwrap()
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "7");
}

#[tokio::test]
async fn reembed_stamps_the_document_prefix_the_server_then_requires() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let backup = dir.path().join("index.db.bak");
    seeded_db(&db, &dir.path().join("repo")).await;
    let store = Store::open(&db, "hash-old", 64).unwrap();
    assert!(store.guard_document_prefix("doc: ").is_err());
    drop(store);

    reembed::run(
        &db,
        &backup,
        &HashEmbedder { dim: 32 },
        "hash-new",
        "doc: ",
        |_, _| (),
    )
    .await
    .unwrap();

    let store = Store::open(&db, "hash-new", 32).unwrap();
    store.guard_document_prefix("doc: ").unwrap();
    let err = store.guard_document_prefix("").err().unwrap();
    assert!(err.to_string().contains("document_prefix"));
}

#[test]
fn a_fresh_db_takes_any_document_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    Store::open(&db, "hash-new", 32)
        .unwrap()
        .guard_document_prefix("doc: ")
        .unwrap();
    let store = Store::open(&db, "hash-new", 32).unwrap();
    assert!(store.guard_document_prefix("").is_err());
}

#[tokio::test]
async fn replacing_vectors_fails_when_the_db_changed_since_it_was_read() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let repo = dir.path().join("repo");
    seeded_db(&db, &repo).await;
    let mut store = Store::open_for_reembed(&db).unwrap();
    let chunks = store.chunk_sources().unwrap();
    let memories = store.memory_sources().unwrap();

    write(
        &repo,
        "src/late.rs",
        "pub fn arrived_late() -> bool {\n    true\n}\n",
    );
    let mut writer = Store::open(&db, "hash-old", 64).unwrap();
    index_repo(
        &mut writer,
        &HashEmbedder { dim: 64 },
        REPO_KEY,
        &repo,
        &IndexConfig::default(),
    )
    .await
    .unwrap();
    drop(writer);

    let chunks: Vec<_> = chunks.into_iter().map(|c| (c, vec![0.5; 32])).collect();
    let memories: Vec<_> = memories.into_iter().map(|m| (m, vec![0.5; 32])).collect();
    let err = store
        .replace_all_vectors("hash-new", 32, "", &chunks, &memories)
        .err()
        .unwrap();
    assert!(err.to_string().contains("changed"));
    drop(store);
    Store::open(&db, "hash-old", 64).unwrap();
}
