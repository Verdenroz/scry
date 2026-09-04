//! Hot paths measured by soothfast: content hashing runs on every file of
//! every sync, and remote-URL normalization on every repo detection.

use soothfast::{bench, fixture, keep};

soothfast::bench_main!();

/// Deterministic pseudo-random bytes (seeded LCG, no rand dep).
#[fixture]
fn bytes_n(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x5DEE_CE66_D111_1111;
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) as u8
        })
        .collect()
}

#[fixture]
fn remote_url() -> String {
    "ssh://git@github.com:22/Verdenroz/scry.git".to_string()
}

#[bench(
    group = "sync",
    setup_sized = bytes_n,
    sizes(4096, 65536, 1048576),
    complexity = "n",
    alloc = 0,
    covers = "scry_core::hashing::hash_bytes"
)]
fn bench_hash_bytes(input: &[u8]) {
    keep(scry_core::hashing::hash_bytes(keep(input)));
}

#[bench(
    group = "repo",
    setup = remote_url,
    alloc = 5,
    covers = "scry_core::repo::normalize_remote_url"
)]
fn bench_normalize_remote_url(url: &str) {
    keep(scry_core::repo::normalize_remote_url(keep(url)));
}

/// Synthetic Rust source: n small functions plus a use block.
#[fixture]
fn rust_source_n(n: usize) -> String {
    let mut source = String::from("use std::io;\n\n");
    for i in 0..n {
        source.push_str(&format!(
            "fn handler_{i}(input: u32) -> u32 {{\n    input + {i}\n}}\n\n"
        ));
    }
    source
}

#[fixture]
fn symbol_table() -> Vec<String> {
    (0..5000)
        .map(|i| format!("Service{i} > handle_request_{i}"))
        .collect()
}

#[bench(
    group = "chunker",
    setup_sized = rust_source_n,
    sizes(64, 256, 1024),
    complexity = "n",
    covers = "scry_core::chunker::chunk_file"
)]
fn bench_chunk_rust(source: &str) {
    keep(scry_core::chunker::chunk_file("bench.rs", keep(source)));
}

#[bench(
    group = "search",
    setup = symbol_table,
    covers = "scry_core::search::expand_symbols"
)]
fn bench_expand_symbols(symbols: &[String]) {
    keep(scry_core::search::expand_symbols(
        keep(symbols),
        "request handler service auth",
    ));
}

/// In-memory store with n chunks of pseudo-random unit vectors and
/// word-bearing content, so dense, lexical, and fused search all have work.
struct SearchFixture {
    store: scry_core::store::Store,
    repo_id: i64,
    query: Vec<f32>,
}

const BENCH_DIM: usize = 1024;

fn unit_vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let raw: Vec<f32> = (0..BENCH_DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            ((x >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        })
        .collect();
    let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
    raw.into_iter().map(|v| v / norm).collect()
}

#[fixture]
fn search_store_n(n: usize) -> SearchFixture {
    use scry_core::store::{NewChunk, Store};
    let mut store = Store::open_in_memory("bench", BENCH_DIM).unwrap();
    let repo_id = store.upsert_repo("bench/repo").unwrap();
    let words = [
        "request", "service", "auth", "handler", "cache", "token", "parse", "route",
    ];
    for file in 0..n.div_ceil(64) {
        let file_id = store
            .upsert_file(repo_id, &format!("src/mod_{file}.rs"), "0", 1, 0)
            .unwrap();
        let chunks: Vec<NewChunk> = (0..64)
            .map(|i| file * 64 + i)
            .take_while(|&i| i < n)
            .map(|i| NewChunk {
                start_line: 1,
                end_line: 4,
                symbol: Some(format!("Service{} > handle_{}", i % 97, words[i % 8])),
                content: format!(
                    "fn handle_{}(input: u32) -> u32 {{ {} {} }}",
                    words[i % 8],
                    words[(i / 8) % 8],
                    words[(i / 64) % 8]
                ),
                content_hash: format!("{i:016x}"),
                embedding: unit_vector(i as u64 + 1),
            })
            .collect();
        store
            .replace_file_chunks(file_id, repo_id, &chunks)
            .unwrap();
    }
    SearchFixture {
        store,
        repo_id,
        query: unit_vector(7),
    }
}

#[bench(
    group = "store",
    setup_sized = search_store_n,
    sizes(2048, 16384),
    covers = "scry_core::store::Store::dense_search"
)]
fn bench_dense_search(f: &SearchFixture) {
    keep(
        f.store
            .dense_search(Some(f.repo_id), keep(&f.query), 50)
            .unwrap(),
    );
}

#[bench(
    group = "store",
    setup_sized = search_store_n,
    sizes(2048, 16384),
    covers = "scry_core::store::Store::lexical_search"
)]
fn bench_lexical_search(f: &SearchFixture) {
    let fts = scry_core::search::fts_query("request handler service auth", &[]);
    keep(
        f.store
            .lexical_search(Some(f.repo_id), keep(&fts), 50, None)
            .unwrap(),
    );
}

#[bench(
    group = "search",
    setup_sized = search_store_n,
    sizes(2048, 16384),
    covers = "scry_core::search::search_with_vector"
)]
fn bench_search_with_vector(f: &SearchFixture) {
    let options = scry_core::search::SearchOptions {
        limit: 10,
        path_prefix: None,
    };
    keep(
        scry_core::search::search_with_vector(
            &f.store,
            Some(f.repo_id),
            keep("request handler service auth"),
            keep(&f.query),
            &options,
        )
        .unwrap(),
    );
}
